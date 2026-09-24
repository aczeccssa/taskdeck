//! Daemon lifecycle: singleton lock, IPC listener, background jobs, web boot.
//!
//! `run()` is the composition root — it is the ONLY place that starts the web
//! server (`crate::web::serve`), so the module graph stays acyclic:
//! web -> daemon::{state,dispatch,metrics,...} -> (state, cluster, protocol).

mod audit;
mod client;
mod dispatch;
mod gates;
mod handle;
mod inventory;
mod metrics;
pub(crate) mod notifications;
mod process_tree;
mod sampler;
mod scaling;
mod scheduler;
mod state;
mod util;

#[cfg(test)]
mod tests;

// Interface-compat re-exports: downstream modules (web, cluster, tui, main,
// platform_service) keep using `crate::daemon::<item>` paths.
#[allow(unused_imports)]
pub use audit::record_audit_value;
#[allow(unused_imports)]
pub use client::{
    GlobalPaths, configured_settings, is_running, open_daemon_log, request, request_from,
    root_path, socket_path,
};
#[allow(unused_imports)]
pub use dispatch::{dispatch_async, dispatch_async_with_audit};
#[allow(unused_imports)]
pub use metrics::{
    MAX_NODE_METRIC_SAMPLES, MAX_TASK_METRIC_SAMPLES, NodeMetricsStore,
    TASK_METRICS_SAMPLE_INTERVAL_MS, TaskMetricsStore,
};
#[allow(unused_imports)]
pub use scaling::spawn_scaling_evaluator;
#[allow(unused_imports)]
pub use state::{DaemonState, UnavailableSession};

use std::fs::{self, OpenOptions};
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
#[cfg(unix)]
use tokio::net::UnixListener;
#[cfg(windows)]
use tokio::net::windows::named_pipe::ServerOptions;

use crate::cluster::spawn_worker_client;
use crate::protocol::{Envelope, Response};
use crate::state::NodeRole;
use crate::web;
use crate::update;
use sampler::{panic_message, spawn_task_history_sampler, spawn_task_metrics_sampler};
use scheduler::spawn_task_scheduler;

async fn serve_connection<S>(state: DaemonState, stream: S) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        let response = match Envelope::parse_line(&line) {
            Ok(envelope) => match envelope.audit {
                Some(audit) => {
                    dispatch_async_with_audit(state.clone(), envelope.request, Some(audit)).await
                }
                None => dispatch_async(state.clone(), envelope.request).await,
            },
            Err(error) => Response::error(format!("invalid request: {error}")),
        };
        let mut payload = serde_json::to_vec(&response)?;
        payload.push(b'\n');
        writer.write_all(&payload).await?;
    }
    Ok(())
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
    if update::enabled() {
        let update_state = state.clone();
        tokio::spawn(async move {
            let now = update::timestamp_ms();
            let last = update_state.store.metadata("update_last_checked_ms").ok().flatten().and_then(|v| v.parse().ok());
            if !update::should_check(last, now) { return; }
            let status = tokio::task::spawn_blocking(update::check).await.ok();
            if let Some(status) = status {
                let _ = update_state.store.set_metadata("update_last_checked_ms", &status.checked_at_ms.unwrap_or(now).to_string());
                if status.available {
                    let _ = update_state.store.record_event("update", "new Taskdeck release available", serde_json::json!({"version":status.latest_version,"url":status.release_url}));
                }
            }
        });
    }
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
    let history_sampler = spawn_task_history_sampler(state.clone());
    let task_scheduler = spawn_task_scheduler(state.clone());
    let scaling_evaluator = spawn_scaling_evaluator(state.clone());
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
    let metrics_panic = metrics_sampler.join().err();
    if let Some(handle) = scaling_evaluator {
        let _ = handle.join();
    }
    for (name, handle) in [("history", history_sampler), ("scheduler", task_scheduler)] {
        if let Err(payload) = handle.join() {
            eprintln!("{name} worker panicked: {}", panic_message(payload));
        }
    }
    if let Some(payload) = metrics_panic {
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
    drop(sessions);
    let node_id = state.public_settings().node_id;
    if let Err(error) = state.store.finish_running_task_runs(
        &node_id,
        "stopped",
        "daemon stopped while run was active",
    ) {
        eprintln!("failed to finish task run history during shutdown: {error:#}");
    }
}
