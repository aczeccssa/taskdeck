//! Task and node metric stores.

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
use super::notifications::{collect_run_transitions, emit_transition_notifications};
use super::process_tree::AggregatedProcessTree;
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

pub const TASK_METRICS_SAMPLE_INTERVAL_MS: u64 = 1_000;
pub const MAX_TASK_METRIC_SAMPLES: usize = 600;
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct TaskMetricsKey {
    pub(super) session: String,
    pub(super) task: String,
}

#[derive(Debug, Clone)]
pub(super) struct TaskMetricsTarget {
    pub(super) session: String,
    pub(super) task: String,
    pub(super) root_pid: Option<u32>,
    pub(super) start_generation: u64,
    pub(super) history_generation: u64,
}
#[derive(Debug, Clone, Default)]
pub(super) struct TaskMetricsEntry {
    pub(super) running: bool,
    pub(super) current: TaskMetricsAggregate,
    pub(super) processes: Vec<TaskProcessSnapshot>,
    pub(super) samples: VecDeque<TaskMetricsSample>,
    pub(super) restart_markers_ms: VecDeque<u64>,
}

impl TaskMetricsEntry {
pub(super)     fn apply_observation(&mut self, timestamp_ms: u64, observation: Option<AggregatedProcessTree>) {
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

pub(super)     fn snapshot(&self, window_seconds: usize) -> TaskMetricsSnapshot {
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

pub(super)     fn mark_restart(&mut self, timestamp_ms: u64) {
        self.restart_markers_ms.push_back(timestamp_ms);
        while self.restart_markers_ms.len() > MAX_TASK_METRIC_SAMPLES {
            self.restart_markers_ms.pop_front();
        }
    }
}

#[derive(Debug, Default)]
pub struct TaskMetricsStore {
    pub(super) entries: HashMap<TaskMetricsKey, TaskMetricsEntry>,
}

impl TaskMetricsStore {
pub(super)     fn record(
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

pub(super)     fn snapshot(&self, session: &str, task: &str, window_seconds: usize) -> TaskMetricsSnapshot {
        self.entries
            .get(&TaskMetricsKey {
                session: session.to_string(),
                task: task.to_string(),
            })
            .map(|entry| entry.snapshot(window_seconds))
            .unwrap_or_else(|| TaskMetricsEntry::default().snapshot(window_seconds))
    }

pub(super)     fn remove_session(&mut self, session: &str) {
        self.entries.retain(|key, _| key.session != session);
    }

pub(super)     fn clear_task(&mut self, session: &str, task: &str) {
        self.entries.remove(&TaskMetricsKey {
            session: session.to_string(),
            task: task.to_string(),
        });
    }

pub(super)     fn mark_restart(&mut self, session: &str, task: &str, timestamp_ms: u64) {
        self.entries
            .entry(TaskMetricsKey {
                session: session.to_string(),
                task: task.to_string(),
            })
            .or_default()
            .mark_restart(timestamp_ms);
    }

pub(super)     fn retain_current_tasks(&mut self, current: &HashSet<TaskMetricsKey>) {
        self.entries.retain(|key, _| current.contains(key));
    }
}
pub const MAX_NODE_METRIC_SAMPLES: usize = 300;
pub const SCALING_EVALUATION_INTERVAL_MS: u64 = 5_000;
#[derive(Debug, Default)]
pub struct NodeMetricsStore {
    pub(super) samples: Mutex<HashMap<String, std::collections::VecDeque<NodeMetricsSample>>>,
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
