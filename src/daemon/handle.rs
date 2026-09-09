//! IPC/HTTP request routing match (single responsibility: routing).

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

pub(super) fn handle(state: &DaemonState, request: Request) -> Result<Response> {
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
