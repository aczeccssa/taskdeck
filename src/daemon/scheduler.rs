//! Cron task scheduler loop.

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

fn record_scheduled_failure(state: &DaemonState, key: &ScheduleKey, error: impl Into<String>) {
    let error = error.into();
    let snapshot = state
        .sessions
        .lock()
        .ok()
        .and_then(|mut sessions| sessions.get_mut(&key.session)?.snapshot(0).ok())
        .and_then(|session| session.tasks.into_values().find(|task| task.label == key.task))
        .unwrap_or_else(|| crate::protocol::TaskSnapshot {
            label: key.task.clone(),
            status: TaskStatus::Failed,
            pid: None,
            command: String::new(),
            cwd: PathBuf::from("."),
            auto_start: false,
            last_exit: None,
            exit_code: None,
            logs: Vec::new(),
            run_generation: 0,
            started_at_ms: 0,
            schedule: None,
            service: Default::default(),
        });
    let node_id = state.public_settings().node_id;
    if let Err(persist_error) = state.store.record_task_run_failure(
        &node_id,
        &snapshot,
        "cron",
        &key.session,
        error,
    ) {
        eprintln!("failed to persist scheduled failure: {persist_error:#}");
    }
    state
        .run_triggers
        .lock()
        .expect("run trigger lock")
        .remove(key);
}

pub(super) fn spawn_task_scheduler(state: DaemonState) -> thread::JoinHandle<()> {
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
                            "reason": reason.clone(),
                        }),
                    );
                    record_scheduled_failure(
                        &state,
                        &key,
                        format!("scheduled start blocked: {reason}"),
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
                        state
                            .run_triggers
                            .lock()
                            .expect("run trigger lock")
                            .remove(&key);
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
                        record_scheduled_failure(&state, &key, error_message.clone());
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
                    None => {
                        drop(sessions);
                        record_scheduled_failure(&state, &key, "scheduled session disappeared");
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::TaskRunFilter;

    #[test]
    fn scheduled_failure_is_persisted_when_session_disappears() {
        let state = DaemonState::new();
        let key = ScheduleKey {
            session: "missing-session".to_string(),
            task: "missing-task".to_string(),
        };

        record_scheduled_failure(&state, &key, "scheduled session disappeared");

        let runs = state
            .store
            .list_task_runs(&TaskRunFilter {
                session: Some(key.session),
                task: Some(key.task),
                status: Some("failed".to_string()),
                trigger: Some("cron".to_string()),
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(runs.total, 1);
        assert_eq!(
            runs.items[0].error_message.as_deref(),
            Some("scheduled session disappeared")
        );
        assert!(runs.items[0].finished_at_ms.is_some());
    }
}
