//! Audit record persistence helpers for dispatch.

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

pub fn record_audit_value(
    state: &DaemonState,
    context: AuditContext,
    audit_id: Option<String>,
    request_kind: &str,
    operation: &str,
    session: Option<&str>,
    task: Option<&str>,
    status: AuditStatus,
    started_at_ms: u64,
    duration_ms: u64,
    request: serde_json::Value,
    response: serde_json::Value,
    details: serde_json::Value,
    executor_node_id: Option<String>,
) -> Result<AuditRecord> {
    let settings = state.store.node_settings()?;
    let mut context = context;
    if context.origin_node_id.is_none() {
        context.origin_node_id = Some(settings.node_id.clone());
    }
    let success = matches!(status, AuditStatus::Success | AuditStatus::Started);
    let error = (!success)
        .then(|| {
            response
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("request failed")
        })
        .map(truncate_error_summary);
    let replicated_at_ms = if settings.role == NodeRole::Worker {
        None
    } else {
        Some(current_timestamp_ms())
    };
    state.store.record_audit(AuditRecord {
        audit_id: audit_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        correlation_id: context.correlation_id,
        timestamp_ms: started_at_ms,
        duration_ms,
        source: context.source,
        transport: context.transport,
        origin_node_id: context.origin_node_id,
        executor_node_id: executor_node_id.or(Some(settings.node_id)),
        request_kind: request_kind.to_string(),
        operation: operation.to_string(),
        session: session.map(str::to_string).or(context.session),
        task: task.map(str::to_string).or(context.task),
        status,
        success,
        error,
        request,
        response,
        details,
        replicated_at_ms,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn record_request_audit(
    state: &DaemonState,
    request: &Request,
    context: AuditContext,
    audit_id: Option<String>,
    started_at_ms: u64,
    duration_ms: u64,
    status: AuditStatus,
    executor_node_id: Option<String>,
    response: &Response,
    mut details: serde_json::Value,
) {
    if let Some(origin_audit_id) = context.origin_audit_id.as_deref() {
        if let Some(object) = details.as_object_mut() {
            object.insert(
                "origin_audit_id".to_string(),
                serde_json::Value::String(origin_audit_id.to_string()),
            );
        }
    }
    let request_value = serde_json::to_value(request).unwrap_or_else(|error| {
        serde_json::json!({"serialization_error": error.to_string(), "kind": request.kind()})
    });
    let response_value = serde_json::to_value(response).unwrap_or_else(
        |error| serde_json::json!({"serialization_error": error.to_string(), "ok": response.ok}),
    );
    if let Err(error) = record_audit_value(
        state,
        context,
        audit_id,
        request.kind(),
        &request.operation(),
        request.session(),
        request.task(),
        status,
        started_at_ms,
        duration_ms,
        request_value,
        response_value,
        details,
        executor_node_id,
    ) {
        eprintln!("failed to persist audit record: {error:#}");
    }
}

pub(super) fn truncate_error_summary(message: &str) -> String {
    const LIMIT: usize = 512;
    if message.len() <= LIMIT {
        message.to_string()
    } else {
        format!("{}...", message.chars().take(LIMIT).collect::<String>())
    }
}
