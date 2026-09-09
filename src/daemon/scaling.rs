//! Elastic scaling policy evaluator.

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

pub const SCALING_EVALUATION_INTERVAL_MS: u64 = 5_000;
const SCALING_STREAK_THRESHOLD: u32 = 3;
const SCALING_REMOTE_FETCH_INTERVAL_MS: u64 = 10_000;
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

pub(super) fn evaluate_scaling_policies(
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

pub(super) fn scale_policy_task(
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

pub(super) fn task_running_on_node(
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
