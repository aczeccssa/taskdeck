//! Task metric / history background samplers.

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

use super::gates::sample_node_metrics;
use super::notifications::RunTransition;
pub(super) fn spawn_task_history_sampler(state: DaemonState) -> thread::JoinHandle<()> {
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
pub(super) fn record_task_metrics_for_targets(
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

pub(super) fn sample_task_metrics_with<F>(state: &DaemonState, timestamp_ms: u64, mut load_processes: F)
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

pub(super) fn refresh_processes_for_metrics(system: &mut System) -> Vec<ObservedProcess> {
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

pub(super) fn sample_task_metrics(state: &DaemonState, system: &mut System) {
    let timestamp_ms = current_timestamp_ms();
    sample_task_metrics_with(state, timestamp_ms, || {
        refresh_processes_for_metrics(system)
    });
}

pub(super) fn panic_message(payload: Box<dyn std::any::Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

pub(super) fn spawn_task_metrics_sampler(state: DaemonState) -> thread::JoinHandle<()> {
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
