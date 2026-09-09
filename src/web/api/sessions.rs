//! Session/task/log/metric/config/action/event endpoints.

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
use super::nodes::*;
use super::notifications::*;
use super::quotas::*;
use super::scaling::*;
use super::tokens::*;
use super::workflow_groups::*;
use super::workflow_runs::*;
use super::workspaces::*;
pub(crate) async fn list_sessions(

    State(state): State<DaemonState>,

    Query(query): Query<HashMap<String, String>>,

) -> Json<Response> {

    let node = match selected_node(&state, &query) {

        Ok(node) => node,

        Err(response) => return Json(response),

    };

    let request = RemoteRequest::ListSessions;

    Json(

        state

            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))

            .await,

    )

}



pub(crate) async fn session_snapshot(

    State(state): State<DaemonState>,

    Path(session): Path<String>,

    Query(query): Query<HashMap<String, String>>,

) -> Json<Response> {

    let node = match selected_node(&state, &query) {

        Ok(node) => node,

        Err(response) => return Json(response),

    };

    let tail = query.get("tail").and_then(|value| value.parse().ok());

    let request = RemoteRequest::Snapshot { session, tail };

    Json(

        state

            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))

            .await,

    )

}



pub(crate) async fn task_logs(

    State(state): State<DaemonState>,

    Path((session, task)): Path<(String, String)>,

    Query(query): Query<HashMap<String, String>>,

) -> Json<Response> {

    let node = match selected_node(&state, &query) {

        Ok(node) => node,

        Err(response) => return Json(response),

    };

    let after = match query.get("after") {

        Some(value) => match value.parse::<u64>() {

            Ok(value) => Some(value),

            Err(_) => {

                return Json(Response::error_with_data(

                    "invalid log cursor",

                    json!({"kind": "validation_error", "status": 400}),

                ));

            }

        },

        None => None,

    };

    let limit = match query.get("limit") {

        Some(value) => match value.parse::<usize>() {

            Ok(value) if value > 0 => value.clamp(1, 5_000),

            _ => {

                return Json(Response::error_with_data(

                    "invalid log limit",

                    json!({"kind": "validation_error", "status": 400}),

                ));

            }

        },

        None => 1_000,

    };

    let request = RemoteRequest::TaskLogs {

        session,

        task,

        after,

        limit,

    };

    Json(

        state

            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))

            .await,

    )

}



pub(crate) fn parse_metrics_window_seconds(

    query: &HashMap<String, String>,

) -> std::result::Result<usize, Response> {

    match query.get("window") {

        None => Ok(600),

        Some(value) => value

            .parse::<usize>()

            .map(|window| window.clamp(1, 600))

            .map_err(|_| {

                Response::error_with_data(

                    "invalid metrics window",

                    json!({

                        "kind": "validation_error",

                        "status": 400,

                    }),

                )

            }),

    }

}



pub(crate) async fn task_metrics(

    State(state): State<DaemonState>,

    Path((session, task)): Path<(String, String)>,

    Query(query): Query<HashMap<String, String>>,

) -> Json<Response> {

    let node = match selected_node(&state, &query) {

        Ok(node) => node,

        Err(response) => return Json(response),

    };

    let window_seconds = match parse_metrics_window_seconds(&query) {

        Ok(window_seconds) => window_seconds,

        Err(response) => return Json(response),

    };

    let request = RemoteRequest::TaskMetrics {

        session,

        task,

        window_seconds,

    };

    Json(

        state

            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))

            .await,

    )

}



pub(crate) async fn clear_task_history(

    State(state): State<DaemonState>,

    Path((session, task)): Path<(String, String)>,

    Query(query): Query<HashMap<String, String>>,

) -> Json<Response> {

    let node = match selected_node(&state, &query) {

        Ok(node) => node,

        Err(response) => return Json(response),

    };

    let request = RemoteRequest::ClearTaskHistory { session, task };

    Json(

        state

            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))

            .await,

    )

}



pub(crate) async fn session_config(

    State(state): State<DaemonState>,

    Path(session): Path<String>,

    Query(query): Query<HashMap<String, String>>,

) -> Json<Response> {

    let node = match selected_node(&state, &query) {

        Ok(node) => node,

        Err(response) => return Json(response),

    };

    let request = RemoteRequest::GetSessionConfig { session };

    Json(

        state

            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))

            .await,

    )

}



#[derive(Deserialize)]

pub(crate) struct UpdateSessionConfigBody {

    revision: String,

    #[serde(default)]

    workspace_env: Option<std::collections::BTreeMap<String, String>>,

    tasks: Vec<EditableTaskInput>,

}



pub(crate) async fn update_session_config(

    State(state): State<DaemonState>,

    Path(session): Path<String>,

    Query(query): Query<HashMap<String, String>>,

    Json(body): Json<UpdateSessionConfigBody>,

) -> Json<Response> {

    let node = match selected_node(&state, &query) {

        Ok(node) => node,

        Err(response) => return Json(response),

    };

    let request = RemoteRequest::PutSessionConfig {

        session,

        revision: body.revision,

        workspace_env: body.workspace_env,

        tasks: body.tasks,

    };

    Json(

        state

            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))

            .await,

    )

}



#[derive(Deserialize)]

pub(crate) struct ActionBody {

    node: Option<String>,

    session: String,

    task: Option<String>,

    action: Action,

}



pub(crate) async fn action(State(state): State<DaemonState>, Json(body): Json<ActionBody>) -> Json<Response> {

    let node = match body.node {

        Some(node) if !node.trim().is_empty() => node,

        _ if state.public_settings().role == crate::state::NodeRole::Worker => "self".to_string(),

        _ => {

            return Json(Response::error_with_data(

                "node is required for leader actions",

                json!({"kind": "validation_error", "status": 400}),

            ));

        }

    };

    let request = RemoteRequest::Action {

        session: body.session,

        task: body.task,

        action: body.action,

    };

    Json(

        state

            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))

            .await,

    )

}

pub(crate) async fn list_task_runs(

    State(state): State<DaemonState>,

    Query(query): Query<HashMap<String, String>>,

) -> Json<Response> {

    let filter = match TaskRunFilter::parse(&query) {

        Ok(v) => v,

        Err(response) => return Json(response),

    };

    let node = match selected_node(&state, &query) {

        Ok(node) => node,

        Err(response) => return Json(response),

    };

    let request = RemoteRequest::ListTaskRuns { filter };

    Json(

        state

            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))

            .await,

    )

}

pub(crate) async fn list_events_route(

    State(state): State<DaemonState>,

    Query(query): Query<HashMap<String, String>>,

) -> Json<Response> {

    let filter = match EventFilter::parse(&query) {

        Ok(v) => v,

        Err(response) => return Json(response),

    };

    match state.store.list_events(&filter) {

        Ok(page) => Json(Response::ok("events", page)),

        Err(error) => Json(Response::error(format!("{error:#}"))),

    }

}
