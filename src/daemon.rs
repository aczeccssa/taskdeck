use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

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
    McpCallRecord, Request, Response, TaskMetricsAggregate, TaskMetricsSample, TaskMetricsSnapshot,
    TaskProcessSnapshot, casefold_search_text,
};
use crate::runtime::{SessionRuntime, Sessions};
use crate::service;
use crate::state::{NodeRole, NodeSettings, StateStore};
use crate::web;

pub const MAX_MCP_CALLS: usize = 500;
pub const TASK_METRICS_SAMPLE_INTERVAL_MS: u64 = 1_000;
pub const MAX_TASK_METRIC_SAMPLES: usize = 600;

#[derive(Clone)]
pub struct DaemonState {
    pub store: Arc<StateStore>,
    pub settings: Arc<Mutex<NodeSettings>>,
    pub cluster: LeaderCluster,
    pub sessions: Arc<Mutex<Sessions>>,
    pub unavailable_sessions: Arc<Mutex<BTreeMap<String, UnavailableSession>>>,
    pub mcp_calls: Arc<Mutex<VecDeque<Arc<McpCallHistoryEntry>>>>,
    pub next_mcp_call_id: Arc<AtomicU64>,
    pub task_metrics: Arc<Mutex<TaskMetricsStore>>,
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
        Self {
            store,
            settings: Arc::new(Mutex::new(settings)),
            cluster,
            sessions: Arc::new(Mutex::new(Sessions::new())),
            unavailable_sessions: Arc::new(Mutex::new(BTreeMap::new())),
            mcp_calls: Arc::new(Mutex::new(VecDeque::new())),
            next_mcp_call_id: Arc::new(AtomicU64::new(1)),
            task_metrics: Arc::new(Mutex::new(TaskMetricsStore::default())),
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
        let mut sessions = Sessions::new();
        let mut unavailable_sessions = BTreeMap::new();
        if settings.execution_enabled() {
            for registration in store.registrations()? {
                match config::discover(&registration.project, Some(&registration.session)) {
                    Ok(definition) => {
                        let mut runtime = SessionRuntime::new(definition);
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
        Ok(Self {
            store,
            settings: Arc::new(Mutex::new(settings)),
            cluster,
            sessions: Arc::new(Mutex::new(sessions)),
            unavailable_sessions: Arc::new(Mutex::new(unavailable_sessions)),
            mcp_calls: Arc::new(Mutex::new(VecDeque::new())),
            next_mcp_call_id: Arc::new(AtomicU64::new(1)),
            task_metrics: Arc::new(Mutex::new(TaskMetricsStore::default())),
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
        self.sessions
            .lock()
            .expect("sessions lock")
            .values_mut()
            .filter_map(|runtime| runtime.snapshot(0).ok())
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

    pub async fn dispatch_node(&self, node: &str, request: RemoteRequest) -> Response {
        let settings = self.settings.lock().expect("node settings lock").clone();
        if node == "self" {
            if !settings.execution_enabled() {
                return Response::error("pure master does not have a self executor");
            }
            return dispatch_async(self.clone(), request.into_local()).await;
        }
        if settings.role != NodeRole::Leader {
            return Response::error("worker nodes can only control their local self executor");
        }
        self.cluster.request(node, request).await
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

    pub fn record_mcp_call(&self, mut record: McpCallRecord) {
        record.id = self.next_mcp_call_id.fetch_add(1, Ordering::Relaxed);
        let entry = Arc::new(McpCallHistoryEntry::new(record));
        let mut calls = self.mcp_calls.lock().expect("MCP calls lock");
        calls.push_front(entry);
        calls.truncate(MAX_MCP_CALLS);
    }

    pub fn recent_mcp_calls(&self, limit: usize) -> Vec<Arc<McpCallHistoryEntry>> {
        self.mcp_calls
            .lock()
            .expect("MCP calls lock")
            .iter()
            .take(limit.min(MAX_MCP_CALLS))
            .cloned()
            .collect()
    }

    pub fn mcp_call(&self, id: u64) -> Option<Arc<McpCallHistoryEntry>> {
        self.mcp_calls
            .lock()
            .expect("MCP calls lock")
            .iter()
            .find(|call| call.record.id == id)
            .cloned()
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
            .expect("config finalize hook lock") = Some(content.into());
    }

    #[cfg(test)]
    fn take_put_config_before_finalize_content(&self) -> Option<String> {
        self.put_config_before_finalize_content
            .lock()
            .expect("config finalize hook lock")
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
    pub fn clear_put_config_runtime_failure(&self) {
        *self
            .put_config_runtime_failure_after
            .lock()
            .expect("runtime failure hook lock") = None;
    }

    #[cfg(test)]
    fn put_config_runtime_failure_after(&self) -> Option<usize> {
        *self
            .put_config_runtime_failure_after
            .lock()
            .expect("runtime failure hook lock")
    }
}

#[derive(Debug)]
pub struct McpCallHistoryEntry {
    pub record: McpCallRecord,
    pub input: serde_json::Value,
    pub session: Option<String>,
    pub task: Option<String>,
    pub searchable_text: String,
}

impl McpCallHistoryEntry {
    fn new(record: McpCallRecord) -> Self {
        let input = record
            .request
            .pointer("/params/arguments")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let session = input
            .get("session")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let task = input
            .get("task")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let searchable_text = build_mcp_call_searchable_text(
            &record.tool,
            record.operation.as_deref(),
            record.target_node.as_deref(),
            session.as_deref(),
            task.as_deref(),
            &input,
        );
        Self {
            record,
            input,
            session,
            task,
            searchable_text,
        }
    }
}

fn build_mcp_call_searchable_text(
    tool: &str,
    operation: Option<&str>,
    target_node: Option<&str>,
    session: Option<&str>,
    task: Option<&str>,
    input: &serde_json::Value,
) -> String {
    let serialized_input = serde_json::to_string(input).unwrap_or_default();
    casefold_search_text(&format!(
        "{tool}\u{1f}{operation}\u{1f}{target_node}\u{1f}{session}\u{1f}{task}\u{1f}{serialized_input}",
        operation = operation.unwrap_or(""),
        target_node = target_node.unwrap_or(""),
        session = session.unwrap_or(""),
        task = task.unwrap_or(""),
    ))
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

fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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
    if let Err(payload) = metrics_sampler.join() {
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
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => dispatch_async(state.clone(), request).await,
            Err(error) => Response::error(format!("invalid request: {error}")),
        };
        let mut payload = serde_json::to_vec(&response)?;
        payload.push(b'\n');
        writer.write_all(&payload).await?;
    }
    Ok(())
}

pub async fn dispatch_async(state: DaemonState, request: Request) -> Response {
    match tokio::task::spawn_blocking(move || dispatch(&state, request)).await {
        Ok(response) => response,
        Err(error) => Response::error(format!("request worker failed: {error}")),
    }
}

fn dispatch(state: &DaemonState, request: Request) -> Response {
    let result = handle(state, request);
    match result {
        Ok(response) => response,
        Err(error) => Response::error(format!("{error:#}")),
    }
}

fn prepare_session_config_write(
    state: &DaemonState,
    project: &std::path::Path,
    revision: &str,
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
            state.put_config_post_check_delay(),
        )
    }

    #[cfg(not(test))]
    {
        config::prepare_session_config_write(project, revision, tasks)
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

            let mut prepared = match prepare_session_config_write(state, &project, &revision, tasks)
            {
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
            let mut sessions = state.sessions.lock().expect("sessions lock");
            let runtime = sessions
                .get_mut(&session)
                .with_context(|| format!("session '{session}' not found"))?;
            let effects = runtime.apply(task.as_deref(), action.clone())?;
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
    let paths = GlobalPaths::discover()?;
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
    request_on_stream(request, stream).await
}

async fn request_on_stream<S>(request: &Request, stream: S) -> Result<Response>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut payload = serde_json::to_vec(request)?;
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
    use crate::protocol::{EditableTaskInput, SessionConfigSnapshot, TaskMetricsAggregate};

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
    fn mcp_call_history_is_newest_first_and_bounded() {
        let state = DaemonState::new();
        for _ in 0..MAX_MCP_CALLS + 2 {
            state.record_mcp_call(call_record());
        }

        let calls = state.recent_mcp_calls(MAX_MCP_CALLS + 20);
        assert_eq!(calls.len(), MAX_MCP_CALLS);
        assert_eq!(calls.first().map(|call| call.record.id), Some(502));
        assert_eq!(calls.last().map(|call| call.record.id), Some(3));
        assert!(state.mcp_call(1).is_none());
        assert_eq!(
            state.mcp_call(502).map(|call| call.record.tool.clone()),
            Some("taskdeck_control".to_string())
        );
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
