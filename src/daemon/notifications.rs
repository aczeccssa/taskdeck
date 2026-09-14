//! Run-transition notifications and webhook delivery.

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
use super::process_tree::{
    AggregatedProcessTree, ObservedProcess, aggregate_process_tree, collect_metric_targets,
    current_metric_keys, observed_processes, running_metric_targets,
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

pub(super) fn emit_transition_notifications(state: &DaemonState, transitions: &[RunTransition]) {
    let node_id = state.public_settings().node_id;
    emit_transition_notifications_for_node(state.store.clone(), &node_id, transitions);
}

/// Records matching lifecycle notifications on behalf of a local or remote executor.
///
/// The leader uses this for worker inventory transitions so alert rules configured in
/// the leader Web UI apply to work running on every connected worker.
pub(crate) fn emit_transition_notifications_for_node(
    store: Arc<StateStore>,
    node_id: &str,
    transitions: &[RunTransition],
) {
    let rules: Vec<NotificationRule> = match store.notification_rules() {
        Ok(rules) => rules.into_iter().filter(|rule| rule.enabled).collect(),
        Err(_) => return,
    };
    if rules.is_empty() {
        return;
    }
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
            match store.insert_notification(
                node_id,
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
                    store.clone(),
                    node_id.to_string(),
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
pub(super) fn spawn_webhook_delivery(
    store: Arc<StateStore>,
    node_id: String,
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
        "node_id": node_id,
        "session": session,
        "task": task,
        "title": title,
        "message": message,
        "details": details,
        "timestamp_ms": current_timestamp_ms(),
    });
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
pub(crate) enum RunTransition {
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

pub(super) fn trigger_for(
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

pub(crate) fn finished_run_details(
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

/// Derives lifecycle transitions from two worker inventory snapshots.
///
/// Worker snapshots are the leader's only view of remote task state. Applying the
/// same generation/status rules as the local sampler keeps leader alert rules and
/// the central inbox authoritative for remote executors too.
pub(crate) fn collect_inventory_transitions(
    previous: &[crate::protocol::SessionSnapshot],
    current: &[crate::protocol::SessionSnapshot],
) -> Vec<RunTransition> {
    let previous_tasks = previous
        .iter()
        .flat_map(|session| {
            session
                .tasks
                .values()
                .map(move |task| ((session.name.clone(), task.label.clone()), task.clone()))
        })
        .collect::<HashMap<_, _>>();
    let mut transitions = Vec::new();
    for session in current {
        for task_snapshot in session.tasks.values() {
            let previous = previous_tasks.get(&(session.name.clone(), task_snapshot.label.clone()));
            let previous_generation = previous.map(|task| task.run_generation);
            let generation_changed = previous_generation != Some(task_snapshot.run_generation);
            let previous_active = previous.is_some_and(|task| {
                matches!(task.status, TaskStatus::Running | TaskStatus::Paused)
            });
            let current_finished = matches!(
                task_snapshot.status,
                TaskStatus::Exited | TaskStatus::Failed | TaskStatus::Idle
            );
            let generation = task_snapshot.run_generation;

            if generation > 0 && generation_changed && previous_active {
                transitions.push(RunTransition::Finished {
                    session: session.name.clone(),
                    task: task_snapshot.label.clone(),
                    generation: previous_generation.expect("previous generation"),
                    status: "stopped".to_string(),
                    exit_code: None,
                    error_message: None,
                });
            }
            if generation > 0 && generation_changed {
                let trigger = if task_snapshot.auto_start {
                    "auto_start"
                } else {
                    "manual"
                };
                transitions.push(RunTransition::Started(
                    session.name.clone(),
                    Box::new(task_snapshot.clone()),
                    trigger.to_string(),
                ));
                if current_finished {
                    transitions.push(finished_run_details(&session.name, task_snapshot));
                }
            } else if generation > 0 && previous_active && current_finished {
                transitions.push(finished_run_details(&session.name, task_snapshot));
            }
        }
    }
    transitions
}

pub(super) fn collect_run_transitions(
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
