use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
#[cfg(windows)]
use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;

use crate::cluster::{LeaderCluster, RemoteRequest, spawn_worker_client};
use crate::config;
use crate::protocol::{
    AuditContext, AuditRecord, AuditSource, AuditStatus, AuditTransport, Envelope,
    NodeMetricsSample, NotificationRule, Request, Response, ScalingMetric, ScalingPolicy,
    TaskMetricsAggregate, TaskMetricsSample, TaskMetricsSnapshot, TaskProcessSnapshot, TaskStatus,
};
use crate::runtime::{SessionRuntime, Sessions};
use crate::service;
use crate::state::{NodeRole, NodeSettings, StateStore};
use crate::web;

pub const TASK_METRICS_SAMPLE_INTERVAL_MS: u64 = 1_000;
pub const MAX_TASK_METRIC_SAMPLES: usize = 600;

#[derive(Clone)]
pub struct DaemonState {
    pub store: Arc<StateStore>,
    pub settings: Arc<Mutex<NodeSettings>>,
    pub cluster: LeaderCluster,
    pub sessions: Arc<Mutex<Sessions>>,
    pub unavailable_sessions: Arc<Mutex<BTreeMap<String, UnavailableSession>>>,
    pub task_metrics: Arc<Mutex<TaskMetricsStore>>,
    pub node_metrics: Arc<NodeMetricsStore>,
    run_triggers: Arc<Mutex<HashMap<ScheduleKey, String>>>,
    pub config_mutations: Arc<Mutex<()>>,
    pub shutdown: Arc<AtomicBool>,
    #[cfg(test)]
    pub put_config_post_check_delay: Arc<Mutex<Option<Duration>>>,
    #[cfg(test)]
    pub put_config_before_finalize_content: Arc<Mutex<Option<String>>>,
    #[cfg(test)]
    pub put_config_runtime_failure_after: Arc<Mutex<Option<usize>>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UnavailableSession {
    pub session: String,
    pub project: PathBuf,
    pub error: String,
}

impl DaemonState {
    #[cfg(test)]
    pub fn new() -> Self {
        let store = Arc::new(StateStore::open_in_memory().expect("in-memory state store"));
        let settings = store.node_settings().expect("default node settings");
        let cluster = LeaderCluster::new(store.clone(), settings.enrollment_token.clone())
            .expect("test leader cluster");
        let node_metrics = cluster.node_metrics();
        Self {
            store,
            settings: Arc::new(Mutex::new(settings)),
            cluster,
            sessions: Arc::new(Mutex::new(Sessions::new())),
            unavailable_sessions: Arc::new(Mutex::new(BTreeMap::new())),
            task_metrics: Arc::new(Mutex::new(TaskMetricsStore::default())),
            node_metrics,
            run_triggers: Arc::new(Mutex::new(HashMap::new())),
            config_mutations: Arc::new(Mutex::new(())),
            shutdown: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            put_config_post_check_delay: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            put_config_before_finalize_content: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            put_config_runtime_failure_after: Arc::new(Mutex::new(None)),
        }
    }

    pub fn load(paths: &GlobalPaths) -> Result<Self> {
        let store = Arc::new(StateStore::open(&paths.root)?);
        let settings = store.node_settings()?;
        let cluster = LeaderCluster::new(store.clone(), settings.enrollment_token.clone())?;
        let node_metrics = cluster.node_metrics();
        let mut sessions = Sessions::new();
        let mut unavailable_sessions = BTreeMap::new();
        if settings.execution_enabled() {
            for registration in store.registrations()? {
                match config::discover(&registration.project, Some(&registration.session)) {
                    Ok(definition) => {
                        let mut runtime = SessionRuntime::new(definition);
                        runtime.set_alias(registration.alias);
                        runtime.auto_start();
                        sessions.insert(registration.session, runtime);
                    }
                    Err(error) => {
                        unavailable_sessions.insert(
                            registration.session.clone(),
                            UnavailableSession {
                                session: registration.session,
                                project: registration.project,
                                error: format!("{error:#}"),
                            },
                        );
                    }
                }
            }
        }
        let auth = store.apply_auth_environment()?;
        if auth.enabled {
            let _ = store.record_event(
                "auth",
                "access-key authentication enabled",
                serde_json::json!({"enabled":true}),
            );
        }
        Ok(Self {
            store,
            settings: Arc::new(Mutex::new(settings)),
            cluster,
            sessions: Arc::new(Mutex::new(sessions)),
            unavailable_sessions: Arc::new(Mutex::new(unavailable_sessions)),
            task_metrics: Arc::new(Mutex::new(TaskMetricsStore::default())),
            node_metrics,
            run_triggers: Arc::new(Mutex::new(HashMap::new())),
            config_mutations: Arc::new(Mutex::new(())),
            shutdown: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            put_config_post_check_delay: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            put_config_before_finalize_content: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            put_config_runtime_failure_after: Arc::new(Mutex::new(None)),
        })
    }

    pub fn execution_enabled(&self) -> bool {
        self.settings
            .lock()
            .expect("node settings lock")
            .execution_enabled()
    }

    pub fn public_settings(&self) -> crate::state::PublicNodeSettings {
        self.settings.lock().expect("node settings lock").public()
    }

