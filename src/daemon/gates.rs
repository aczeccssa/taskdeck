//! Quota/dependency start gates and local-run counting.

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
    config_write_error_response, prepare_session_config_write,
    reject_unavailable_session, require_local_execution,
};
use super::handle::handle;
use super::metrics::{
    MAX_TASK_METRIC_SAMPLES, NodeMetricsStore, TASK_METRICS_SAMPLE_INTERVAL_MS,
    TaskMetricsKey, TaskMetricsStore, TaskMetricsTarget,
};
use super::notifications::{collect_run_transitions, emit_transition_notifications};
use super::process_tree::{
    aggregate_process_tree, collect_metric_targets, current_metric_keys, observed_processes,
    running_metric_targets, AggregatedProcessTree, ObservedProcess,
};
use super::state::DaemonState;
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

impl DaemonState {
pub(super)     fn running_task_counts(&self) -> (usize, HashMap<String, usize>) {

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



pub(super)     fn task_status_on(&self, node: &str, session: &str, task: &str) -> Option<TaskStatus> {

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

pub(super) fn count_running_tasks(state: &DaemonState) -> u32 {
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

pub(super) fn sample_node_metrics(state: &DaemonState, system: &mut System) {
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
