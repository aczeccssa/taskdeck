//! DaemonState construction, settings and test hooks.

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

use super::audit::{record_audit_value, record_request_audit};
use super::client::GlobalPaths;
use super::dispatch::{
    config_write_error_response, prepare_session_config_write, reject_unavailable_session,
    require_local_execution,
};
use super::handle::handle;
use super::metrics::{
    MAX_TASK_METRIC_SAMPLES, NodeMetricsStore, TASK_METRICS_SAMPLE_INTERVAL_MS, TaskMetricsKey,
    TaskMetricsStore, TaskMetricsTarget,
};
use super::notifications::{collect_run_transitions, emit_transition_notifications};
use super::process_tree::{
    AggregatedProcessTree, ObservedProcess, aggregate_process_tree, collect_metric_targets,
    current_metric_keys, observed_processes, running_metric_targets,
};
use super::util::{ScheduleKey, current_timestamp_ms, status_label};
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

#[derive(Clone)]
pub struct DaemonState {
    pub store: Arc<StateStore>,
    pub settings: Arc<Mutex<NodeSettings>>,
    pub cluster: LeaderCluster,
    pub sessions: Arc<Mutex<Sessions>>,
    pub unavailable_sessions: Arc<Mutex<BTreeMap<String, UnavailableSession>>>,
    pub task_metrics: Arc<Mutex<TaskMetricsStore>>,
    pub node_metrics: Arc<NodeMetricsStore>,
    pub(super) run_triggers: Arc<Mutex<HashMap<ScheduleKey, String>>>,
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

        store.finish_running_task_runs(
            &settings.node_id,
            "failed",
            "daemon restarted before run completion",
        )?;

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
}

impl DaemonState {
    #[cfg(test)]

    pub fn set_put_config_post_check_delay(&self, delay: Duration) {
        *self
            .put_config_post_check_delay
            .lock()
            .expect("config write delay lock") = Some(delay);
    }

    #[cfg(test)]

    pub(super) fn put_config_post_check_delay(&self) -> Option<Duration> {
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

    pub(super) fn take_put_config_before_finalize_content(&self) -> Option<String> {
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

    pub(super) fn put_config_runtime_failure_after(&self) -> Option<usize> {
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
