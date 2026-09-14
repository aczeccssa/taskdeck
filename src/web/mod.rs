//! Web server: route assembly, health and small utilities.

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

mod api;
mod assets;
mod auth;
mod mcp_server;

use api::agent::*;
use api::audit::*;
use api::board_templates::*;
use api::boards::*;
use api::dependencies::*;
use api::mcp_calls::*;
use api::nodes::*;
use api::notifications::*;
use api::quotas::*;
use api::scaling::*;
use api::sessions::*;
use api::tokens::*;
use api::workflow_groups::*;
use api::workflow_runs::*;
use api::workspaces::*;
use assets::*;
use auth::*;
use mcp_server::*;

#[cfg(test)]
mod tests;
pub async fn serve(state: DaemonState, listener: tokio::net::TcpListener) -> Result<()> {
    axum::serve(listener, app(state))
        .await
        .context("Web server failed")
}

pub(crate) fn app(state: DaemonState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/dashboard", get(index))
        .route("/workflows", get(index))
        .route("/boards", get(index))
        .route("/alerts", get(index))
        .route("/calls", get(index))
        .route("/audit", get(index))
        .route("/docs", get(index))
        .route("/settings", get(index))
        .route("/healthz", get(health))
        .route("/api/agent/connect", get(agent_connect))
        .route("/api/nodes", get(list_nodes))
        .route(
            "/api/nodes/{node}/settings",
            get(node_settings).put(update_node_settings),
        )
        .route("/api/nodes/{node}", delete(delete_node))
        .route(
            "/api/nodes/self/service",
            get(node_service).post(node_service_action),
        )
        .route("/api/workspaces", get(list_workspaces))
        .route("/api/workspaces/{session}", delete(remove_workspace))
        .route(
            "/api/workspaces/{session}/alias",
            put(update_workspace_alias),
        )
        .route(
            "/api/workflow-groups",
            get(list_workflow_groups).post(create_workflow_group),
        )
        .route(
            "/api/workflow-groups/{group}",
            get(get_workflow_group)
                .put(update_workflow_group)
                .delete(delete_workflow_group),
        )
        .route(
            "/api/workflow-groups/{group}/actions",
            post(workflow_group_action),
        )
        .route(
            "/api/workflow-groups/{group}/revisions",
            get(list_workflow_revisions),
        )
        .route(
            "/api/workflow-groups/{group}/revisions/{revision}/restore",
            post(restore_workflow_revision),
        )
        .route("/api/workflow-groups/{group}/run", post(run_workflow_group))
        .route("/api/quotas", get(list_quotas).post(create_quota))
        .route(
            "/api/quotas/{quota}",
            put(update_quota).delete(delete_quota),
        )
        .route("/api/notifications", get(list_notifications))
        .route("/api/notifications/read", post(mark_notifications_read))
        .route(
            "/api/notification-rules",
            get(list_notification_rules).post(create_notification_rule),
        )
        .route(
            "/api/notification-rules/{rule}",
            put(update_notification_rule).delete(delete_notification_rule),
        )
        .route("/api/tokens", get(list_api_tokens).post(create_api_token))
        .route("/api/tokens/{token}", delete(revoke_api_token))
        .route(
            "/api/board-templates",
            get(list_board_templates).post(create_board_template),
        )
        .route("/api/board-templates/import", post(import_board_template))
        .route(
            "/api/board-templates/{template}",
            delete(delete_board_template),
        )
        .route(
            "/api/board-templates/{template}/apply",
            post(apply_board_template),
        )
        .route(
            "/api/board-templates/{template}/export",
            get(export_board_template),
        )
        .route(
            "/api/dependencies",
            get(list_dependencies).post(create_dependency),
        )
        .route("/api/dependencies/{dependency}", delete(delete_dependency))
        .route("/api/node-metrics", get(node_metrics))
        .route(
            "/api/scaling-policies",
            get(list_scaling_policies).post(create_scaling_policy),
        )
        .route(
            "/api/scaling-policies/{policy}",
            put(update_scaling_policy).delete(delete_scaling_policy),
        )
        .route("/api/boards", get(list_boards).post(create_board))
        .route(
            "/api/boards/{board}",
            get(get_board).put(update_board).delete(delete_board),
        )
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{session}", get(session_snapshot))
        .route("/api/sessions/{session}/tasks/{task}/logs", get(task_logs))
        .route(
            "/api/sessions/{session}/tasks/{task}/metrics",
            get(task_metrics),
        )
        .route(
            "/api/sessions/{session}/tasks/{task}/history",
            delete(clear_task_history),
        )
        .route(
            "/api/sessions/{session}/config",
            get(session_config).put(update_session_config),
        )
        .route("/api/audit", get(list_audit))
        .route("/api/audit/{audit_id}", get(audit_detail))
        .route("/api/mcp-calls", get(list_mcp_calls))
        .route("/api/mcp-calls/{id}", get(mcp_call_detail))
        .route("/api/action", post(action))
        .route("/mcp", post(mcp))
        .route("/api/task-runs", get(list_task_runs))
        .route("/api/events", get(list_events_route))
        .route("/login", get(index).post(login_submit))
        .route("/logout", post(logout))
        .route("/me", get(auth_status))
        .route("/{*asset_path}", get(static_asset))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state)
}

pub(crate) async fn health() -> StatusCode {
    StatusCode::OK
}

pub(crate) fn current_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(crate) fn optional_query_value(query: &HashMap<String, String>, key: &str) -> Option<String> {
    query.get(key).and_then(|value| {
        let trimmed = value.trim();

        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

pub(crate) fn parse_positive_usize(
    query: &HashMap<String, String>,

    key: &str,

    default: usize,
) -> std::result::Result<usize, Response> {
    match optional_query_value(query, key) {
        None => Ok(default),

        Some(value) => value
            .parse::<usize>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                Response::error_with_data(
                    format!("invalid {key}"),
                    json!({

                        "kind": "validation_error",

                        "status": 400,

                    }),
                )
            }),
    }
}