    pub fn local_inventory(&self) -> Vec<crate::protocol::SessionSnapshot> {
        if !self.execution_enabled() {
            return Vec::new();
        }
        let aliases = self
            .store
            .registrations()
            .map(|registrations| {
                registrations
                    .into_iter()
                    .map(|registration| (registration.session, registration.alias))
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        self.sessions
            .lock()
            .expect("sessions lock")
            .values_mut()
            .filter_map(|runtime| {
                runtime.set_alias(aliases.get(runtime.name()).cloned().flatten());
                runtime.snapshot(0).ok()
            })
            .collect()
    }

    pub fn node_summaries(&self) -> Vec<crate::protocol::NodeSummary> {
        let settings = self.settings.lock().expect("node settings lock").clone();
        let mut nodes = Vec::new();
        if settings.execution_enabled() {
            let mut sessions = self
                .sessions
                .lock()
                .expect("sessions lock")
                .keys()
                .cloned()
                .chain(
                    self.unavailable_sessions
                        .lock()
                        .expect("unavailable sessions lock")
                        .keys()
                        .cloned(),
                )
                .collect::<Vec<_>>();
            sessions.sort();
            sessions.dedup();
            nodes.push(crate::protocol::NodeSummary {
                id: "self".to_string(),
                name: settings.name.clone(),
                role: settings.role.as_label().to_string(),
                mode: if settings.role == NodeRole::Leader {
                    settings.leader_mode.as_label().to_string()
                } else {
                    "local_executor".to_string()
                },
                online: true,
                is_self: true,
                last_seen_ms: Some(current_timestamp_ms()),
                sessions,
            });
        }
        if settings.role == NodeRole::Leader {
            nodes.extend(self.cluster.remote_nodes());
        }
        nodes
    }

    #[allow(dead_code)]
    pub async fn dispatch_node(&self, node: &str, request: RemoteRequest) -> Response {
        self.dispatch_node_with_audit(node, request, AuditContext::internal())
            .await
    }

    pub async fn dispatch_node_with_audit(
        &self,
        node: &str,
        request: RemoteRequest,
        audit: AuditContext,
    ) -> Response {
        let settings = self.settings.lock().expect("node settings lock").clone();
        let local_request = request.clone().into_local();
        let audit = audit.with_request_defaults(&local_request);
        if node == "self" {
            if !settings.execution_enabled() {
                let response = Response::error("pure master does not have a self executor");
                record_request_audit(
                    self,
                    &local_request,
                    audit,
                    None,
                    current_timestamp_ms(),
                    0,
                    AuditStatus::Error,
                    Some(settings.node_id),
                    &response,
                    serde_json::json!({"node": "self"}),
                );
                return response;
            }
            return dispatch_async_with_audit(self.clone(), local_request, Some(audit)).await;
        }
        if settings.role != NodeRole::Leader {
            let response =
                Response::error("worker nodes can only control their local self executor");
            record_request_audit(
                self,
                &local_request,
                audit,
                None,
                current_timestamp_ms(),
                0,
                AuditStatus::Error,
                Some(settings.node_id),
                &response,
                serde_json::json!({"node": node}),
            );
            return response;
        }

        let origin_audit_id = uuid::Uuid::new_v4().to_string();
        let mut worker_audit = audit.clone();
        if worker_audit.origin_node_id.is_none() {
            worker_audit.origin_node_id = Some(settings.node_id.clone());
        }
        worker_audit.origin_audit_id = Some(origin_audit_id.clone());
        worker_audit.transport = AuditTransport::Agent;

        let started_at_ms = current_timestamp_ms();
        let started = Instant::now();
        let (response, status) = self
            .cluster
            .request_with_audit(node, request, Some(worker_audit))
            .await;
        record_request_audit(
            self,
            &local_request,
            audit,
            Some(origin_audit_id),
            started_at_ms,
            started.elapsed().as_millis() as u64,
            status,
            Some(node.to_string()),
            &response,
            serde_json::json!({"node": node, "remote_transport": "agent"}),
        );
        response
    }

    pub fn service_rows(&self, node: Option<&str>) -> Vec<serde_json::Value> {
        let inventories = match node {
            Some("self") => vec![("self".to_string(), self.local_inventory())],
            Some(node) => self
                .cluster
                .cached_inventory(node)
                .map(|inventory| vec![(node.to_string(), inventory)])
                .unwrap_or_default(),
            None => {
                let mut inventories = Vec::new();
                let settings = self.settings.lock().expect("node settings lock");
                if settings.execution_enabled() {
                    inventories.push(("self".to_string(), self.local_inventory()));
                }
                if settings.role == NodeRole::Leader {
                    for node in self.cluster.remote_nodes() {
                        if let Some(inventory) = self.cluster.cached_inventory(&node.id) {
                            inventories.push((node.id, inventory));
                        }
                    }
                }
                inventories
            }
        };
        inventories
            .into_iter()
            .flat_map(|(node_id, sessions)| {
                sessions.into_iter().flat_map(move |session| {
                    let node_id = node_id.clone();
                    session
                        .tasks
                        .into_iter()
                        .filter_map(move |(task, snapshot)| {
                            let service = snapshot.service;
                            if service.classification
                                == crate::protocol::ServiceClassification::Unknown
                                && service.endpoints.is_empty()
                            {
                                return None;
                            }
                            Some(serde_json::json!({
                                "node": node_id,
                                "session": session.name,
                                "task": task,
                                "service": service,
                            }))
                        })
                })
            })
            .collect()
    }

    #[cfg(test)]
    pub fn set_put_config_post_check_delay(&self, delay: Duration) {
        *self
            .put_config_post_check_delay
            .lock()
            .expect("config write delay lock") = Some(delay);
    }
    #[cfg(test)]
    fn put_config_post_check_delay(&self) -> Option<Duration> {
        *self
            .put_config_post_check_delay
            .lock()
            .expect("config write delay lock")
    }
    #[cfg(test)]
    pub fn set_put_config_before_finalize_content(&self, content: impl Into<String>) {
        *self
            .put_config_before_finalize_content
            .lock()
            .expect("config write finalize hook lock") = Some(content.into());
    }
    #[cfg(test)]
    fn take_put_config_before_finalize_content(&self) -> Option<String> {
        self.put_config_before_finalize_content
            .lock()
            .expect("config write finalize hook lock")
            .take()
    }
    #[cfg(test)]
    pub fn set_put_config_runtime_failure_after(&self, count: usize) {
        *self
            .put_config_runtime_failure_after
            .lock()
            .expect("runtime failure hook lock") = Some(count);
    }
    #[cfg(test)]
    fn put_config_runtime_failure_after(&self) -> Option<usize> {
        *self
            .put_config_runtime_failure_after
            .lock()
            .expect("runtime failure hook lock")
    }
    #[cfg(test)]
    pub fn clear_put_config_runtime_failure(&self) {
        *self
            .put_config_runtime_failure_after
            .lock()
            .expect("runtime failure hook lock") = None;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TaskMetricsKey {
    session: String,
    task: String,
}

#[derive(Debug, Clone)]
struct TaskMetricsTarget {
    session: String,
    task: String,
    root_pid: Option<u32>,
    start_generation: u64,
    history_generation: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct ObservedProcess {
    pid: u32,
    ppid: Option<u32>,
    name: String,
    cpu_percent: f32,
    memory_bytes: u64,
    status: String,
    run_time_seconds: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct AggregatedProcessTree {
    aggregate: TaskMetricsAggregate,
    processes: Vec<TaskProcessSnapshot>,
}

#[derive(Debug, Clone, Default)]
struct TaskMetricsEntry {
    running: bool,
    current: TaskMetricsAggregate,
    processes: Vec<TaskProcessSnapshot>,
    samples: VecDeque<TaskMetricsSample>,
    restart_markers_ms: VecDeque<u64>,
}

impl TaskMetricsEntry {
    fn apply_observation(&mut self, timestamp_ms: u64, observation: Option<AggregatedProcessTree>) {
        match observation {
            Some(observation) => {
                self.running = true;
                self.current = observation.aggregate.clone();
                self.processes = observation.processes;
                self.samples.push_back(TaskMetricsSample {
                    timestamp_ms,
                    cpu_percent: self.current.cpu_percent,
                    memory_bytes: self.current.memory_bytes,
                    process_count: self.current.process_count,
                });
                while self.samples.len() > MAX_TASK_METRIC_SAMPLES {
                    self.samples.pop_front();
                }
            }
            None => {
                self.running = false;
                self.current = TaskMetricsAggregate::zero();
                self.processes.clear();
            }
        }
    }

    fn snapshot(&self, window_seconds: usize) -> TaskMetricsSnapshot {
        let sample_count = window_seconds.min(MAX_TASK_METRIC_SAMPLES);
        let skip = self.samples.len().saturating_sub(sample_count);
        let samples = self.samples.iter().skip(skip).cloned().collect::<Vec<_>>();
        let first_timestamp = samples.first().map(|sample| sample.timestamp_ms);
        TaskMetricsSnapshot {
            sample_interval_ms: TASK_METRICS_SAMPLE_INTERVAL_MS,
            window_seconds: sample_count as u64,
            cpu_percent_unit:
                "100.0 = one fully utilized logical CPU; sums can exceed 100 across cores/processes"
                    .to_string(),
            running: self.running,
            current: self.current.clone(),
            samples,
            processes: self.processes.clone(),
            restart_markers_ms: self
                .restart_markers_ms
                .iter()
                .copied()
                .filter(|timestamp| first_timestamp.is_none_or(|first| *timestamp >= first))
                .collect(),
        }
    }

    fn mark_restart(&mut self, timestamp_ms: u64) {
        self.restart_markers_ms.push_back(timestamp_ms);
        while self.restart_markers_ms.len() > MAX_TASK_METRIC_SAMPLES {
            self.restart_markers_ms.pop_front();
        }
    }
}

#[derive(Debug, Default)]
pub struct TaskMetricsStore {
    entries: HashMap<TaskMetricsKey, TaskMetricsEntry>,
}

impl TaskMetricsStore {
    fn record(
        &mut self,
        session: impl Into<String>,
        task: impl Into<String>,
        timestamp_ms: u64,
        observation: Option<AggregatedProcessTree>,
    ) {
        let key = TaskMetricsKey {
            session: session.into(),
            task: task.into(),
        };
        self.entries
            .entry(key)
            .or_default()
            .apply_observation(timestamp_ms, observation);
    }

    fn snapshot(&self, session: &str, task: &str, window_seconds: usize) -> TaskMetricsSnapshot {
        self.entries
            .get(&TaskMetricsKey {
                session: session.to_string(),
                task: task.to_string(),
            })
            .map(|entry| entry.snapshot(window_seconds))
            .unwrap_or_else(|| TaskMetricsEntry::default().snapshot(window_seconds))
    }

    fn remove_session(&mut self, session: &str) {
        self.entries.retain(|key, _| key.session != session);
    }

    fn clear_task(&mut self, session: &str, task: &str) {
        self.entries.remove(&TaskMetricsKey {
            session: session.to_string(),
            task: task.to_string(),
        });
    }

    fn mark_restart(&mut self, session: &str, task: &str, timestamp_ms: u64) {
        self.entries
            .entry(TaskMetricsKey {
                session: session.to_string(),
                task: task.to_string(),
            })
            .or_default()
            .mark_restart(timestamp_ms);
    }

    fn retain_current_tasks(&mut self, current: &HashSet<TaskMetricsKey>) {
        self.entries.retain(|key, _| current.contains(key));
    }
}

pub const MAX_NODE_METRIC_SAMPLES: usize = 300;
pub const SCALING_EVALUATION_INTERVAL_MS: u64 = 5_000;
const SCALING_STREAK_THRESHOLD: u32 = 3;
const SCALING_REMOTE_FETCH_INTERVAL_MS: u64 = 10_000;

/// Ring buffers of node-level CPU/memory samples keyed by node id. The local
/// sampler pushes self samples; worker samples arrive embedded in inventory
/// pushes on the leader.
#[derive(Debug, Default)]
pub struct NodeMetricsStore {
    samples: Mutex<HashMap<String, std::collections::VecDeque<NodeMetricsSample>>>,
}

impl NodeMetricsStore {
    pub fn push(&self, node_id: &str, sample: NodeMetricsSample) {
        let mut samples = self.samples.lock().expect("node metrics lock");
        let window = samples.entry(node_id.to_string()).or_default();
        if window
            .back()
            .is_some_and(|last| last.timestamp_ms >= sample.timestamp_ms)
        {
            return;
        }
        window.push_back(sample);
        while window.len() > MAX_NODE_METRIC_SAMPLES {
            window.pop_front();
        }
    }

    pub fn window(&self, node_id: &str, limit: usize) -> Vec<NodeMetricsSample> {
        let samples = self.samples.lock().expect("node metrics lock");
        match samples.get(node_id) {
            Some(window) => window.iter().rev().take(limit).rev().cloned().collect(),
            None => Vec::new(),
        }
    }

    pub fn latest(&self, node_id: &str) -> Option<NodeMetricsSample> {
        let samples = self.samples.lock().expect("node metrics lock");
        samples
            .get(node_id)
            .and_then(|window| window.back().cloned())
    }
}

fn count_running_tasks(state: &DaemonState) -> u32 {
    let mut sessions = state.sessions.lock().expect("sessions lock");
    let mut running = 0;
    for runtime in sessions.values_mut() {
        if let Ok(snapshot) = runtime.snapshot(0) {
            running += snapshot
                .tasks
                .values()
                .filter(|task| matches!(task.status, TaskStatus::Running | TaskStatus::Paused))
                .count();
        }
    }
    running as u32
}

fn sample_node_metrics(state: &DaemonState, system: &mut System) {
    system.refresh_cpu_all();
    system.refresh_memory();
    let sample = NodeMetricsSample {
        timestamp_ms: current_timestamp_ms(),
        cpu_percent: system.global_cpu_usage(),
        memory_bytes: system.used_memory(),
        memory_total_bytes: system.total_memory(),
        running_tasks: count_running_tasks(state),
    };
    let node_id = state.public_settings().node_id;
    state.node_metrics.push(&node_id, sample);
}

impl DaemonState {
    /// Count running/paused tasks per scope to evaluate quotas.
    fn running_task_counts(&self) -> (usize, HashMap<String, usize>) {
        let mut sessions = self.sessions.lock().expect("sessions lock");
        let mut per_session = HashMap::new();
        let mut node_total = 0;
        for (name, runtime) in sessions.iter_mut() {
            if let Ok(snapshot) = runtime.snapshot(0) {
                for task in snapshot.tasks.values() {
                    if matches!(task.status, TaskStatus::Running | TaskStatus::Paused) {
                        node_total += 1;
                        *per_session.entry(name.clone()).or_insert(0) += 1;
                    }
                }
            }
        }
        (node_total, per_session)
    }

    /// Enforce workspace/node running-task quotas before local starts.
    pub fn check_quotas(&self, session: &str) -> std::result::Result<(), String> {
        let quotas = match self.store.quotas() {
            Ok(quotas) => quotas,
            Err(_) => return Ok(()),
        };
        let quotas: Vec<_> = quotas
            .into_iter()
            .filter(|quota| quota.node_id == self.public_settings().node_id)
            .collect();
        if quotas.is_empty() {
            return Ok(());
        }
        let (node_total, per_session) = self.running_task_counts();
        for quota in quotas {
            match &quota.session {
                Some(scope) if scope == session => {
                    let running = per_session.get(session).copied().unwrap_or(0);
                    if running >= quota.max_running_tasks as usize {
                        return Err(format!(
                            "workspace '{session}' quota reached ({running}/{} running tasks)",
                            quota.max_running_tasks
                        ));
                    }
                }
                None => {
                    if node_total >= quota.max_running_tasks as usize {
                        return Err(format!(
                            "node quota reached ({node_total}/{} running tasks)",
                            quota.max_running_tasks
                        ));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn task_status_on(&self, node: &str, session: &str, task: &str) -> Option<TaskStatus> {
        let settings = self.public_settings();
        if node == "self" || node == settings.node_id {
            let mut sessions = self.sessions.lock().expect("sessions lock");
            let snapshot = sessions.get_mut(session)?.snapshot(0).ok()?;
            return snapshot.tasks.get(task).map(|task| task.status.clone());
        }
        if settings.role != NodeRole::Leader {
            return None;
        }
        let inventory = self.cluster.cached_inventory(node)?;
        let session_snapshot = inventory
            .iter()
            .find(|session_view| session_view.name == session)?;
        session_snapshot
            .tasks
            .get(task)
            .map(|task| task.status.clone())
    }

    /// Enforce cross-workspace task dependencies before local starts.
    pub fn check_dependencies(&self, session: &str, task: &str) -> std::result::Result<(), String> {
        let node_id = self.public_settings().node_id;
        let dependencies = match self.store.dependencies_for_task(&node_id, session, task) {
            Ok(dependencies) => dependencies,
            Err(_) => return Ok(()),
        };
        if dependencies.is_empty() {
            return Ok(());
        }
        for dependency in dependencies {
            let reason_target = format!(
                "{}:{}:{}",
                dependency.depends_node_id, dependency.depends_session, dependency.depends_task
            );
            match self.task_status_on(
                &dependency.depends_node_id,
                &dependency.depends_session,
                &dependency.depends_task,
            ) {
                Some(status) if matches!(status, TaskStatus::Running) => {}
                Some(status) => {
                    return Err(format!(
                        "dependency {reason_target} is not running (status: {})",
                        status_label(status)
                    ));
                }
                None => {
                    return Err(format!(
                        "dependency {reason_target} is not visible from this node"
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn check_start_gates(&self, session: &str, task: &str) -> std::result::Result<(), String> {
        self.check_quotas(session)?;
        self.check_dependencies(session, task)
    }
}

fn emit_transition_notifications(state: &DaemonState, transitions: &[RunTransition]) {
    let rules: Vec<NotificationRule> = match state.store.notification_rules() {
        Ok(rules) => rules.into_iter().filter(|rule| rule.enabled).collect(),
        Err(_) => return,
    };
    if rules.is_empty() {
        return;
    }
    let node_id = state.public_settings().node_id;
    for transition in transitions {
        let (event_type, severity, session, task, title, message, details) = match transition {
            RunTransition::Started(session, snapshot, trigger) => (
                "task_started",
                "info",
                session.clone(),
                snapshot.label.clone(),
                format!("task started: {}", snapshot.label),
                format!(
                    "workspace '{session}' task '{}' started (trigger: {trigger})",
                    snapshot.label
                ),
                serde_json::json!({"trigger": trigger}),
            ),
            RunTransition::Finished {
                session,
                task,
                status,
                exit_code,
                error_message,
                ..
            } => {
                let (event_type, severity) = match status.as_str() {
                    "failed" => ("task_failed", "critical"),
                    "stopped" => ("task_stopped", "warning"),
                    _ => ("task_exited", "info"),
                };
                (
                    event_type,
                    severity,
                    session.clone(),
                    task.clone(),
                    format!("task {status}: {task}"),
                    format!(
                        "workspace '{session}' task '{task}' finished with status '{status}'{}",
                        error_message
                            .as_deref()
                            .map(|error| format!(": {error}"))
                            .unwrap_or_default()
                    ),
                    serde_json::json!({"status": status, "exit_code": exit_code}),
                )
            }
        };
        for rule in &rules {
            if !rule
                .event_types
                .iter()
                .any(|candidate| candidate == event_type)
            {
                continue;
            }
            if rule
                .scope_session
                .as_deref()
                .is_some_and(|scope| scope != session)
            {
                continue;
            }
            if rule
                .scope_task
                .as_deref()
                .is_some_and(|scope| scope != task)
            {
                continue;
            }
            match state.store.insert_notification(
                &node_id,
                Some(&rule.id),
                Some(&rule.name),
                event_type,
                severity,
                Some(&session),
                Some(&task),
                &title,
                &message,
                &details,
            ) {
                Ok(_) => {}
                Err(error) => eprintln!("failed to record notification: {error:#}"),
            }
            if let Some(webhook_url) = &rule.webhook_url {
                spawn_webhook_delivery(
                    state,
                    webhook_url.clone(),
                    event_type,
                    severity,
                    &session,
                    &task,
                    &title,
                    &message,
                    &details,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_webhook_delivery(
    state: &DaemonState,
    url: String,
    event_type: &str,
    severity: &str,
    session: &str,
    task: &str,
    title: &str,
    message: &str,
    details: &serde_json::Value,
) {
    let payload = serde_json::json!({
        "kind": "taskdeck.notification",
        "event_type": event_type,
        "severity": severity,
        "node_id": state.public_settings().node_id,
        "session": session,
        "task": task,
        "title": title,
        "message": message,
        "details": details,
        "timestamp_ms": current_timestamp_ms(),
    });
    let store = state.store.clone();
    let event_type = event_type.to_string();
    thread::spawn(move || {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(5))
            .build();
        if let Err(error) = agent.post(&url).send_json(payload) {
            let _ = store.record_event(
                "notification",
                &format!("webhook delivery failed: {error}"),
                serde_json::json!({"url": url, "event_type": event_type}),
            );
        }
    });
}

pub fn spawn_scaling_evaluator(state: DaemonState) -> Option<thread::JoinHandle<()>> {
    // Capture the tokio runtime handle before leaving the async context so the
    // evaluator thread can block on node dispatches.
    let runtime = tokio::runtime::Handle::try_current().ok();
    Some(thread::spawn(move || {
        let mut above_streaks: HashMap<String, u32> = HashMap::new();
        let mut below_streaks: HashMap<String, u32> = HashMap::new();
        let mut remote_cache: HashMap<String, (u64, Option<TaskMetricsSnapshot>)> = HashMap::new();
        while !state.shutdown.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(SCALING_EVALUATION_INTERVAL_MS));
            if state.shutdown.load(Ordering::SeqCst) {
                break;
            }
            evaluate_scaling_policies(
                &state,
                runtime.as_ref(),
                &mut above_streaks,
                &mut below_streaks,
                &mut remote_cache,
            );
        }
    }))
}

fn evaluate_scaling_policies(
    state: &DaemonState,
    runtime: Option<&tokio::runtime::Handle>,
    above_streaks: &mut HashMap<String, u32>,
    below_streaks: &mut HashMap<String, u32>,
    remote_cache: &mut HashMap<String, (u64, Option<TaskMetricsSnapshot>)>,
) {
    let policies = match state.store.scaling_policies() {
        Ok(policies) => policies,
        Err(_) => return,
    };
    let now = current_timestamp_ms();
    for policy in policies.iter().filter(|policy| policy.enabled) {
        let value = match policy.watch_node_id.as_str() {
            "self" => {
                let snapshot = state
                    .task_metrics
                    .lock()
                    .expect("task metrics lock")
                    .snapshot(&policy.watch_session, &policy.watch_task, 5);
                if !snapshot.running {
                    0.0
                } else {
                    match policy.metric {
                        ScalingMetric::CpuPercent => snapshot.current.cpu_percent as f64,
                        ScalingMetric::MemoryBytes => snapshot.current.memory_bytes as f64,
                    }
                }
            }
            node => {
                if state.public_settings().role != NodeRole::Leader {
                    continue;
                }
                let cache_key = format!("{node}:{}:{}", policy.watch_session, policy.watch_task);
                let cached = remote_cache.get(&cache_key);
                let fresh = cached.is_some_and(|(fetched, _)| {
                    now.saturating_sub(*fetched) < SCALING_REMOTE_FETCH_INTERVAL_MS
                });
                if !fresh {
                    let snapshot = runtime.and_then(|runtime| {
                        let response = runtime.block_on(state.dispatch_node(
                            node,
                            RemoteRequest::TaskMetrics {
                                session: policy.watch_session.clone(),
                                task: policy.watch_task.clone(),
                                window_seconds: 5,
                            },
                        ));
                        serde_json::from_value::<TaskMetricsSnapshot>(
                            response.data.unwrap_or(serde_json::Value::Null),
                        )
                        .ok()
                    });
                    remote_cache.insert(cache_key.clone(), (now, snapshot));
                }
                match remote_cache
                    .get(&cache_key)
                    .and_then(|(_, snapshot)| snapshot.as_ref())
                {
                    Some(snapshot) if snapshot.running => match policy.metric {
                        ScalingMetric::CpuPercent => snapshot.current.cpu_percent as f64,
                        ScalingMetric::MemoryBytes => snapshot.current.memory_bytes as f64,
                    },
                    _ => 0.0,
                }
            }
        };

        let cooldown_elapsed = policy
            .last_action_ms
            .map(|last| now.saturating_sub(last) >= policy.cooldown_seconds * 1000)
            .unwrap_or(true);
        if value > policy.scale_out_threshold {
            *above_streaks.entry(policy.id.clone()).or_insert(0) += 1;
            below_streaks.remove(&policy.id);
        } else {
            above_streaks.remove(&policy.id);
        }
        if value < policy.scale_in_threshold {
            *below_streaks.entry(policy.id.clone()).or_insert(0) += 1;
            above_streaks.remove(&policy.id);
        } else {
            below_streaks.remove(&policy.id);
        }

        if !cooldown_elapsed {
            continue;
        }
        let above = above_streaks.get(&policy.id).copied().unwrap_or(0);
        let below = below_streaks.get(&policy.id).copied().unwrap_or(0);
        if above >= SCALING_STREAK_THRESHOLD {
            above_streaks.remove(&policy.id);
            below_streaks.remove(&policy.id);
            scale_policy_task(
                state,
                runtime,
                policy,
                "scale_out",
                crate::protocol::Action::Start,
                now,
            );
        } else if below >= SCALING_STREAK_THRESHOLD {
            above_streaks.remove(&policy.id);
            below_streaks.remove(&policy.id);
            scale_policy_task(
                state,
                runtime,
                policy,
                "scale_in",
                crate::protocol::Action::Stop,
                now,
            );
        }
    }
}

fn scale_policy_task(
    state: &DaemonState,
    runtime: Option<&tokio::runtime::Handle>,
    policy: &ScalingPolicy,
    action_label: &str,
    action: crate::protocol::Action,
    now: u64,
) {
    let Some(runtime) = runtime else {
        return;
    };
    let running = task_running_on_node(
        state,
        &policy.scale_out_node_id,
        &policy.scale_out_session,
        &policy.scale_out_task,
    );
    let should_fire = match (action_label, running) {
        ("scale_out", Some(false)) => true,
        ("scale_in", Some(true)) => true,
        _ => false,
    };
    if !should_fire {
        return;
    }
    let request = RemoteRequest::Action {
        session: policy.scale_out_session.clone(),
        task: Some(policy.scale_out_task.clone()),
        action,
    };
    let response = runtime.block_on(state.dispatch_node_with_audit(
        &policy.scale_out_node_id,
        request,
        AuditContext::new(AuditSource::Internal, AuditTransport::Internal),
    ));
    let node_id = state.public_settings().node_id;
    let success = response.ok;
    let _ = state
        .store
        .record_scaling_action(&policy.id, action_label, now);
    let _ = state.store.record_event(
        "autoscale",
        &format!(
            "policy '{}' {} action {}: {}",
            policy.name, action_label, policy.scale_out_task, response.message
        ),
        serde_json::json!({
            "policy": policy.name,
            "action": action_label,
            "success": success,
        }),
    );
    let title = format!(
        "auto-scaling {}: {}",
        action_label.trim_start_matches("scale_"),
        policy.scale_out_task
    );
    let message = format!(
        "policy '{}' {} task '{}:{}' (metric {}: {:.1}): {}",
        policy.name,
        action_label,
        policy.scale_out_session,
        policy.scale_out_task,
        policy.metric.as_str(),
        match policy.metric {
            ScalingMetric::CpuPercent => policy.scale_out_threshold,
            ScalingMetric::MemoryBytes => policy.scale_out_threshold,
        },
        response.message
    );
    let _ = state.store.insert_notification(
        &node_id,
        None,
        None,
        action_label,
        if success { "info" } else { "warning" },
        Some(&policy.scale_out_session),
        Some(&policy.scale_out_task),
        &title,
        &message,
        &serde_json::json!({"policy_id": policy.id, "success": success}),
    );
}

fn task_running_on_node(
    state: &DaemonState,
    node: &str,
    session: &str,
    task: &str,
) -> Option<bool> {
    let settings = state.public_settings();
    if node == "self" || node == settings.node_id {
        let mut sessions = state.sessions.lock().expect("sessions lock");
        let snapshot = sessions.get_mut(session)?.snapshot(0).ok()?;
        return snapshot
            .tasks
            .get(task)
            .map(|task| matches!(task.status, TaskStatus::Running | TaskStatus::Paused));
    }
    if settings.role != NodeRole::Leader {
        return None;
    }
    let inventory = state.cluster.cached_inventory(node)?;
    let session_snapshot = inventory.iter().find(|view| view.name == session)?;
    session_snapshot
        .tasks
        .get(task)
        .map(|task| matches!(task.status, TaskStatus::Running | TaskStatus::Paused))
}

fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ScheduleKey {
    session: String,
    task: String,
}

fn status_label(status: TaskStatus) -> String {
    serde_json::to_value(&status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{status:?}"))
}

enum RunTransition {
    Started(String, Box<crate::protocol::TaskSnapshot>, String),
    Finished {
        session: String,
        task: String,
        generation: u64,
        status: String,
        exit_code: Option<i32>,
        error_message: Option<String>,
    },
}

fn trigger_for(
    state: &DaemonState,
    key: &ScheduleKey,
    snapshot: &crate::protocol::TaskSnapshot,
) -> String {
    state
        .run_triggers
        .lock()
        .expect("run trigger lock")
        .remove(key)
        .unwrap_or_else(|| {
            if snapshot.auto_start {
                "auto_start".to_string()
            } else {
                "manual".to_string()
            }
        })
}

fn finished_run_details(
    session: &str,
    task_snapshot: &crate::protocol::TaskSnapshot,
) -> RunTransition {
    let status = if matches!(task_snapshot.status, TaskStatus::Idle) {
        "stopped".to_string()
    } else {
        status_label(task_snapshot.status.clone())
    };
    RunTransition::Finished {
        session: session.to_string(),
        task: task_snapshot.label.clone(),
        generation: task_snapshot.run_generation,
        status,
        exit_code: task_snapshot.exit_code,
        error_message: if matches!(task_snapshot.status, TaskStatus::Failed)
            || task_snapshot.exit_code.is_none()
        {
            task_snapshot.last_exit.clone()
        } else {
            None
        },
    }
}

fn collect_run_transitions(
    state: &DaemonState,
    tracked: &mut HashMap<ScheduleKey, (u64, TaskStatus)>,
) -> Vec<RunTransition> {
    let mut transitions = Vec::new();
    {
        let mut sessions = state.sessions.lock().expect("sessions lock");
        for (session_name, runtime) in sessions.iter_mut() {
            let Ok(snapshot) = runtime.snapshot(0) else {
                continue;
            };
            for task_snapshot in snapshot.tasks.into_values() {
                let key = ScheduleKey {
                    session: session_name.clone(),
                    task: task_snapshot.label.clone(),
                };
                let previous = tracked
                    .get(&key)
                    .map(|(generation, status)| (*generation, status.clone()));
                let (previous_generation, previous_status) = previous
                    .map(|(generation, status)| (Some(generation), Some(status)))
                    .unwrap_or((None, None));
                let generation_changed = previous_generation != Some(task_snapshot.run_generation);
                let previous_active = previous_status.is_some_and(|status| {
                    matches!(status, TaskStatus::Running | TaskStatus::Paused)
                });
                let current_finished = matches!(
                    task_snapshot.status,
                    TaskStatus::Exited | TaskStatus::Failed | TaskStatus::Idle
                );
                let generation = task_snapshot.run_generation;

                // A restart can replace a still-active generation before the sampler observes
                // its stop. Close the previous row before recording the replacement.
                if generation > 0 && generation_changed && previous_active {
                    transitions.push(RunTransition::Finished {
                        session: session_name.clone(),
                        task: task_snapshot.label.clone(),
                        generation: previous_generation.expect("previous generation"),
                        status: "stopped".to_string(),
                        exit_code: None,
                        error_message: None,
                    });
                }

                // Sampling is asynchronous: a sub-second task can already be Exited/Failed on
                // its first observation. Record both boundaries from that snapshot.
                if generation > 0 && generation_changed {
                    transitions.push(RunTransition::Started(
                        session_name.clone(),
                        Box::new(task_snapshot.clone()),
                        trigger_for(state, &key, &task_snapshot),
                    ));
                    if current_finished {
                        transitions.push(finished_run_details(session_name, &task_snapshot));
                    }
                } else if generation > 0 && previous_active && current_finished {
                    transitions.push(finished_run_details(session_name, &task_snapshot));
                }

                tracked.insert(key.clone(), (generation, task_snapshot.status.clone()));
            }
        }
    }
    transitions
}

fn spawn_task_history_sampler(state: DaemonState) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut tracked = HashMap::<ScheduleKey, (u64, TaskStatus)>::new();
        while !state.shutdown.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(500));
            let transitions = collect_run_transitions(&state, &mut tracked);
            if transitions.is_empty() {
                continue;
            }
            emit_transition_notifications(&state, &transitions);
            let node_id = match state.store.node_settings() {
                Ok(v) => v.node_id,
                Err(_) => continue,
            };
            for transition in transitions {
                match transition {
                    RunTransition::Started(session, snapshot, trigger) => {
                        if let Err(error) = state
                            .store
                            .record_task_run_start(&node_id, &snapshot, &trigger, &session)
                        {
                            eprintln!("failed to persist task run: {error:#}");
                        }
                    }
                    RunTransition::Finished {
                        session,
                        task,
                        generation,
                        status,
                        exit_code,
                        error_message,
                    } => {
                        let _ = state.store.finish_task_run(
                            &node_id,
                            &session,
                            &task,
                            generation,
                            &status,
                            exit_code,
                            error_message.as_deref(),
                        );
                    }
                }
            }
        }
    })
}

fn spawn_task_scheduler(state: DaemonState) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut last_processed_second = current_timestamp_ms() / 1000;
        let _ = state.store.record_event(
            "scheduler",
            "scheduler started; missed executions while offline were not replayed",
            serde_json::json!({}),
        );
        loop {
            thread::sleep(Duration::from_millis(250));
            if state.shutdown.load(Ordering::SeqCst) {
                break;
            }
            let now = current_timestamp_ms();
            let now_second = now / 1000;
            if now_second <= last_processed_second {
                continue;
            }
            let mut due_actions: Vec<ScheduleKey> = Vec::new();
            {
                let mut sessions = state.sessions.lock().expect("sessions lock");
                for (session_name, runtime) in sessions.iter_mut() {
                    let Ok(snapshot) = runtime.snapshot(0) else {
                        continue;
                    };
                    for task_snapshot in snapshot.tasks.into_values() {
                        let Some(expression) = task_snapshot.schedule.as_deref() else {
                            continue;
                        };
                        let fields = expression.split_whitespace().count();
                        let normalized = if fields == 5 {
                            format!("0 {expression}")
                        } else {
                            expression.to_string()
                        };
                        let Ok(schedule) = normalized.parse::<cron::Schedule>() else {
                            continue;
                        };
                        for timestamp_ms in
                            ((last_processed_second + 1) * 1000)..=(now_second * 1000)
                        {
                            if let Some(after_utc) =
                                chrono::DateTime::<chrono::Utc>::from_timestamp_millis(
                                    timestamp_ms as i64,
                                )
                            {
                                let local_after =
                                    <chrono::Local as chrono::TimeZone>::from_utc_datetime(
                                        &chrono::Local,
                                        &after_utc.naive_utc(),
                                    );
                                if schedule.includes(local_after) {
                                    due_actions.push(ScheduleKey {
                                        session: session_name.clone(),
                                        task: task_snapshot.label.clone(),
                                    });
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            last_processed_second = now_second;
            for key in due_actions {
                if let Err(reason) = state.check_start_gates(&key.session, &key.task) {
                    let _ = state.store.record_event(
                        "scheduler",
                        "scheduled start blocked by start gates",
                        serde_json::json!({
                            "session": key.session,
                            "task": key.task,
                            "reason": reason,
                        }),
                    );
                    continue;
                }
                state
                    .run_triggers
                    .lock()
                    .expect("run trigger lock")
                    .insert(key.clone(), "cron".to_string());
                let started_at_ms = current_timestamp_ms();
                let started = Instant::now();
                let mut sessions = state.sessions.lock().expect("sessions lock");
                match sessions
                    .get_mut(&key.session)
                    .map(|runtime| runtime.scheduled_start(&key.task))
                {
                    Some(Ok(true)) => {
                        let node_id = state.public_settings().node_id;
                        if let Some(runtime) = sessions.get_mut(&key.session) {
                            if let Ok(snapshot) = runtime.snapshot(0) {
                                if let Some(run_snapshot) = snapshot
                                    .tasks
                                    .into_values()
                                    .find(|value| value.label == key.task)
                                {
                                    if let Err(error) = state.store.record_task_run_start(
                                        &node_id,
                                        &run_snapshot,
                                        "cron",
                                        &key.session,
                                    ) {
                                        eprintln!("failed to persist scheduled run: {error:#}");
                                    }
                                }
                            }
                        }
                        drop(sessions);
                        let _ = record_audit_value(
                            &state,
                            AuditContext::new(AuditSource::Scheduler, AuditTransport::Internal),
                            None,
                            "scheduler",
                            "start",
                            Some(&key.session),
                            Some(&key.task),
                            AuditStatus::Success,
                            started_at_ms,
                            started.elapsed().as_millis() as u64,
                            serde_json::json!({"type":"scheduler","action":"start","session":key.session,"task":key.task}),
                            serde_json::json!({"ok":true,"message":"scheduled task started"}),
                            serde_json::json!({"trigger":"cron"}),
                            Some(node_id),
                        );
                    }
                    Some(Ok(false)) => {
                        drop(sessions);
                        let _ = state.store.record_event(
                            "scheduler",
                            "scheduled task already running; execution skipped",
                            serde_json::json!({"session":key.session,"task":key.task}),
                        );
                        let _ = record_audit_value(
                            &state,
                            AuditContext::new(AuditSource::Scheduler, AuditTransport::Internal),
                            None,
                            "scheduler",
                            "start",
                            Some(&key.session),
                            Some(&key.task),
                            AuditStatus::Success,
                            started_at_ms,
                            started.elapsed().as_millis() as u64,
                            serde_json::json!({"type":"scheduler","action":"start","session":key.session,"task":key.task}),
                            serde_json::json!({"ok":true,"message":"scheduled task already running; execution skipped"}),
                            serde_json::json!({"trigger":"cron","skipped":true}),
                            None,
                        );
                    }
                    Some(Err(error)) => {
                        let error_message = error.to_string();
                        drop(sessions);
                        let _ = state.store.record_event(
                            "scheduler",
                            "scheduled task failed to start",
                            serde_json::json!({"session":key.session,"task":key.task,"error":error_message}),
                        );
                        let _ = record_audit_value(
                            &state,
                            AuditContext::new(AuditSource::Scheduler, AuditTransport::Internal),
                            None,
                            "scheduler",
                            "start",
                            Some(&key.session),
                            Some(&key.task),
                            AuditStatus::Error,
                            started_at_ms,
                            started.elapsed().as_millis() as u64,
                            serde_json::json!({"type":"scheduler","action":"start","session":key.session,"task":key.task}),
                            serde_json::json!({"ok":false,"message":error_message}),
                            serde_json::json!({"trigger":"cron"}),
                            None,
                        );
                    }
                    None => {}
                }
            }
        }
    })
}
fn collect_metric_targets(state: &DaemonState) -> Vec<TaskMetricsTarget> {
    let sessions = &mut *state.sessions.lock().expect("sessions lock");
    sessions
        .iter_mut()
        .flat_map(|(session, runtime)| {
            runtime.task_root_pids_for_metrics().into_iter().map(
                |(task, root_pid, start_generation, history_generation)| TaskMetricsTarget {
                    session: session.clone(),
                    task,
                    root_pid,
                    start_generation,
                    history_generation,
                },
            )
        })
        .collect()
}

fn current_metric_keys(targets: &[TaskMetricsTarget]) -> HashSet<TaskMetricsKey> {
    targets
        .iter()
        .map(|target| TaskMetricsKey {
            session: target.session.clone(),
            task: target.task.clone(),
        })
        .collect()
}

fn observed_processes(system: &System) -> Vec<ObservedProcess> {
    system
        .processes()
        .iter()
        .map(|(pid, process)| ObservedProcess {
            pid: pid.as_u32(),
            ppid: process.parent().map(Pid::as_u32),
            name: process.name().to_string_lossy().into_owned(),
            cpu_percent: process.cpu_usage(),
            memory_bytes: process.memory(),
            status: format!("{:?}", process.status()).to_lowercase(),
            run_time_seconds: process.run_time(),
        })
        .collect()
}

fn aggregate_process_tree(
    root_pid: u32,
    processes: &[ObservedProcess],
) -> Option<AggregatedProcessTree> {
    let process_map = processes
        .iter()
        .cloned()
        .map(|process| (process.pid, process))
        .collect::<HashMap<_, _>>();
    if !process_map.contains_key(&root_pid) {
        return None;
    }

    let mut children_by_parent = HashMap::<u32, Vec<u32>>::new();
    for process in processes {
        if let Some(ppid) = process.ppid {
            children_by_parent
                .entry(ppid)
                .or_default()
                .push(process.pid);
        }
    }

    let mut stack = vec![root_pid];
    let mut process_rows = Vec::new();
    let mut aggregate = TaskMetricsAggregate::zero();
    while let Some(pid) = stack.pop() {
        let Some(process) = process_map.get(&pid) else {
            continue;
        };
        aggregate.cpu_percent += process.cpu_percent;
        aggregate.memory_bytes += process.memory_bytes;
        aggregate.process_count += 1;
        process_rows.push(TaskProcessSnapshot {
            pid: process.pid,
            ppid: process.ppid,
            name: process.name.clone(),
            cpu_percent: process.cpu_percent,
            memory_bytes: process.memory_bytes,
            status: process.status.clone(),
            run_time_seconds: process.run_time_seconds,
        });
        if let Some(children) = children_by_parent.get(&pid) {
            stack.extend(children.iter().rev().copied());
        }
    }
    process_rows.sort_by_key(|process| (u8::from(process.pid != root_pid), process.pid));
    Some(AggregatedProcessTree {
        aggregate,
        processes: process_rows,
    })
}

fn running_metric_targets(targets: &[TaskMetricsTarget]) -> Vec<&TaskMetricsTarget> {
    targets
        .iter()
        .filter(|target| target.root_pid.is_some())
        .collect()
}

fn record_task_metrics_for_targets(
    state: &DaemonState,
    targets: &[TaskMetricsTarget],
    timestamp_ms: u64,
    processes: &[ObservedProcess],
) {
    let current_keys = current_metric_keys(targets);
    let mut observations = HashMap::<TaskMetricsKey, Option<AggregatedProcessTree>>::new();
    {
        let sessions = state.sessions.lock().expect("sessions lock");
        for target in targets {
            let is_current = sessions
                .get(&target.session)
                .and_then(|runtime| runtime.task_metric_identity(&target.task))
                .is_some_and(|(pid, generation, history_generation)| {
                    pid == target.root_pid
                        && generation == target.start_generation
                        && history_generation == target.history_generation
                });
            let observation = if is_current {
                target
                    .root_pid
                    .and_then(|root_pid| aggregate_process_tree(root_pid, processes))
            } else {
                None
            };
            observations.insert(
                TaskMetricsKey {
                    session: target.session.clone(),
                    task: target.task.clone(),
                },
                observation,
            );
        }
    }

    let service_updates = targets
        .iter()
        .map(|target| {
            let key = TaskMetricsKey {
                session: target.session.clone(),
                task: target.task.clone(),
            };
            let pids = observations
                .get(&key)
                .and_then(Option::as_ref)
                .map(|observation| {
                    observation
                        .processes
                        .iter()
                        .map(|process| process.pid)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let (endpoints, inspection) = service::inspect_listeners(&pids);
            (target, endpoints, inspection)
        })
        .collect::<Vec<_>>();
    {
        let mut sessions = state.sessions.lock().expect("sessions lock");
        for (target, endpoints, inspection) in service_updates {
            let is_current = sessions
                .get(&target.session)
                .and_then(|runtime| runtime.task_metric_identity(&target.task))
                .is_some_and(|(pid, generation, history_generation)| {
                    pid == target.root_pid
                        && generation == target.start_generation
                        && history_generation == target.history_generation
                });
            if is_current {
                if let Some(runtime) = sessions.get_mut(&target.session) {
                    runtime.set_service_observation(&target.task, endpoints, inspection);
                }
            }
        }
    }

    // History can be cleared while listener inspection is running. Revalidate while
    // holding the same sessions -> metrics lock order as clear/restart so a stale
    // observation cannot repopulate a newly cleared metrics entry.
    let sessions = state.sessions.lock().expect("sessions lock");
    let current_keys = current_keys
        .into_iter()
        .filter(|key| {
            sessions
                .get(&key.session)
                .and_then(|runtime| runtime.task_metric_identity(&key.task))
                .is_some()
        })
        .collect::<HashSet<_>>();
    let mut metrics = state.task_metrics.lock().expect("task metrics lock");
    metrics.retain_current_tasks(&current_keys);
    for target in targets {
        let is_current = sessions
            .get(&target.session)
            .and_then(|runtime| runtime.task_metric_identity(&target.task))
            .is_some_and(|(pid, generation, history_generation)| {
                pid == target.root_pid
                    && generation == target.start_generation
                    && history_generation == target.history_generation
            });
        if !is_current {
            continue;
        }
        let key = TaskMetricsKey {
            session: target.session.clone(),
            task: target.task.clone(),
        };
        let observation = observations.remove(&key).unwrap_or(None);
        metrics.record(
            target.session.clone(),
            target.task.clone(),
            timestamp_ms,
            observation,
        );
    }
}

fn sample_task_metrics_with<F>(state: &DaemonState, timestamp_ms: u64, mut load_processes: F)
where
    F: FnMut() -> Vec<ObservedProcess>,
{
    let targets = collect_metric_targets(state);
    if targets.is_empty() {
        state
            .task_metrics
            .lock()
            .expect("task metrics lock")
            .retain_current_tasks(&HashSet::new());
        return;
    }

    let processes = if running_metric_targets(&targets).is_empty() {
        Vec::new()
    } else {
        load_processes()
    };
    record_task_metrics_for_targets(state, &targets, timestamp_ms, &processes);
}

fn refresh_processes_for_metrics(system: &mut System) -> Vec<ObservedProcess> {
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .without_tasks(),
    );
    observed_processes(system)
}

fn sample_task_metrics(state: &DaemonState, system: &mut System) {
    let timestamp_ms = current_timestamp_ms();
    sample_task_metrics_with(state, timestamp_ms, || {
        refresh_processes_for_metrics(system)
    });
}

fn panic_message(payload: Box<dyn std::any::Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

fn spawn_task_metrics_sampler(state: DaemonState) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut system = System::new();
        while !state.shutdown.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(TASK_METRICS_SAMPLE_INTERVAL_MS));
            if state.shutdown.load(Ordering::SeqCst) {
                break;
            }
            sample_task_metrics(&state, &mut system);
            sample_node_metrics(&state, &mut system);
        }
    })
}

pub struct GlobalPaths {
    pub root: PathBuf,
    pub socket: PathBuf,
    pub lock: PathBuf,
    pub log: PathBuf,
}

impl GlobalPaths {
    pub fn discover() -> Result<Self> {
        let root = if let Some(path) = std::env::var_os("TASKDECK_HOME") {
            PathBuf::from(path)
        } else {
            let home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .context("neither HOME nor USERPROFILE is set")?;
            PathBuf::from(home).join(".taskdeck")
        };
        Ok(Self {
            socket: ipc_path(&root),
            lock: root.join("daemon.lock"),
            log: root.join("daemon.log"),
            root,
        })
    }

    pub fn prepare(&self) -> Result<()> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("failed to create {}", self.root.display()))
    }
}

#[cfg(unix)]
fn ipc_path(root: &std::path::Path) -> PathBuf {
    root.join("taskdeck.sock")
}

#[cfg(windows)]
fn ipc_path(root: &std::path::Path) -> PathBuf {
    let hash = root
        .to_string_lossy()
        .to_lowercase()
        .bytes()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    PathBuf::from(format!(r"\\.\pipe\taskdeck-{hash:016x}"))
}

pub async fn run(web_port_override: Option<u16>) -> Result<()> {
    let paths = GlobalPaths::discover()?;
    paths.prepare()?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&paths.lock)
        .context("failed to open daemon lock")?;
    lock.try_lock_exclusive()
        .map_err(|_| anyhow::anyhow!("taskdeck daemon is already running"))?;

    #[cfg(unix)]
    let listener = {
        if paths.socket.exists() {
            let _ = fs::remove_file(&paths.socket);
        }
        UnixListener::bind(&paths.socket)
            .with_context(|| format!("failed to bind {}", paths.socket.display()))?
    };
    #[cfg(windows)]
    let mut listener = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&paths.socket)
        .with_context(|| format!("failed to create named pipe {}", paths.socket.display()))?;
    let state = DaemonState::load(&paths)?;
    let public_settings = state.public_settings();
    let worker_settings = state.settings.lock().expect("node settings lock").clone();
    let web_port = web_port_override.unwrap_or(public_settings.web_port);
    let web_listener =
        tokio::net::TcpListener::bind((public_settings.bind_host.as_str(), web_port))
            .await
            .with_context(|| {
                format!(
                    "failed to bind Web UI to {}:{web_port}",
                    public_settings.bind_host
                )
            })?;
    let metrics_sampler = spawn_task_metrics_sampler(state.clone());
    let history_sampler = spawn_task_history_sampler(state.clone());
    let task_scheduler = spawn_task_scheduler(state.clone());
    let scaling_evaluator = spawn_scaling_evaluator(state.clone());
    let worker_client =
        if worker_settings.role == NodeRole::Worker && worker_settings.leader_url.is_some() {
            Some(spawn_worker_client(state.clone(), worker_settings))
        } else {
            None
        };
    let web_state = state.clone();
    let web_task = tokio::spawn(async move { web::serve(web_state, web_listener).await });

    while !state.shutdown.load(Ordering::SeqCst) {
        #[cfg(unix)]
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let connection_state = state.clone();
                        tokio::spawn(async move {
                            let _ = serve_connection(connection_state, stream).await;
                        });
                    }
                    Err(error) => eprintln!("IPC accept error: {error}"),
                }
            }
            _ = tokio::signal::ctrl_c() => {
                state.shutdown.store(true, Ordering::SeqCst);
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
        #[cfg(windows)]
        tokio::select! {
            connected = listener.connect() => {
                match connected {
                    Ok(()) => {
                        let stream = listener;
                        listener = ServerOptions::new()
                            .create(&paths.socket)
                            .with_context(|| {
                                format!(
                                    "failed to create named pipe {}",
                                    paths.socket.display()
                                )
                            })?;
                        let connection_state = state.clone();
                        tokio::spawn(async move {
                            let _ = serve_connection(connection_state, stream).await;
                        });
                    }
                    Err(error) => eprintln!("IPC accept error: {error}"),
                }
            }
            _ = tokio::signal::ctrl_c() => {
                state.shutdown.store(true, Ordering::SeqCst);
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }

    web_task.abort();
    if let Some(worker_client) = worker_client {
        worker_client.abort();
    }
    let metrics_panic = metrics_sampler.join().err();
    if let Some(handle) = scaling_evaluator {
        let _ = handle.join();
    }
    for (name, handle) in [("history", history_sampler), ("scheduler", task_scheduler)] {
        if let Err(payload) = handle.join() {
            eprintln!("{name} worker panicked: {}", panic_message(payload));
        }
    }
    if let Some(payload) = metrics_panic {
        let message = panic_message(payload);
        eprintln!("task metrics sampler thread panicked: {message}");
        stop_all(&state);
        #[cfg(unix)]
        let _ = fs::remove_file(&paths.socket);
        drop(lock);
        bail!("task metrics sampler thread panicked: {message}");
    }
    stop_all(&state);
    #[cfg(unix)]
    let _ = fs::remove_file(&paths.socket);
    drop(lock);
    Ok(())
}

fn stop_all(state: &DaemonState) {
    let mut sessions = state.sessions.lock().expect("sessions lock");
    for session in sessions.values_mut() {
        session.stop_all();
    }
}

async fn serve_connection<S>(state: DaemonState, stream: S) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        let response = match Envelope::parse_line(&line) {
            Ok(envelope) => match envelope.audit {
                Some(audit) => {
                    dispatch_async_with_audit(state.clone(), envelope.request, Some(audit)).await
                }
                None => dispatch_async(state.clone(), envelope.request).await,
            },
            Err(error) => Response::error(format!("invalid request: {error}")),
        };
        let mut payload = serde_json::to_vec(&response)?;
        payload.push(b'\n');
        writer.write_all(&payload).await?;
    }
    Ok(())
}

pub async fn dispatch_async(state: DaemonState, request: Request) -> Response {
    dispatch_async_with_audit(state, request, None).await
}

pub async fn dispatch_async_with_audit(
    state: DaemonState,
    request: Request,
    audit: Option<AuditContext>,
) -> Response {
    match tokio::task::spawn_blocking(move || dispatch_with_audit(&state, request, audit)).await {
        Ok(response) => response,
        Err(error) => Response::error(format!("request worker failed: {error}")),
    }
}

#[cfg(test)]
fn dispatch(state: &DaemonState, request: Request) -> Response {
    dispatch_with_audit(state, request, None)
}

fn dispatch_with_audit(
    state: &DaemonState,
    request: Request,
    audit: Option<AuditContext>,
) -> Response {
    let context = audit
        .unwrap_or_else(AuditContext::internal)
        .with_request_defaults(&request);
    let started_at_ms = current_timestamp_ms();
    let started = Instant::now();
    let result = handle(state, request.clone());
    let response = match result {
        Ok(response) => response,
        Err(error) => Response::error(format!("{error:#}")),
    };
    let status = AuditStatus::from_ok(response.ok);
    let executor_node_id = state
        .store
        .node_settings()
        .ok()
        .map(|settings| settings.node_id);
    record_request_audit(
        state,
        &request,
        context,
        None,
        started_at_ms,
        started.elapsed().as_millis() as u64,
        status,
        executor_node_id,
        &response,
        serde_json::json!({}),
    );
    response
}

#[allow(clippy::too_many_arguments)]
pub fn record_audit_value(
    state: &DaemonState,
    context: AuditContext,
    audit_id: Option<String>,
    request_kind: &str,
    operation: &str,
    session: Option<&str>,
    task: Option<&str>,
    status: AuditStatus,
    started_at_ms: u64,
    duration_ms: u64,
    request: serde_json::Value,
    response: serde_json::Value,
    details: serde_json::Value,
    executor_node_id: Option<String>,
) -> Result<AuditRecord> {
    let settings = state.store.node_settings()?;
    let mut context = context;
    if context.origin_node_id.is_none() {
        context.origin_node_id = Some(settings.node_id.clone());
    }
    let success = matches!(status, AuditStatus::Success | AuditStatus::Started);
    let error = (!success)
        .then(|| {
            response
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("request failed")
        })
        .map(truncate_error_summary);
    let replicated_at_ms = if settings.role == NodeRole::Worker {
        None
    } else {
        Some(current_timestamp_ms())
    };
    state.store.record_audit(AuditRecord {
        audit_id: audit_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        correlation_id: context.correlation_id,
        timestamp_ms: started_at_ms,
        duration_ms,
        source: context.source,
        transport: context.transport,
        origin_node_id: context.origin_node_id,
        executor_node_id: executor_node_id.or(Some(settings.node_id)),
        request_kind: request_kind.to_string(),
        operation: operation.to_string(),
        session: session.map(str::to_string).or(context.session),
        task: task.map(str::to_string).or(context.task),
        status,
        success,
        error,
        request,
        response,
        details,
        replicated_at_ms,
    })
}

#[allow(clippy::too_many_arguments)]
fn record_request_audit(
    state: &DaemonState,
    request: &Request,
    context: AuditContext,
    audit_id: Option<String>,
    started_at_ms: u64,
    duration_ms: u64,
    status: AuditStatus,
    executor_node_id: Option<String>,
    response: &Response,
    mut details: serde_json::Value,
) {
    if let Some(origin_audit_id) = context.origin_audit_id.as_deref() {
        if let Some(object) = details.as_object_mut() {
            object.insert(
                "origin_audit_id".to_string(),
                serde_json::Value::String(origin_audit_id.to_string()),
            );
        }
    }
    let request_value = serde_json::to_value(request).unwrap_or_else(|error| {
        serde_json::json!({"serialization_error": error.to_string(), "kind": request.kind()})
    });
    let response_value = serde_json::to_value(response).unwrap_or_else(
        |error| serde_json::json!({"serialization_error": error.to_string(), "ok": response.ok}),
    );
    if let Err(error) = record_audit_value(
        state,
        context,
        audit_id,
        request.kind(),
        &request.operation(),
        request.session(),
        request.task(),
        status,
        started_at_ms,
        duration_ms,
        request_value,
        response_value,
        details,
        executor_node_id,
    ) {
        eprintln!("failed to persist audit record: {error:#}");
    }
}

fn truncate_error_summary(message: &str) -> String {
    const LIMIT: usize = 512;
    if message.len() <= LIMIT {
        message.to_string()
    } else {
        format!("{}...", message.chars().take(LIMIT).collect::<String>())
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare_session_config_write(
    state: &DaemonState,
    project: &std::path::Path,
    revision: &str,
    workspace_env: Option<&std::collections::BTreeMap<String, String>>,
    tasks: Vec<crate::protocol::EditableTaskInput>,
) -> std::result::Result<config::PreparedSessionConfigWrite, config::WriteConfigError> {
    #[cfg(not(test))]
    let _ = state;

    #[cfg(test)]
    {
        config::prepare_session_config_write_for_test(
            project,
            revision,
            tasks,
            workspace_env,
            state.put_config_post_check_delay(),
        )
    }

    #[cfg(not(test))]
    {
        config::prepare_session_config_write(project, revision, tasks, workspace_env)
    }
}

fn config_write_error_response(error: config::WriteConfigError) -> Result<Response> {
    match error {
        config::WriteConfigError::StaleRevision { current_revision } => {
            Ok(Response::error_with_data(
                "stale config revision",
                serde_json::json!({
                    "kind": "stale_revision",
                    "status": 409,
                    "current_revision": current_revision,
                }),
            ))
        }
        config::WriteConfigError::Validation { message } => Ok(Response::error_with_data(
            &message,
            serde_json::json!({
                "kind": "validation_error",
                "status": 400,
            }),
        )),
        config::WriteConfigError::Other(error) => Err(error),
    }
}

fn handle(state: &DaemonState, request: Request) -> Result<Response> {
    match request {
        Request::Ping => Ok(Response::empty("pong")),
        Request::Register { project, session } => {
            require_local_execution(state)?;
            let _config_mutation = state
                .config_mutations
                .lock()
                .expect("config mutations lock");
            let definition = config::discover(&project, session.as_deref())?;
            let name = definition.session.clone();
            let project = definition.project.clone();
            let mut sessions = state.sessions.lock().expect("sessions lock");
            if let Some(existing) = sessions.get_mut(&name) {
                if !existing.same_project(&project) {
                    bail!(
                        "session '{name}' already belongs to another project; choose a different --session"
                    );
                }
                return Ok(Response::ok(
                    format!(
                        "session '{name}' already registered; use update to reload configuration"
                    ),
                    existing.snapshot(200)?,
                ));
            }
            let mut runtime = SessionRuntime::new(definition);
            runtime.auto_start();
            let snapshot = runtime.snapshot(200)?;
            state.store.upsert_registration(&name, &project)?;
            sessions.insert(name.clone(), runtime);
            state
                .unavailable_sessions
                .lock()
                .expect("unavailable sessions lock")
                .remove(&name);
            Ok(Response::ok(
                format!("registered session '{name}'"),
                snapshot,
            ))
        }
        Request::Update { project, session } => {
            require_local_execution(state)?;
            let _config_mutation = state
                .config_mutations
                .lock()
                .expect("config mutations lock");
            let mut definition = config::discover(&project, session.as_deref())?;
            let configured_name = definition.session.clone();
            let project = definition.project.clone();
            let mut sessions = state.sessions.lock().expect("sessions lock");
            let name = if session.is_some() {
                configured_name
            } else {
                let matches = sessions
                    .iter()
                    .filter(|(_, runtime)| runtime.same_project(&project))
                    .map(|(name, _)| name.clone())
                    .collect::<Vec<_>>();
                if matches.iter().any(|name| name == &configured_name) {
                    configured_name
                } else {
                    match matches.as_slice() {
                        [name] => name.clone(),
                        [] => configured_name,
                        _ => bail!(
                            "multiple sessions are registered for project {}; specify --session",
                            project.display()
                        ),
                    }
                }
            };
            let runtime = sessions
                .get_mut(&name)
                .with_context(|| format!("session '{name}' is not registered"))?;
            if !runtime.same_project(&project) {
                bail!("session '{name}' belongs to another project");
            }
            definition.session = name.clone();
            runtime.update(definition)?;
            let snapshot = runtime.snapshot(200)?;
            Ok(Response::ok(format!("updated session '{name}'"), snapshot))
        }
        Request::ListSessions => {
            require_local_execution(state)?;
            let sessions = state.sessions.lock().expect("sessions lock");
            let unavailable = state
                .unavailable_sessions
                .lock()
                .expect("unavailable sessions lock");
            let mut names = sessions
                .keys()
                .chain(unavailable.keys())
                .cloned()
                .collect::<Vec<_>>();
            names.sort();
            names.dedup();
            Ok(Response::ok("sessions", names))
        }
        Request::Snapshot { session, tail } => {
            require_local_execution(state)?;
            reject_unavailable_session(state, &session)?;
            let mut sessions = state.sessions.lock().expect("sessions lock");
            let runtime = sessions
                .get_mut(&session)
                .with_context(|| format!("session '{session}' not found"))?;
            Ok(Response::ok(
                "snapshot",
                runtime.snapshot(tail.unwrap_or(500))?,
            ))
        }
        Request::ListTaskRuns { filter } => {
            let records = state.store.list_task_runs(&filter)?;
            Ok(Response::ok("task runs", records))
        }
        Request::ListEvents { filter } => {
            let events = state.store.list_events(&filter)?;
            Ok(Response::ok("events", events))
        }
        Request::TaskLogs {
            session,
            task,
            after,
            limit,
        } => {
            require_local_execution(state)?;
            reject_unavailable_session(state, &session)?;
            let mut sessions = state.sessions.lock().expect("sessions lock");
            let runtime = sessions
                .get_mut(&session)
                .with_context(|| format!("session '{session}' not found"))?;
            Ok(Response::ok(
                "task logs",
                runtime.task_logs(&task, after, limit)?,
            ))
        }
        Request::TaskMetrics {
            session,
            task,
            window_seconds,
        } => {
            require_local_execution(state)?;
            reject_unavailable_session(state, &session)?;
            {
                let sessions = state.sessions.lock().expect("sessions lock");
                let runtime = sessions
                    .get(&session)
                    .with_context(|| format!("session '{session}' not found"))?;
                if !runtime.has_task(&task) {
                    bail!("task '{task}' not found in session '{session}'");
                }
            }
            let metrics = state.task_metrics.lock().expect("task metrics lock");
            Ok(Response::ok(
                "task metrics",
                metrics.snapshot(&session, &task, window_seconds),
            ))
        }
        Request::ClearTaskHistory { session, task } => {
            require_local_execution(state)?;
            reject_unavailable_session(state, &session)?;
            let mut sessions = state.sessions.lock().expect("sessions lock");
            let runtime = sessions
                .get_mut(&session)
                .with_context(|| format!("session '{session}' not found"))?;
            runtime.clear_task_history(&task)?;
            state
                .task_metrics
                .lock()
                .expect("task metrics lock")
                .clear_task(&session, &task);
            Ok(Response::empty(format!(
                "cleared history for task '{task}'"
            )))
        }
        Request::GetSessionConfig { session } => {
            require_local_execution(state)?;
            reject_unavailable_session(state, &session)?;
            let sessions = state.sessions.lock().expect("sessions lock");
            let runtime = sessions
                .get(&session)
                .with_context(|| format!("session '{session}' not found"))?;
            let snapshot = config::read_session_config(runtime.project(), &session)?;
            Ok(Response::ok("session config", snapshot))
        }
        Request::PutSessionConfig {
            session,
            revision,
            workspace_env,
            tasks,
        } => {
            require_local_execution(state)?;
            reject_unavailable_session(state, &session)?;
            let _config_mutation = state
                .config_mutations
                .lock()
                .expect("config mutations lock");
            let (project, session_names) = {
                let sessions = state.sessions.lock().expect("sessions lock");
                let runtime = sessions
                    .get(&session)
                    .with_context(|| format!("session '{session}' not found"))?;
                let project = runtime.project().to_path_buf();
                let names = sessions
                    .iter()
                    .filter(|(_, runtime)| runtime.same_project(&project))
                    .map(|(name, _)| name.clone())
                    .collect::<Vec<_>>();
                (project, names)
            };

            let mut prepared = match prepare_session_config_write(
                state,
                &project,
                &revision,
                workspace_env.as_ref(),
                tasks,
            ) {
                Ok(prepared) => prepared,
                Err(error) => return config_write_error_response(error),
            };

            let definitions = session_names
                .iter()
                .map(|name| {
                    prepared
                        .project_definition(name)
                        .map(|definition| (name.clone(), definition))
                })
                .collect::<Result<Vec<_>>>()?;

            #[cfg(test)]
            if let Some(content) = state.take_put_config_before_finalize_content() {
                fs::write(project.join(config::PROJECT_CONFIG), content)?;
            }

            if let Err(error) = prepared.finalize() {
                return config_write_error_response(error);
            }

            let mut reconciliation_errors = Vec::new();
            {
                let mut sessions = state.sessions.lock().expect("sessions lock");
                for (index, (name, definition)) in definitions.into_iter().enumerate() {
                    #[cfg(not(test))]
                    let _ = index;

                    #[cfg(test)]
                    if state.put_config_runtime_failure_after() == Some(index) {
                        reconciliation_errors.push(serde_json::json!({
                            "session": name,
                            "message": "simulated runtime update failure",
                        }));
                        continue;
                    }

                    match sessions.get_mut(&name) {
                        Some(runtime) => {
                            if let Err(error) = runtime.update(definition) {
                                reconciliation_errors.push(serde_json::json!({
                                    "session": name,
                                    "message": format!("{error:#}"),
                                }));
                            }
                        }
                        None => reconciliation_errors.push(serde_json::json!({
                            "session": name,
                            "message": "session is no longer registered",
                        })),
                    }
                }
            }

            let snapshot = prepared.session_snapshot(&session)?;
            if !reconciliation_errors.is_empty() {
                return Ok(Response::error_with_data(
                    "configuration saved to disk, but runtime reconciliation was incomplete",
                    serde_json::json!({
                        "kind": "reconciliation_error",
                        "status": 500,
                        "saved": true,
                        "current_revision": snapshot.revision,
                        "errors": reconciliation_errors,
                    }),
                ));
            }

            Ok(Response::ok(
                format!("updated config for session '{session}'"),
                snapshot,
            ))
        }
        Request::Action {
            session,
            task,
            action,
        } => {
            require_local_execution(state)?;
            reject_unavailable_session(state, &session)?;
            if matches!(
                action,
                crate::protocol::Action::Start | crate::protocol::Action::Restart
            ) {
                let gate = match task.as_deref() {
                    Some(task_label) => {
                        if matches!(action, crate::protocol::Action::Start) {
                            state.check_start_gates(&session, task_label)
                        } else {
                            state.check_dependencies(&session, task_label)
                        }
                    }
                    None => state.check_quotas(&session).and_then(|_| Ok(())),
                };
                if let Err(reason) = gate {
                    bail!("start blocked: {reason}");
                }
            }
            let mut sessions = state.sessions.lock().expect("sessions lock");
            let mut pre_stop_runs: Vec<(String, u64)> = Vec::new();
            if matches!(
                action,
                crate::protocol::Action::Stop | crate::protocol::Action::Restart
            ) {
                if let Ok(snapshot) = sessions.get_mut(&session).expect("checked").snapshot(0) {
                    let task_filter = task.as_deref();
                    for value in snapshot.tasks.into_values() {
                        if task_filter.is_none_or(|label| label == value.label)
                            && matches!(value.status, TaskStatus::Running | TaskStatus::Paused)
                            && value.run_generation > 0
                        {
                            pre_stop_runs.push((value.label, value.run_generation));
                        }
                    }
                }
            }
            let runtime = sessions
                .get_mut(&session)
                .with_context(|| format!("session '{session}' not found"))?;
            let effects = runtime.apply(task.as_deref(), action)?;
            let timestamp_ms = current_timestamp_ms();
            let mut metrics = state.task_metrics.lock().expect("task metrics lock");
            for effect in effects.iter().filter(|effect| effect.restarted) {
                if effect.history_cleared {
                    metrics.clear_task(&session, &effect.task);
                } else {
                    metrics.mark_restart(&session, &effect.task, timestamp_ms);
                }
            }
            drop(metrics);
            drop(sessions);
            if matches!(
                action,
                crate::protocol::Action::Stop | crate::protocol::Action::Restart
            ) {
                let node_id = state
                    .store
                    .node_settings()
                    .map(|v| v.node_id)
                    .unwrap_or_default();
                for (stopped_task, generation) in pre_stop_runs {
                    let _ = state.store.finish_task_run(
                        &node_id,
                        &session,
                        &stopped_task,
                        generation,
                        "stopped",
                        None,
                        None,
                    );
                }
            }
            let mut sessions = state.sessions.lock().expect("sessions lock");
            let runtime = sessions
                .get_mut(&session)
                .with_context(|| format!("session '{session}' not found"))?;
            Ok(Response::ok(
                format!("{action:?} completed"),
                runtime.snapshot(50)?,
            ))
        }
        Request::RemoveSession { session } => {
            require_local_execution(state)?;
            let mut sessions = state.sessions.lock().expect("sessions lock");
            if let Some(mut runtime) = sessions.remove(&session) {
                runtime.stop_all();
            } else if state
                .unavailable_sessions
                .lock()
                .expect("unavailable sessions lock")
                .remove(&session)
                .is_none()
            {
                bail!("session '{session}' not found");
            }
            state.store.remove_registration(&session)?;
            drop(sessions);
            state
                .task_metrics
                .lock()
                .expect("task metrics lock")
                .remove_session(&session);
            Ok(Response::empty(format!("removed session '{session}'")))
        }
        Request::ListWorkspaces => {
            require_local_execution(state)?;
            Ok(Response::ok(
                "workspaces",
                state.store.workspace_summaries()?,
            ))
        }
        Request::SetWorkspaceAlias { session, alias } => {
            require_local_execution(state)?;
            let normalized = alias
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            state
                .store
                .set_registration_alias(&session, normalized.as_deref())?;
            let mut sessions = state.sessions.lock().expect("sessions lock");
            if let Some(runtime) = sessions.get_mut(&session) {
                runtime.set_alias(normalized);
            }
            let summary = state
                .store
                .workspace_summaries()?
                .into_iter()
                .find(|workspace| workspace.session == session)
                .context("session disappeared while setting alias")?;
            Ok(Response::ok("workspace alias updated", summary))
        }
        Request::GetNodeSettings => Ok(Response::ok(
            "node settings",
            state.store.node_settings_view()?,
        )),
        Request::PutNodeSettings { patch } => {
            let result = state.store.configure_patch(patch)?;
            Ok(Response::ok(
                if result.restart_required {
                    "saved; restart required"
                } else {
                    "saved"
                },
                result,
            ))
        }
        Request::Shutdown => {
            state.shutdown.store(true, Ordering::SeqCst);
            Ok(Response::empty("daemon shutdown requested"))
        }
    }
}

fn require_local_execution(state: &DaemonState) -> Result<()> {
    if state.execution_enabled() {
        Ok(())
    } else {
        bail!("this node is a pure master and does not provide local task execution")
    }
}

fn reject_unavailable_session(state: &DaemonState, session: &str) -> Result<()> {
    if let Some(unavailable) = state
        .unavailable_sessions
        .lock()
        .expect("unavailable sessions lock")
        .get(session)
    {
        bail!(
            "session '{}' is unavailable from {}: {}",
            unavailable.session,
            unavailable.project.display(),
            unavailable.error
        );
    }
    Ok(())
}

pub async fn request(request: &Request) -> Result<Response> {
    request_from(request, AuditSource::Cli).await
}

pub async fn request_from(request: &Request, source: AuditSource) -> Result<Response> {
    let paths = GlobalPaths::discover()?;
    let audit = client_audit_context(&paths, request, source);
    #[cfg(unix)]
    let stream = UnixStream::connect(&paths.socket)
        .await
        .with_context(|| format!("cannot connect to daemon at {}", paths.socket.display()))?;
    #[cfg(windows)]
    let stream = loop {
        match ClientOptions::new().open(&paths.socket) {
            Ok(stream) => break stream,
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("cannot connect to daemon at {}", paths.socket.display())
                });
            }
        }
    };
    request_on_stream(request, audit, stream).await
}

fn client_audit_context(
    paths: &GlobalPaths,
    request: &Request,
    source: AuditSource,
) -> AuditContext {
    let mut audit = AuditContext::new(source, AuditTransport::Ipc).with_request_defaults(request);
    if let Ok(settings) = StateStore::open(&paths.root).and_then(|store| store.node_settings()) {
        audit.origin_node_id = Some(settings.node_id);
    }
    audit
}

async fn request_on_stream<S>(request: &Request, audit: AuditContext, stream: S) -> Result<Response>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let envelope = Envelope::new(request.clone(), audit);
    let mut payload = serde_json::to_vec(&envelope)?;
    payload.push(b'\n');
    writer.write_all(&payload).await?;
    let mut lines = BufReader::new(reader).lines();
    let line = lines
        .next_line()
        .await?
        .context("daemon closed connection without a response")?;
    serde_json::from_str(&line).context("invalid daemon response")
}

pub async fn is_running() -> bool {
    matches!(request(&Request::Ping).await, Ok(response) if response.ok)
}

pub fn open_daemon_log() -> Result<File> {
    let paths = GlobalPaths::discover()?;
    paths.prepare()?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.log)
        .context("failed to open daemon log")
}

pub fn socket_path() -> Result<PathBuf> {
    Ok(GlobalPaths::discover()?.socket)
}

pub fn root_path() -> Result<PathBuf> {
    Ok(GlobalPaths::discover()?.root)
}

pub fn configured_settings() -> Result<crate::state::PublicNodeSettings> {
    let paths = GlobalPaths::discover()?;
    Ok(StateStore::open(&paths.root)?.node_settings()?.public())
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::{Duration, Instant};

    use serde_json::json;

    use super::*;
    use crate::protocol::{
        EditableTaskInput, McpCallRecord, SessionConfigSnapshot, TaskMetricsAggregate,
    };

    fn call_record() -> McpCallRecord {
        McpCallRecord {
            id: 0,
            tool: "taskdeck_control".to_string(),
            operation: Some("sessions".to_string()),
            started_at_ms: 1,
            duration_ms: 2,
            success: true,
            target_node: None,
            request: json!({"method": "tools/call"}),
            response: json!({"result": {"isError": false}}),
        }
    }

    #[test]
    fn dispatch_with_audit_records_success_and_error_sources() {
        let state = DaemonState::new();
        let cli_context =
            AuditContext::new(AuditSource::Cli, AuditTransport::Ipc).with_origin_node("cli-origin");
        let response = dispatch_with_audit(&state, Request::Ping, Some(cli_context));
        assert!(response.ok);

        let page = state
            .store
            .list_audit(&crate::protocol::AuditFilter {
                q: None,
                source: Some("cli".to_string()),
                status: Some("success".to_string()),
                node: Some("cli-origin".to_string()),
                session: None,
                task: None,
                operation: Some("ping".to_string()),
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(page.total, 1);
        let success = state
            .store
            .audit_detail(&page.items[0].audit_id)
            .unwrap()
            .unwrap();
        assert_eq!(success.source, AuditSource::Cli);
        assert_eq!(success.transport, AuditTransport::Ipc);
        assert_eq!(success.origin_node_id.as_deref(), Some("cli-origin"));
        assert!(success.executor_node_id.is_some());

        let response = dispatch_with_audit(
            &state,
            Request::Snapshot {
                session: "missing".to_string(),
                tail: None,
            },
            Some(AuditContext::new(AuditSource::Tui, AuditTransport::Ipc)),
        );
        assert!(!response.ok);
        let errors = state
            .store
            .list_audit(&crate::protocol::AuditFilter {
                q: Some("session 'missing'".to_string()),
                source: Some("tui".to_string()),
                status: Some("error".to_string()),
                node: None,
                session: Some("missing".to_string()),
                task: None,
                operation: Some("snapshot".to_string()),
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(errors.total, 1);
        assert!(
            errors.items[0]
                .error
                .as_deref()
                .unwrap()
                .contains("missing")
        );
    }

    #[tokio::test]
    async fn dispatch_node_compatibility_wrapper_still_audits_failures() {
        let state = DaemonState::new();
        let response = state
            .dispatch_node(
                "remote-on-worker",
                crate::cluster::RemoteRequest::ListSessions,
            )
            .await;
        assert!(!response.ok);
        let page = state
            .store
            .list_audit(&crate::protocol::AuditFilter {
                q: Some("worker nodes can only control".to_string()),
                source: Some("internal".to_string()),
                status: Some("error".to_string()),
                node: None,
                session: None,
                task: None,
                operation: Some("list_sessions".to_string()),
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(page.total, 1);
        let detail = state
            .store
            .audit_detail(&page.items[0].audit_id)
            .unwrap()
            .unwrap();
        assert_eq!(detail.details["node"], "remote-on-worker");
    }

    #[tokio::test]
    async fn leader_remote_worker_failures_are_audited_with_requested_executor() {
        let state = DaemonState::new();
        let settings = state
            .store
            .configure(crate::state::NodeSettingsUpdate {
                role: Some(crate::state::NodeRole::Leader),
                ..crate::state::NodeSettingsUpdate::default()
            })
            .unwrap();
        *state.settings.lock().expect("node settings lock") = settings.clone();
        let response = state
            .dispatch_node_with_audit(
                "missing-worker",
                crate::cluster::RemoteRequest::ListSessions,
                AuditContext::new(AuditSource::Web, AuditTransport::Http),
            )
            .await;
        assert!(!response.ok);

        let page = state
            .store
            .list_audit(&crate::protocol::AuditFilter {
                q: Some("worker 'missing-worker' not found".to_string()),
                source: Some("web".to_string()),
                status: Some("error".to_string()),
                node: Some("missing-worker".to_string()),
                session: None,
                task: None,
                operation: Some("list_sessions".to_string()),
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(page.total, 1);
        let detail = state
            .store
            .audit_detail(&page.items[0].audit_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            detail.origin_node_id.as_deref(),
            Some(settings.node_id.as_str())
        );
        assert_eq!(detail.executor_node_id.as_deref(), Some("missing-worker"));
        assert_eq!(detail.source, AuditSource::Web);
    }

    #[test]
    fn history_sampler_records_a_subsecond_run_and_exit_code() {
        let state = DaemonState::new();
        let project = tempfile::tempdir().unwrap();
        let mut runtime = SessionRuntime::new(crate::config::ProjectDefinition {
            session: "demo".to_string(),
            project: project.path().to_path_buf(),
            source: "taskdeck.yaml".to_string(),
            tasks: std::collections::BTreeMap::from([(
                "quick".to_string(),
                crate::config::TaskSpec {
                    label: "quick".to_string(),
                    program: "exit".to_string(),
                    args: vec!["7".to_string()],
                    cwd: project.path().to_path_buf(),
                    env: Default::default(),
                    shell: true,
                    auto_start: false,
                    stop_timeout_ms: 500,
                    clear_logs_on_restart: false,
                    schedule: None,
                },
            )]),
            task_order: vec!["quick".to_string()],
        });
        runtime
            .apply(Some("quick"), crate::protocol::Action::Start)
            .unwrap();
        state
            .sessions
            .lock()
            .expect("sessions lock")
            .insert("demo".to_string(), runtime);

        let mut tracked = std::collections::HashMap::new();
        let mut transitions = collect_run_transitions(&state, &mut tracked);
        for _ in 0..100 {
            if transitions.len() >= 2 {
                break;
            }
            thread::sleep(Duration::from_millis(10));
            transitions.extend(collect_run_transitions(&state, &mut tracked));
        }
        assert_eq!(transitions.len(), 2);
        let node_id = state.store.node_settings().unwrap().node_id;
        for transition in transitions {
            match transition {
                RunTransition::Started(session, snapshot, trigger) => {
                    state
                        .store
                        .record_task_run_start(&node_id, &snapshot, &trigger, &session)
                        .unwrap();
                }
                RunTransition::Finished {
                    session,
                    task,
                    generation,
                    status,
                    exit_code,
                    error_message,
                } => {
                    assert!(
                        state
                            .store
                            .finish_task_run(
                                &node_id,
                                &session,
                                &task,
                                generation,
                                &status,
                                exit_code,
                                error_message.as_deref(),
                            )
                            .unwrap()
                    );
                }
            }
        }

        let runs = state
            .store
            .list_task_runs(&crate::protocol::TaskRunFilter {
                session: Some("demo".into()),
                task: Some("quick".into()),
                status: None,
                trigger: None,
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(runs.total, 1);
        assert_eq!(runs.items[0].trigger, "manual");
        assert_eq!(runs.items[0].status, "failed");
        assert_eq!(runs.items[0].exit_code, Some(7));
        assert!(runs.items[0].duration_ms.is_some());
    }

    #[test]
    fn history_sampler_finishes_a_stopped_run_with_exit_code() {
        let state = DaemonState::new();
        let project = tempfile::tempdir().unwrap();
        let mut runtime = SessionRuntime::new(crate::config::ProjectDefinition {
            session: "demo".to_string(),
            project: project.path().to_path_buf(),
            source: "taskdeck.yaml".to_string(),
            tasks: std::collections::BTreeMap::from([(
                "long".to_string(),
                crate::config::TaskSpec {
                    label: "long".to_string(),
                    program: "trap 'exit 0' TERM; while :; do sleep 1; done".to_string(),
                    args: Vec::new(),
                    cwd: project.path().to_path_buf(),
                    env: Default::default(),
                    shell: true,
                    auto_start: false,
                    stop_timeout_ms: 3000,
                    clear_logs_on_restart: false,
                    schedule: None,
                },
            )]),
            task_order: vec!["long".to_string()],
        });
        runtime
            .apply(Some("long"), crate::protocol::Action::Start)
            .unwrap();
        state
            .sessions
            .lock()
            .expect("sessions lock")
            .insert("demo".to_string(), runtime);

        let mut tracked = std::collections::HashMap::new();
        let started = collect_run_transitions(&state, &mut tracked);
        assert_eq!(started.len(), 1);
        assert!(matches!(started[0], RunTransition::Started(_, _, _)));

        state
            .sessions
            .lock()
            .expect("sessions lock")
            .get_mut("demo")
            .unwrap()
            .apply(Some("long"), crate::protocol::Action::Stop)
            .unwrap();
        let finished = collect_run_transitions(&state, &mut tracked);
        assert_eq!(finished.len(), 1);
        let RunTransition::Finished {
            exit_code,
            session,
            task,
            generation,
            status,
            error_message,
        } = &finished[0]
        else {
            panic!("expected finished transition");
        };
        assert_eq!(*exit_code, None);
        assert_eq!(status, "stopped");
        assert!(
            error_message
                .as_deref()
                .is_some_and(|message| message.contains("signal"))
        );
        let node_id = state.store.node_settings().unwrap().node_id;
        if let RunTransition::Started(_, snapshot, trigger) = &started[0] {
            state
                .store
                .record_task_run_start(&node_id, snapshot, trigger, session)
                .unwrap();
        }
        state
            .store
            .finish_task_run(
                &node_id,
                session,
                task,
                *generation,
                status,
                *exit_code,
                error_message.as_deref(),
            )
            .unwrap();
        let runs = state
            .store
            .list_task_runs(&crate::protocol::TaskRunFilter {
                session: Some("demo".into()),
                task: Some("long".into()),
                status: None,
                trigger: None,
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(runs.items[0].status, "stopped");
        assert_eq!(runs.items[0].exit_code, None);
        assert!(
            runs.items[0]
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("signal"))
        );
    }

    #[test]
    fn mcp_call_history_is_persisted_with_database_ids() {
        let state = DaemonState::new();
        for _ in 0..3 {
            let _ = state.store.record_mcp_call(call_record());
        }
        let page = state
            .store
            .list_mcp_calls(None, None, None, None, None, 1, 20)
            .unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.items[0].id, 3);
        assert_eq!(page.items[0].tool, "taskdeck_control");
        assert!(state.store.mcp_call_detail(1).unwrap().is_some());
    }

    #[test]
    fn worker_binds_to_all_interfaces_by_default() {
        let state = DaemonState::new();
        assert_eq!(state.public_settings().bind_host, "0.0.0.0");
    }

    #[test]
    fn restores_registered_project_after_state_recreation() {
        let root = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        fs::write(
            project.path().join("taskdeck.yaml"),
            "version: 1\nsession: restored\ntasks:\n  idle:\n    command: echo\n    args: [ready]\n",
        )
        .unwrap();
        let project = project.path().canonicalize().unwrap();
        let paths = GlobalPaths {
            root: root.path().to_path_buf(),
            socket: root.path().join("taskdeck.sock"),
            lock: root.path().join("daemon.lock"),
            log: root.path().join("daemon.log"),
        };
        StateStore::open(root.path())
            .unwrap()
            .upsert_registration("restored", &project)
            .unwrap();

        let state = DaemonState::load(&paths).unwrap();
        let response = dispatch(&state, Request::ListSessions);
        assert!(response.ok);
        assert_eq!(response.data.unwrap(), json!(["restored"]));
    }

    #[test]
    fn pure_master_rejects_local_registration() {
        let state = DaemonState::new();
        let settings = state
            .store
            .configure(crate::state::NodeSettingsUpdate {
                role: Some(crate::state::NodeRole::Leader),
                leader_mode: Some(crate::state::LeaderMode::PureMaster),
                ..crate::state::NodeSettingsUpdate::default()
            })
            .unwrap();
        *state.settings.lock().expect("node settings lock") = settings;

        let response = dispatch(
            &state,
            Request::Register {
                project: PathBuf::from("/tmp/missing"),
                session: None,
            },
        );
        assert!(!response.ok);
        assert!(response.message.contains("pure master"));
    }

    fn task_input(label: &str, command: &str) -> EditableTaskInput {
        EditableTaskInput {
            label: label.to_string(),
            command: command.to_string(),
            args: Vec::new(),
            cwd: ".".to_string(),
            env: Default::default(),
            shell: true,
            auto_start: false,
            stop_timeout_ms: 3_000,
            clear_logs_on_restart: false,
            schedule: None,
        }
    }

    fn observed_process(
        pid: u32,
        ppid: Option<u32>,
        cpu_percent: f32,
        memory_bytes: u64,
    ) -> ObservedProcess {
        ObservedProcess {
            pid,
            ppid,
            name: format!("proc-{pid}"),
            cpu_percent,
            memory_bytes,
            status: "run".to_string(),
            run_time_seconds: pid as u64,
        }
    }

    fn wait_for(deadline: Duration, mut condition: impl FnMut() -> bool) {
        let start = Instant::now();
        while start.elapsed() < deadline {
            if condition() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(condition(), "condition was not met within {deadline:?}");
    }

    #[test]
    fn aggregates_only_the_root_process_and_its_descendants() {
        let tree = aggregate_process_tree(
            10,
            &[
                observed_process(10, Some(1), 25.0, 100),
                observed_process(11, Some(10), 10.0, 40),
                observed_process(12, Some(11), 5.0, 20),
                observed_process(99, Some(1), 80.0, 999),
            ],
        )
        .unwrap();

        assert_eq!(
            tree.aggregate,
            TaskMetricsAggregate {
                cpu_percent: 40.0,
                memory_bytes: 160,
                process_count: 3,
            }
        );
        assert_eq!(
            tree.processes
                .iter()
                .map(|process| process.pid)
                .collect::<Vec<_>>(),
            vec![10, 11, 12]
        );
    }

    #[test]
    fn task_metrics_store_truncates_history_and_preserves_it_after_stop() {
        let mut store = TaskMetricsStore::default();
        for timestamp_ms in 1..=(MAX_TASK_METRIC_SAMPLES as u64 + 5) {
            store.record(
                "demo",
                "api",
                timestamp_ms,
                Some(AggregatedProcessTree {
                    aggregate: TaskMetricsAggregate {
                        cpu_percent: timestamp_ms as f32,
                        memory_bytes: timestamp_ms * 10,
                        process_count: 2,
                    },
                    processes: vec![TaskProcessSnapshot {
                        pid: 42,
                        ppid: Some(1),
                        name: "api".to_string(),
                        cpu_percent: timestamp_ms as f32,
                        memory_bytes: timestamp_ms * 10,
                        status: "run".to_string(),
                        run_time_seconds: timestamp_ms,
                    }],
                }),
            );
        }

        let running = store.snapshot("demo", "api", 600);
        assert!(running.running);
        assert_eq!(running.samples.len(), MAX_TASK_METRIC_SAMPLES);
        assert_eq!(running.samples.first().unwrap().timestamp_ms, 6);
        assert_eq!(running.samples.last().unwrap().timestamp_ms, 605);

        store.record("demo", "api", 606, None);
        let stopped = store.snapshot("demo", "api", 600);
        assert!(!stopped.running);
        assert_eq!(stopped.current, TaskMetricsAggregate::zero());
        assert!(stopped.processes.is_empty());
        assert_eq!(stopped.samples.len(), MAX_TASK_METRIC_SAMPLES);
        assert_eq!(stopped.samples.first().unwrap().timestamp_ms, 6);
        assert_eq!(stopped.samples.last().unwrap().timestamp_ms, 605);
    }

    #[test]
    fn task_metrics_restart_markers_are_windowed_and_clear_with_history() {
        let mut store = TaskMetricsStore::default();
        for timestamp_ms in [100, 200, 300] {
            store.record(
                "demo",
                "api",
                timestamp_ms,
                Some(AggregatedProcessTree {
                    aggregate: TaskMetricsAggregate::zero(),
                    processes: Vec::new(),
                }),
            );
        }
        store.mark_restart("demo", "api", 150);
        store.mark_restart("demo", "api", 250);

        assert_eq!(store.snapshot("demo", "api", 2).restart_markers_ms, [250]);
        store.clear_task("demo", "api");
        let cleared = store.snapshot("demo", "api", 600);
        assert!(cleared.samples.is_empty());
        assert!(cleared.restart_markers_ms.is_empty());
        assert!(cleared.processes.is_empty());
    }

    #[test]
    fn task_metrics_prune_deleted_tasks_but_keep_existing_stopped_history() {
        let state = DaemonState::new();
        state.sessions.lock().expect("sessions lock").insert(
            "demo".to_string(),
            SessionRuntime::new(crate::config::ProjectDefinition {
                session: "demo".to_string(),
                project: PathBuf::from("/tmp"),
                source: "taskdeck.yaml".to_string(),
                tasks: std::collections::BTreeMap::from([(
                    "worker".to_string(),
                    crate::config::TaskSpec {
                        label: "worker".to_string(),
                        program: "sleep".to_string(),
                        args: vec!["60".to_string()],
                        cwd: PathBuf::from("/tmp"),
                        env: Default::default(),
                        shell: false,
                        auto_start: false,
                        stop_timeout_ms: 500,
                        clear_logs_on_restart: false,

                        schedule: None,
                    },
                )]),
                task_order: vec!["worker".to_string()],
            }),
        );

        let worker_observation = Some(AggregatedProcessTree {
            aggregate: TaskMetricsAggregate {
                cpu_percent: 5.0,
                memory_bytes: 50,
                process_count: 1,
            },
            processes: vec![TaskProcessSnapshot {
                pid: 7,
                ppid: Some(1),
                name: "worker".to_string(),
                cpu_percent: 5.0,
                memory_bytes: 50,
                status: "run".to_string(),
                run_time_seconds: 1,
            }],
        });
        let mut metrics = state.task_metrics.lock().expect("task metrics lock");
        metrics.record("demo", "api", 1, worker_observation.clone());
        metrics.record("demo", "worker", 2, worker_observation);
        drop(metrics);

        sample_task_metrics_with(&state, 3, || panic!("refresh should not run"));

        let metrics = state.task_metrics.lock().expect("task metrics lock");
        assert_eq!(metrics.entries.len(), 1);
        assert!(metrics.entries.contains_key(&TaskMetricsKey {
            session: "demo".to_string(),
            task: "worker".to_string(),
        }));
        let worker = metrics.snapshot("demo", "worker", 600);
        assert!(!worker.running);
        assert_eq!(worker.samples.len(), 1);
        assert_eq!(metrics.snapshot("demo", "api", 600).samples.len(), 0);
    }

    #[test]
    fn sample_task_metrics_skips_refresh_when_no_running_targets_exist() {
        let state = DaemonState::new();
        state.sessions.lock().expect("sessions lock").insert(
            "demo".to_string(),
            SessionRuntime::new(crate::config::ProjectDefinition {
                session: "demo".to_string(),
                project: PathBuf::from("/tmp"),
                source: "taskdeck.yaml".to_string(),
                tasks: std::collections::BTreeMap::from([(
                    "idle".to_string(),
                    crate::config::TaskSpec {
                        label: "idle".to_string(),
                        program: "sleep".to_string(),
                        args: vec!["60".to_string()],
                        cwd: PathBuf::from("/tmp"),
                        env: Default::default(),
                        shell: false,
                        auto_start: false,
                        stop_timeout_ms: 500,
                        clear_logs_on_restart: false,

                        schedule: None,
                    },
                )]),
                task_order: vec!["idle".to_string()],
            }),
        );

        let mut refreshes = 0usize;
        sample_task_metrics_with(&state, 1, || {
            refreshes += 1;
            Vec::new()
        });
        assert_eq!(refreshes, 0);
    }

    #[test]
    fn stale_generation_targets_are_discarded_before_recording() {
        let state = DaemonState::new();
        let mut runtime = SessionRuntime::new(crate::config::ProjectDefinition {
            session: "demo".to_string(),
            project: PathBuf::from("/tmp"),
            source: "taskdeck.yaml".to_string(),
            tasks: std::collections::BTreeMap::from([(
                "clock".to_string(),
                crate::config::TaskSpec {
                    label: "clock".to_string(),
                    program: "while true; do sleep 1; done".to_string(),
                    args: Vec::new(),
                    cwd: PathBuf::from("/tmp"),
                    env: Default::default(),
                    shell: true,
                    auto_start: false,
                    stop_timeout_ms: 500,
                    clear_logs_on_restart: false,

                    schedule: None,
                },
            )]),
            task_order: vec!["clock".to_string()],
        });
        runtime
            .apply(Some("clock"), crate::protocol::Action::Start)
            .unwrap();
        state
            .sessions
            .lock()
            .expect("sessions lock")
            .insert("demo".to_string(), runtime);

        let stale_targets = collect_metric_targets(&state);
        {
            let mut sessions = state.sessions.lock().expect("sessions lock");
            let runtime = sessions.get_mut("demo").unwrap();
            runtime
                .apply(Some("clock"), crate::protocol::Action::Restart)
                .unwrap();
        }

        record_task_metrics_for_targets(
            &state,
            &stale_targets,
            10,
            &[observed_process(
                stale_targets[0].root_pid.unwrap(),
                Some(1),
                33.0,
                99,
            )],
        );

        let snapshot = state
            .task_metrics
            .lock()
            .expect("task metrics lock")
            .snapshot("demo", "clock", 600);
        assert!(!snapshot.running);
        assert_eq!(snapshot.current, TaskMetricsAggregate::zero());
        assert!(snapshot.samples.is_empty());

        state
            .sessions
            .lock()
            .expect("sessions lock")
            .get_mut("demo")
            .unwrap()
            .stop_all();
    }

    #[test]
    fn collect_metric_targets_excludes_naturally_exited_tasks_without_snapshot_or_action() {
        let state = DaemonState::new();
        let mut runtime = SessionRuntime::new(crate::config::ProjectDefinition {
            session: "demo".to_string(),
            project: PathBuf::from("/tmp"),
            source: "taskdeck.yaml".to_string(),
            tasks: std::collections::BTreeMap::from([(
                "flash".to_string(),
                crate::config::TaskSpec {
                    label: "flash".to_string(),
                    program: "sleep 0.05".to_string(),
                    args: Vec::new(),
                    cwd: PathBuf::from("/tmp"),
                    env: Default::default(),
                    shell: true,
                    auto_start: false,
                    stop_timeout_ms: 500,
                    clear_logs_on_restart: false,

                    schedule: None,
                },
            )]),
            task_order: vec!["flash".to_string()],
        });
        runtime
            .apply(Some("flash"), crate::protocol::Action::Start)
            .unwrap();
        state
            .sessions
            .lock()
            .expect("sessions lock")
            .insert("demo".to_string(), runtime);

        wait_for(Duration::from_secs(2), || {
            collect_metric_targets(&state)
                .first()
                .is_some_and(|target| target.root_pid.is_none())
        });

        let targets = collect_metric_targets(&state);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].session, "demo");
        assert_eq!(targets[0].task, "flash");
        assert_eq!(targets[0].root_pid, None);
    }

    #[test]
    fn update_reloads_a_registered_project_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(config::PROJECT_CONFIG);
        fs::write(
            &config_path,
            "version: 1\nsession: demo\ntasks:\n  api:\n    command: echo old\n",
        )
        .unwrap();
        let state = DaemonState::new();

        let registered = handle(
            &state,
            Request::Register {
                project: dir.path().to_path_buf(),
                session: Some("custom".to_string()),
            },
        )
        .unwrap();
        assert!(registered.ok);

        fs::write(
            &config_path,
            "version: 1\nsession: demo\ntasks:\n  api:\n    command: echo new\n  worker:\n    command: echo worker\n",
        )
        .unwrap();
        let updated = handle(
            &state,
            Request::Update {
                project: dir.path().to_path_buf(),
                session: None,
            },
        )
        .unwrap();
        let snapshot: crate::protocol::SessionSnapshot =
            serde_json::from_value(updated.data.unwrap()).unwrap();

        assert_eq!(updated.message, "updated session 'custom'");
        assert_eq!(snapshot.name, "custom");
        assert_eq!(snapshot.tasks["api"].command, "echo new");
        assert!(snapshot.tasks.contains_key("worker"));
    }

    #[test]
    fn put_session_config_updates_all_registered_sessions_for_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(config::PROJECT_CONFIG);
        fs::write(
            &config_path,
            "version: 1\ntasks:\n  api:\n    command: echo old\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",
        )
        .unwrap();
        let state = DaemonState::new();

        assert!(
            handle(
                &state,
                Request::Register {
                    project: dir.path().to_path_buf(),
                    session: Some("one".to_string()),
                },
            )
            .unwrap()
            .ok
        );
        assert!(
            handle(
                &state,
                Request::Register {
                    project: dir.path().to_path_buf(),
                    session: Some("two".to_string()),
                },
            )
            .unwrap()
            .ok
        );

        let revision = serde_json::from_value::<SessionConfigSnapshot>(
            handle(
                &state,
                Request::GetSessionConfig {
                    session: "one".to_string(),
                },
            )
            .unwrap()
            .data
            .unwrap(),
        )
        .unwrap()
        .revision;

        let updated = handle(
            &state,
            Request::PutSessionConfig {
                session: "one".to_string(),
                revision,
                workspace_env: None,
                tasks: vec![
                    task_input("api", "echo new"),
                    task_input("worker", "echo worker"),
                ],
            },
        )
        .unwrap();
        let config: SessionConfigSnapshot = serde_json::from_value(updated.data.unwrap()).unwrap();

        assert!(updated.ok);
        assert_eq!(config.tasks.len(), 2);

        let one: crate::protocol::SessionSnapshot = serde_json::from_value(
            handle(
                &state,
                Request::Snapshot {
                    session: "one".to_string(),
                    tail: Some(20),
                },
            )
            .unwrap()
            .data
            .unwrap(),
        )
        .unwrap();
        let two: crate::protocol::SessionSnapshot = serde_json::from_value(
            handle(
                &state,
                Request::Snapshot {
                    session: "two".to_string(),
                    tail: Some(20),
                },
            )
            .unwrap()
            .data
            .unwrap(),
        )
        .unwrap();

        assert_eq!(one.tasks["api"].command, "echo new");
        assert_eq!(two.tasks["api"].command, "echo new");
        assert!(one.tasks.contains_key("worker"));
        assert!(two.tasks.contains_key("worker"));
    }

    #[test]
    fn put_session_config_reports_new_auto_start_failures() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(config::PROJECT_CONFIG);
        fs::write(
            &config_path,
            "version: 1\ntasks:\n  api:\n    command: echo ready\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",
        )
        .unwrap();
        let state = DaemonState::new();
        assert!(
            handle(
                &state,
                Request::Register {
                    project: dir.path().to_path_buf(),
                    session: Some("demo".to_string()),
                },
            )
            .unwrap()
            .ok
        );

        let config: SessionConfigSnapshot = serde_json::from_value(
            handle(
                &state,
                Request::GetSessionConfig {
                    session: "demo".to_string(),
                },
            )
            .unwrap()
            .data
            .unwrap(),
        )
        .unwrap();
        let mut broken = task_input("broken", "/taskdeck/no-such-executable");
        broken.shell = false;
        broken.auto_start = true;

        let response = handle(
            &state,
            Request::PutSessionConfig {
                session: "demo".to_string(),
                revision: config.revision,
                workspace_env: None,
                tasks: vec![task_input("api", "echo ready"), broken],
            },
        )
        .unwrap();

        assert!(!response.ok);
        let data = response.data.unwrap();
        assert_eq!(data["kind"], "reconciliation_error");
        assert_eq!(data["saved"], true);
        assert_eq!(data["errors"][0]["session"], "demo");
        assert!(
            data["errors"][0]["message"]
                .as_str()
                .unwrap()
                .contains("failed to auto-start new task")
        );

        let runtime = state.sessions.lock().expect("sessions lock");
        assert!(!runtime["demo"].has_task("broken"));
        assert!(fs::read_to_string(config_path).unwrap().contains("broken:"));
    }

    #[test]
    fn put_session_config_reports_stale_revision_conflicts_in_the_response_envelope() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(config::PROJECT_CONFIG);
        fs::write(
            &config_path,
            "version: 1\ntasks:\n  api:\n    command: echo old\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",
        )
        .unwrap();
        let state = DaemonState::new();

        assert!(
            handle(
                &state,
                Request::Register {
                    project: dir.path().to_path_buf(),
                    session: Some("one".to_string()),
                },
            )
            .unwrap()
            .ok
        );

        let snapshot: SessionConfigSnapshot = serde_json::from_value(
            handle(
                &state,
                Request::GetSessionConfig {
                    session: "one".to_string(),
                },
            )
            .unwrap()
            .data
            .unwrap(),
        )
        .unwrap();

        fs::write(
            &config_path,
            "version: 1\ntasks:\n  api:\n    command: echo newer\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",
        )
        .unwrap();

        let response = handle(
            &state,
            Request::PutSessionConfig {
                session: "one".to_string(),
                revision: snapshot.revision,
                workspace_env: None,
                tasks: vec![task_input("api", "echo changed")],
            },
        )
        .unwrap();

        assert!(!response.ok);
        assert_eq!(response.data.as_ref().unwrap()["kind"], "stale_revision");
        assert_eq!(response.data.as_ref().unwrap()["status"], 409);
    }

    #[test]
    fn concurrent_same_revision_puts_allow_exactly_one_winner() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(config::PROJECT_CONFIG);
        fs::write(
            &config_path,
            "version: 1\ntasks:\n  api:\n    command: echo old\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",
        )
        .unwrap();
        let state = DaemonState::new();
        state.set_put_config_post_check_delay(Duration::from_millis(100));

        assert!(
            handle(
                &state,
                Request::Register {
                    project: dir.path().to_path_buf(),
                    session: Some("one".to_string()),
                },
            )
            .unwrap()
            .ok
        );

        let snapshot: SessionConfigSnapshot = serde_json::from_value(
            handle(
                &state,
                Request::GetSessionConfig {
                    session: "one".to_string(),
                },
            )
            .unwrap()
            .data
            .unwrap(),
        )
        .unwrap();

        let barrier = Arc::new(Barrier::new(3));
        let first_state = state.clone();
        let first_barrier = barrier.clone();
        let first_revision = snapshot.revision.clone();
        let first = thread::spawn(move || {
            first_barrier.wait();
            handle(
                &first_state,
                Request::PutSessionConfig {
                    session: "one".to_string(),
                    revision: first_revision,
                    workspace_env: None,
                    tasks: vec![task_input("api", "echo one")],
                },
            )
            .unwrap()
        });

        let second_state = state.clone();
        let second_barrier = barrier.clone();
        let second_revision = snapshot.revision;
        let second = thread::spawn(move || {
            second_barrier.wait();
            handle(
                &second_state,
                Request::PutSessionConfig {
                    session: "one".to_string(),
                    revision: second_revision,
                    workspace_env: None,
                    tasks: vec![task_input("api", "echo two")],
                },
            )
            .unwrap()
        });

        barrier.wait();
        let responses = [first.join().unwrap(), second.join().unwrap()];
        let success_count = responses.iter().filter(|response| response.ok).count();
        let conflict_count = responses
            .iter()
            .filter(|response| {
                response
                    .data
                    .as_ref()
                    .is_some_and(|data| data["kind"] == "stale_revision")
            })
            .count();

        assert_eq!(success_count, 1);
        assert_eq!(conflict_count, 1);
    }

    #[test]
    fn put_session_config_detects_external_edit_before_finalize_without_updating_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(config::PROJECT_CONFIG);
        fs::write(
            &config_path,
            "version: 1\ntasks:\n  api:\n    command: echo old\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",
        )
        .unwrap();
        let state = DaemonState::new();
        state.set_put_config_before_finalize_content(
            "version: 1\ntasks:\n  api:\n    command: echo external\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",
        );

        assert!(
            handle(
                &state,
                Request::Register {
                    project: dir.path().to_path_buf(),
                    session: Some("one".to_string()),
                },
            )
            .unwrap()
            .ok
        );

        let snapshot: SessionConfigSnapshot = serde_json::from_value(
            handle(
                &state,
                Request::GetSessionConfig {
                    session: "one".to_string(),
                },
            )
            .unwrap()
            .data
            .unwrap(),
        )
        .unwrap();

        let response = handle(
            &state,
            Request::PutSessionConfig {
                session: "one".to_string(),
                revision: snapshot.revision,
                workspace_env: None,
                tasks: vec![task_input("api", "echo submitted")],
            },
        )
        .unwrap();

        assert!(!response.ok);
        assert_eq!(response.data.as_ref().unwrap()["kind"], "stale_revision");
        assert!(
            fs::read_to_string(&config_path)
                .unwrap()
                .contains("echo external")
        );
        let runtime: crate::protocol::SessionSnapshot = serde_json::from_value(
            handle(
                &state,
                Request::Snapshot {
                    session: "one".to_string(),
                    tail: Some(20),
                },
            )
            .unwrap()
            .data
            .unwrap(),
        )
        .unwrap();
        assert_eq!(runtime.tasks["api"].command, "echo old");
    }

    #[test]
    fn register_and_update_wait_for_the_config_mutation_lock() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(config::PROJECT_CONFIG);
        fs::write(
            &config_path,
            "version: 1\ntasks:\n  api:\n    command: echo old\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",
        )
        .unwrap();
        let state = DaemonState::new();

        let guard = state
            .config_mutations
            .lock()
            .expect("config mutations lock");
        let (register_tx, register_rx) = mpsc::channel();
        let register_state = state.clone();
        let register_project = dir.path().to_path_buf();
        let register = thread::spawn(move || {
            let response = handle(
                &register_state,
                Request::Register {
                    project: register_project,
                    session: Some("one".to_string()),
                },
            )
            .unwrap();
            register_tx.send(response.ok).unwrap();
        });
        assert!(register_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(guard);
        assert!(register_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        register.join().unwrap();

        let guard = state
            .config_mutations
            .lock()
            .expect("config mutations lock");
        let (update_tx, update_rx) = mpsc::channel();
        let update_state = state.clone();
        let update_project = dir.path().to_path_buf();
        let update = thread::spawn(move || {
            let response = handle(
                &update_state,
                Request::Update {
                    project: update_project,
                    session: Some("one".to_string()),
                },
            )
            .unwrap();
            update_tx.send(response.ok).unwrap();
        });
        assert!(update_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(guard);
        assert!(update_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        update.join().unwrap();
    }

    #[test]
    fn put_session_config_commits_then_reconciles_remaining_sessions_and_retry_converges() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(config::PROJECT_CONFIG);
        fs::write(
            &config_path,
            "version: 1\ntasks:\n  api:\n    command: echo old\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",
        )
        .unwrap();
        let state = DaemonState::new();
        for session in ["a", "b", "c"] {
            assert!(
                handle(
                    &state,
                    Request::Register {
                        project: dir.path().to_path_buf(),
                        session: Some(session.to_string()),
                    },
                )
                .unwrap()
                .ok
            );
        }
        state.set_put_config_runtime_failure_after(1);

        let snapshot: SessionConfigSnapshot = serde_json::from_value(
            handle(
                &state,
                Request::GetSessionConfig {
                    session: "a".to_string(),
                },
            )
            .unwrap()
            .data
            .unwrap(),
        )
        .unwrap();

        let response = handle(
            &state,
            Request::PutSessionConfig {
                session: "a".to_string(),
                revision: snapshot.revision,
                workspace_env: None,
                tasks: vec![task_input("api", "echo new")],
            },
        )
        .unwrap();

        assert!(!response.ok);
        assert!(response.message.contains("saved to disk"));
        let data = response.data.as_ref().unwrap();
        assert_eq!(data["kind"], "reconciliation_error");
        assert_eq!(data["status"], 500);
        assert_eq!(data["saved"], true);
        assert_eq!(data["errors"][0]["session"], "b");
        let current_revision = data["current_revision"].as_str().unwrap().to_string();
        assert!(
            fs::read_to_string(&config_path)
                .unwrap()
                .contains("echo new")
        );

        let runtime_command = |session: &str| {
            let snapshot: crate::protocol::SessionSnapshot = serde_json::from_value(
                handle(
                    &state,
                    Request::Snapshot {
                        session: session.to_string(),
                        tail: Some(20),
                    },
                )
                .unwrap()
                .data
                .unwrap(),
            )
            .unwrap();
            snapshot.tasks["api"].command.clone()
        };
        assert_eq!(runtime_command("a"), "echo new");
        assert_eq!(runtime_command("b"), "echo old");
        assert_eq!(runtime_command("c"), "echo new");

        state.clear_put_config_runtime_failure();
        let response = handle(
            &state,
            Request::PutSessionConfig {
                session: "a".to_string(),
                revision: current_revision,
                workspace_env: None,
                tasks: vec![task_input("api", "echo new")],
            },
        )
        .unwrap();
        assert!(response.ok);
        assert_eq!(runtime_command("a"), "echo new");
        assert_eq!(runtime_command("b"), "echo new");
        assert_eq!(runtime_command("c"), "echo new");
    }
}
