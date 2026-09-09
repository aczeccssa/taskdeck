//! IPC client helpers: paths, connect, request, daemon log.

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

pub struct GlobalPaths {
    pub root: PathBuf,
    pub socket: PathBuf,
    pub lock: PathBuf,
    pub log: PathBuf,
}

impl GlobalPaths {
    pub fn discover() -> Result<Self> {
        let root = if let Some(path) = std::env::var_os("TASKDECK_HOME") {
            PathBuf::from(path)
        } else {
            let home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .context("neither HOME nor USERPROFILE is set")?;
            PathBuf::from(home).join(".taskdeck")
        };
        Ok(Self {
            socket: ipc_path(&root),
            lock: root.join("daemon.lock"),
            log: root.join("daemon.log"),
            root,
        })
    }

    pub fn prepare(&self) -> Result<()> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("failed to create {}", self.root.display()))
    }
}

#[cfg(unix)]
pub(super) fn ipc_path(root: &std::path::Path) -> PathBuf {
    root.join("taskdeck.sock")
}

#[cfg(windows)]
pub(super) fn ipc_path(root: &std::path::Path) -> PathBuf {
    let hash = root
        .to_string_lossy()
        .to_lowercase()
        .bytes()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    PathBuf::from(format!(r"\\.\pipe\taskdeck-{hash:016x}"))
}
pub async fn request(request: &Request) -> Result<Response> {
    request_from(request, AuditSource::Cli).await
}

pub async fn request_from(request: &Request, source: AuditSource) -> Result<Response> {
    let paths = GlobalPaths::discover()?;
    let audit = client_audit_context(&paths, request, source);
    #[cfg(unix)]
    let stream = UnixStream::connect(&paths.socket)
        .await
        .with_context(|| format!("cannot connect to daemon at {}", paths.socket.display()))?;
    #[cfg(windows)]
    let stream = loop {
        match ClientOptions::new().open(&paths.socket) {
            Ok(stream) => break stream,
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("cannot connect to daemon at {}", paths.socket.display())
                });
            }
        }
    };
    request_on_stream(request, audit, stream).await
}

pub(super) fn client_audit_context(
    paths: &GlobalPaths,
    request: &Request,
    source: AuditSource,
) -> AuditContext {
    let mut audit = AuditContext::new(source, AuditTransport::Ipc).with_request_defaults(request);
    if let Ok(settings) = StateStore::open(&paths.root).and_then(|store| store.node_settings()) {
        audit.origin_node_id = Some(settings.node_id);
    }
    audit
}

async fn request_on_stream<S>(request: &Request, audit: AuditContext, stream: S) -> Result<Response>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let envelope = Envelope::new(request.clone(), audit);
    let mut payload = serde_json::to_vec(&envelope)?;
    payload.push(b'\n');
    writer.write_all(&payload).await?;
    let mut lines = BufReader::new(reader).lines();
    let line = lines
        .next_line()
        .await?
        .context("daemon closed connection without a response")?;
    serde_json::from_str(&line).context("invalid daemon response")
}

pub async fn is_running() -> bool {
    matches!(request(&Request::Ping).await, Ok(response) if response.ok)
}

pub fn open_daemon_log() -> Result<File> {
    let paths = GlobalPaths::discover()?;
    paths.prepare()?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.log)
        .context("failed to open daemon log")
}

pub fn socket_path() -> Result<PathBuf> {
    Ok(GlobalPaths::discover()?.socket)
}

pub fn root_path() -> Result<PathBuf> {
    Ok(GlobalPaths::discover()?.root)
}

pub fn configured_settings() -> Result<crate::state::PublicNodeSettings> {
    let paths = GlobalPaths::discover()?;
    Ok(StateStore::open(&paths.root)?.node_settings()?.public())
}
