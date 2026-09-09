//! Process-tree observation and aggregation for metrics.

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

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ObservedProcess {
    pub(super) pid: u32,
    pub(super) ppid: Option<u32>,
    pub(super) name: String,
    pub(super) cpu_percent: f32,
    pub(super) memory_bytes: u64,
    pub(super) status: String,
    pub(super) run_time_seconds: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct AggregatedProcessTree {
    pub(super) aggregate: TaskMetricsAggregate,
    pub(super) processes: Vec<TaskProcessSnapshot>,
}
pub(super) fn collect_metric_targets(state: &DaemonState) -> Vec<TaskMetricsTarget> {
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

pub(super) fn current_metric_keys(targets: &[TaskMetricsTarget]) -> HashSet<TaskMetricsKey> {
    targets
        .iter()
        .map(|target| TaskMetricsKey {
            session: target.session.clone(),
            task: target.task.clone(),
        })
        .collect()
}

pub(super) fn observed_processes(system: &System) -> Vec<ObservedProcess> {
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

pub(super) fn aggregate_process_tree(
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

pub(super) fn running_metric_targets(targets: &[TaskMetricsTarget]) -> Vec<&TaskMetricsTarget> {
    targets
        .iter()
        .filter(|target| target.root_pid.is_some())
        .collect()
}
