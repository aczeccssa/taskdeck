//! Node listing, settings and service control endpoints.

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
use super::audit::*;
use super::board_templates::*;
use super::boards::*;
use super::dependencies::*;
use super::mcp_calls::*;
use super::notifications::*;
use super::quotas::*;
use super::scaling::*;
use super::sessions::*;
use super::tokens::*;
use super::workflow_groups::*;
use super::workflow_runs::*;
use super::workspaces::*;
use super::audit::{
    audit_context_for_remote_request, mcp_audit_context, record_feature_http_audit,
    record_mcp_direct_audit, web_audit_context,
};
pub(crate) async fn list_nodes(State(state): State<DaemonState>) -> Json<Response> {

    Json(Response::ok("nodes", state.node_summaries()))

}



pub(crate) async fn node_settings(

    State(state): State<DaemonState>,

    Path(node): Path<String>,

    Query(query): Query<HashMap<String, String>>,

) -> Json<Response> {

    dispatch_selected_node(state, &node, &query, RemoteRequest::GetNodeSettings).await

}



#[derive(Debug, Deserialize)]

pub(crate) struct ServiceActionBody {

    action: ServiceAction,

    scope: ServiceScope,

    #[serde(default)]

    home: Option<String>,

}



pub(crate) async fn node_service(

    Query(query): Query<HashMap<String, String>>,

) -> Result<Json<Response>, Json<Response>> {

    let scope = match query.get("scope").map(String::as_str) {

        Some("system") => ServiceScope::System,

        _ => ServiceScope::User,

    };

    let result = tokio::task::spawn_blocking(move || service_status(scope))

        .await

        .map_err(|error| {

            Json(Response::error(format!(

                "service status task failed: {error}"

            )))

        })?;

    match result {

        Ok(status) => Ok(Json(Response::ok("service status", status))),

        Err(error) => Err(Json(Response::error(format!("{error:#}")))),

    }

}



pub(crate) async fn node_service_action(

    State(state): State<DaemonState>,

    Json(body): Json<ServiceActionBody>,

) -> Result<Json<Response>, Json<Response>> {

    let started = std::time::Instant::now();

    let home = body.home.clone().map(std::path::PathBuf::from);

    let home_for_audit = home.clone();

    let scope = body.scope;

    let action = body.action;

    let result = tokio::task::spawn_blocking(move || service_control(scope, action, home))

        .await

        .map_err(|error| Json(Response::error(format!("service task failed: {error}"))))?;

    let response = match result {

        Ok(status) => Response::ok(

            match action {

                ServiceAction::Status => "service status",

                ServiceAction::Install => "service installed",

                ServiceAction::Uninstall => "service uninstalled",

                ServiceAction::Start => "service started",

                ServiceAction::Stop => "service stopped",

            },

            status,

        ),

        Err(error) => Response::error(format!("{error:#}")),

    };

    let response_value = serde_json::to_value(&response).unwrap_or_else(

        |error| serde_json::json!({"ok": response.ok, "message": format!("{error}")}),

    );

    let _ = record_audit_value(

        &state,

        AuditContext::new(AuditSource::Web, AuditTransport::Http),

        None,

        "service_control",

        match action {

            ServiceAction::Status => "status",

            ServiceAction::Install => "install",

            ServiceAction::Uninstall => "uninstall",

            ServiceAction::Start => "start",

            ServiceAction::Stop => "stop",

        },

        None,

        None,

        AuditStatus::from_ok(response.ok),

        current_millis(),

        started.elapsed().as_millis() as u64,

        serde_json::json!({"action": action, "scope": scope, "home": home_for_audit}),

        response_value,

        serde_json::json!({"node":"self"}),

        Some(state.public_settings().node_id),

    );

    if response.ok {

        Ok(Json(response))

    } else {

        Err(Json(response))

    }

}

pub(crate) async fn update_node_settings(

    State(state): State<DaemonState>,

    Path(node): Path<String>,

    Query(query): Query<HashMap<String, String>>,

    Json(patch): Json<crate::protocol::NodeSettingsPatch>,

) -> Json<Response> {

    dispatch_selected_node(

        state,

        &node,

        &query,

        RemoteRequest::PutNodeSettings { patch },

    )

    .await

}



pub(crate) async fn dispatch_selected_node(

    state: DaemonState,

    requested_node: &str,

    query: &HashMap<String, String>,

    request: RemoteRequest,

) -> Json<Response> {

    let inferred = query

        .get("node")

        .filter(|value| !value.trim().is_empty())

        .cloned()

        .unwrap_or_else(|| requested_node.to_string());

    let response = if inferred == "self" {

        let audit = AuditContext::new(AuditSource::Web, AuditTransport::Http)

            .with_request_defaults(&request.clone().into_local())

            .with_origin_node(state.public_settings().node_id);

        crate::daemon::dispatch_async_with_audit(state.clone(), request.into_local(), Some(audit))

            .await

    } else if state.public_settings().role == crate::state::NodeRole::Worker {

        Response::error("worker node settings are local-only")

    } else {

        state

            .dispatch_node_with_audit(

                &inferred,

                request.clone(),

                web_audit_context(&state, &request),

            )

            .await

    };

    Json(response)

}



pub(crate) fn selected_node(

    state: &DaemonState,

    query: &HashMap<String, String>,

) -> std::result::Result<String, Response> {

    if let Some(node) = query.get("node").filter(|node| !node.trim().is_empty()) {

        return Ok(node.clone());

    }

    if state.public_settings().role == crate::state::NodeRole::Worker {

        Ok("self".to_string())

    } else {

        Err(Response::error_with_data(

            "node is required for leader requests",

            json!({"kind": "validation_error", "status": 400}),

        ))

    }

}
