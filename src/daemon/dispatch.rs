//! Request dispatch: local execution, remote forwarding, config writes.

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

impl DaemonState {
    pub async fn dispatch_node(&self, node: &str, request: RemoteRequest) -> Response {

        self.dispatch_node_with_audit(node, request, AuditContext::internal())

            .await

    }



    pub async fn dispatch_node_with_audit(

        &self,

        node: &str,

        request: RemoteRequest,

        audit: AuditContext,

    ) -> Response {

        let settings = self.settings.lock().expect("node settings lock").clone();

        let local_request = request.clone().into_local();

        let audit = audit.with_request_defaults(&local_request);

        if node == "self" {

            if !settings.execution_enabled() {

                let response = Response::error("pure master does not have a self executor");

                record_request_audit(

                    self,

                    &local_request,

                    audit,

                    None,

                    current_timestamp_ms(),

                    0,

                    AuditStatus::Error,

                    Some(settings.node_id),

                    &response,

                    serde_json::json!({"node": "self"}),

                );

                return response;

            }

            return dispatch_async_with_audit(self.clone(), local_request, Some(audit)).await;

        }

        if settings.role != NodeRole::Leader {

            let response =

                Response::error("worker nodes can only control their local self executor");

            record_request_audit(

                self,

                &local_request,

                audit,

                None,

                current_timestamp_ms(),

                0,

                AuditStatus::Error,

                Some(settings.node_id),

                &response,

                serde_json::json!({"node": node}),

            );

            return response;

        }



        let origin_audit_id = uuid::Uuid::new_v4().to_string();

        let mut worker_audit = audit.clone();

        if worker_audit.origin_node_id.is_none() {

            worker_audit.origin_node_id = Some(settings.node_id.clone());

        }

        worker_audit.origin_audit_id = Some(origin_audit_id.clone());

        worker_audit.transport = AuditTransport::Agent;



        let started_at_ms = current_timestamp_ms();

        let started = Instant::now();

        let (response, status) = self

            .cluster

            .request_with_audit(node, request, Some(worker_audit))

            .await;

        record_request_audit(

            self,

            &local_request,

            audit,

            Some(origin_audit_id),

            started_at_ms,

            started.elapsed().as_millis() as u64,

            status,

            Some(node.to_string()),

            &response,

            serde_json::json!({"node": node, "remote_transport": "agent"}),

        );

        response

    }


}

pub async fn dispatch_async(state: DaemonState, request: Request) -> Response {
    dispatch_async_with_audit(state, request, None).await
}

pub async fn dispatch_async_with_audit(
    state: DaemonState,
    request: Request,
    audit: Option<AuditContext>,
) -> Response {
    match tokio::task::spawn_blocking(move || dispatch_with_audit(&state, request, audit)).await {
        Ok(response) => response,
        Err(error) => Response::error(format!("request worker failed: {error}")),
    }
}

#[cfg(test)]
pub(super) fn dispatch(state: &DaemonState, request: Request) -> Response {
    dispatch_with_audit(state, request, None)
}

pub(super) fn dispatch_with_audit(
    state: &DaemonState,
    request: Request,
    audit: Option<AuditContext>,
) -> Response {
    let context = audit
        .unwrap_or_else(AuditContext::internal)
        .with_request_defaults(&request);
    let started_at_ms = current_timestamp_ms();
    let started = Instant::now();
    let result = handle(state, request.clone());
    let response = match result {
        Ok(response) => response,
        Err(error) => Response::error(format!("{error:#}")),
    };
    let status = AuditStatus::from_ok(response.ok);
    let executor_node_id = state
        .store
        .node_settings()
        .ok()
        .map(|settings| settings.node_id);
    record_request_audit(
        state,
        &request,
        context,
        None,
        started_at_ms,
        started.elapsed().as_millis() as u64,
        status,
        executor_node_id,
        &response,
        serde_json::json!({}),
    );
    response
}
pub(super) fn prepare_session_config_write(
    state: &DaemonState,
    project: &std::path::Path,
    revision: &str,
    workspace_env: Option<&std::collections::BTreeMap<String, String>>,
    tasks: Vec<crate::protocol::EditableTaskInput>,
) -> std::result::Result<config::PreparedSessionConfigWrite, config::WriteConfigError> {
    #[cfg(not(test))]
    let _ = state;

    #[cfg(test)]
    {
        config::prepare_session_config_write_for_test(
            project,
            revision,
            tasks,
            workspace_env,
            state.put_config_post_check_delay(),
        )
    }

    #[cfg(not(test))]
    {
        config::prepare_session_config_write(project, revision, tasks, workspace_env)
    }
}

pub(super) fn config_write_error_response(error: config::WriteConfigError) -> Result<Response> {
    match error {
        config::WriteConfigError::StaleRevision { current_revision } => {
            Ok(Response::error_with_data(
                "stale config revision",
                serde_json::json!({
                    "kind": "stale_revision",
                    "status": 409,
                    "current_revision": current_revision,
                }),
            ))
        }
        config::WriteConfigError::Validation { message } => Ok(Response::error_with_data(
            &message,
            serde_json::json!({
                "kind": "validation_error",
                "status": 400,
            }),
        )),
        config::WriteConfigError::Other(error) => Err(error),
    }
}
pub(super) fn require_local_execution(state: &DaemonState) -> Result<()> {
    if state.execution_enabled() {
        Ok(())
    } else {
        bail!("this node is a pure master and does not provide local task execution")
    }
}

pub(super) fn reject_unavailable_session(state: &DaemonState, session: &str) -> Result<()> {
    if let Some(unavailable) = state
        .unavailable_sessions
        .lock()
        .expect("unavailable sessions lock")
        .get(session)
    {
        bail!(
            "session '{}' is unavailable from {}: {}",
            unavailable.session,
            unavailable.project.display(),
            unavailable.error
        );
    }
    Ok(())
}
