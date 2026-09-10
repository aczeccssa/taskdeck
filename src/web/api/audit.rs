//! Audit endpoints and shared audit-context helpers.

use std::collections::{HashMap, HashSet};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use axum::body::Body;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response as AxumResponse};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::cluster::{self, RemoteRequest};
use crate::daemon::{DaemonState, record_audit_value};
use crate::platform_service::{ServiceAction, service_control, service_status};
#[cfg(test)]
use crate::protocol::McpCallListPage;
use crate::protocol::{
    Action, ApiTokenInput, ApiTokensView, AuditContext, AuditFilter, AuditSource, AuditStatus,
    AuditTransport, Board, BoardCardInput, BoardCardView, BoardInput, BoardTemplateApplyInput,
    BoardTemplateExport, BoardTemplateInput, BoardTemplatesView, BoardView, BoardsView,
    EditableTaskInput, EventFilter, McpCallRecord, NodeMetricsView, NodeSummary,
    NotificationMarkReadInput, NotificationRuleInput, NotificationsView, Response,
    ScalingPoliciesView, ScalingPolicyInput, ServiceScope, SessionSnapshot, TaskDependenciesView,
    TaskDependencyInput, TaskRunFilter, WorkflowGroup, WorkflowGroupActionItem,
    WorkflowGroupActionItemStatus, WorkflowGroupActionSummary, WorkflowGroupInput,
    WorkflowGroupMemberView, WorkflowGroupView, WorkflowGroupsView, WorkflowRevisionsView,
    WorkflowTargetView, WorkspaceQuotaInput, WorkspaceQuotasView, casefold_search_text,
};
use crate::state::NodeRole;

use crate::web::current_millis;
use crate::web::optional_query_value;
use crate::web::parse_positive_usize;

use super::agent::*;
use super::board_templates::*;
use super::boards::*;
use super::dependencies::*;
use super::mcp_calls::*;
use super::nodes::*;
use super::notifications::*;
use super::quotas::*;
use super::scaling::*;
use super::sessions::*;
use super::tokens::*;
use super::workflow_groups::*;
use super::workflow_runs::*;
use super::workspaces::*;
pub(crate) fn record_feature_http_audit(
    state: &DaemonState,

    request_kind: &str,

    operation: &str,

    entity_key: &str,

    entity_id: Option<&str>,

    request: serde_json::Value,

    response: &Response,

    started_at_ms: u64,

    duration_ms: u64,
) {
    let response_value = serde_json::to_value(response)
        .unwrap_or_else(|error| json!({"ok": response.ok, "message": format!("{error}")}));

    let status = if response.ok {
        AuditStatus::Success
    } else {
        AuditStatus::Error
    };

    let mut details = json!({});

    if let Some(entity_id) = entity_id {
        details[entity_key] = json!(entity_id);
    }

    let _ = record_audit_value(
        state,
        AuditContext::new(AuditSource::Web, AuditTransport::Http),
        None,
        request_kind,
        operation,
        None,
        None,
        status,
        started_at_ms,
        duration_ms,
        request,
        response_value,
        details,
        Some(state.public_settings().node_id),
    );
}

pub(crate) fn audit_context_for_remote_request(
    state: &DaemonState,

    source: AuditSource,

    transport: AuditTransport,

    request: &RemoteRequest,
) -> AuditContext {
    let local_request = request.clone().into_local();

    AuditContext::new(source, transport)
        .with_request_defaults(&local_request)
        .with_origin_node(state.public_settings().node_id)
}

pub(crate) fn web_audit_context(state: &DaemonState, request: &RemoteRequest) -> AuditContext {
    audit_context_for_remote_request(state, AuditSource::Web, AuditTransport::Http, request)
}

pub(crate) fn mcp_audit_context(state: &DaemonState, request: &RemoteRequest) -> AuditContext {
    audit_context_for_remote_request(state, AuditSource::Mcp, AuditTransport::Mcp, request)
}

#[allow(clippy::too_many_arguments)]

pub(crate) fn record_mcp_direct_audit(
    state: &DaemonState,

    params: &Value,

    operation: &str,

    response: &Response,

    started_at_ms: u64,

    duration_ms: u64,

    node: Option<&str>,

    session: Option<&str>,

    task: Option<&str>,
) {
    let mut context = AuditContext::new(AuditSource::Mcp, AuditTransport::Mcp);

    context.origin_node_id = Some(state.public_settings().node_id);

    let response_value = serde_json::to_value(response).unwrap_or_else(
        |error| json!({"serialization_error": error.to_string(), "ok": response.ok}),
    );

    let _ = record_audit_value(
        state,
        context,
        None,
        "mcp_tools_call",
        operation,
        session,
        task,
        AuditStatus::from_ok(response.ok),
        started_at_ms,
        duration_ms,
        params.clone(),
        response_value,
        json!({"node": node}),
        node.map(str::to_string)
            .or_else(|| Some(state.public_settings().node_id)),
    );
}

pub(crate) async fn list_audit(
    State(state): State<DaemonState>,

    Query(query): Query<HashMap<String, String>>,
) -> Json<Response> {
    let filter = match AuditFilter::parse(&query) {
        Ok(filter) => filter,

        Err(response) => return Json(response),
    };

    match state.store.list_audit(&filter) {
        Ok(page) => Json(Response::ok("audit records", page)),

        Err(error) => Json(Response::error(format!("{error:#}"))),
    }
}

pub(crate) async fn audit_detail(
    State(state): State<DaemonState>,

    Path(audit_id): Path<String>,
) -> Json<Response> {
    match state.store.audit_detail(&audit_id) {
        Ok(Some(record)) => Json(Response::ok("audit record", record)),

        Ok(None) => Json(Response::error_with_data(
            "audit record not found",
            json!({"kind": "not_found", "status": 404}),
        )),

        Err(error) => Json(Response::error(format!("{error:#}"))),
    }
}
