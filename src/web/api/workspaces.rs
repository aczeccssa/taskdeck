//! Workspace listing and alias endpoints.

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
use super::sessions::*;
use super::tokens::*;
use super::workflow_groups::*;
use super::workflow_groups::{ordered_task_labels, workflow_context, workflow_targets};
use super::workflow_runs::*;
pub(crate) async fn list_workspaces(
    State(state): State<DaemonState>,

    Query(query): Query<HashMap<String, String>>,
) -> Json<Response> {
    let node = match selected_node(&state, &query) {
        Ok(node) => node,

        Err(response) => return Json(response),
    };

    let request = RemoteRequest::ListWorkspaces;

    let response = state
        .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))
        .await;

    if response.ok || node == "self" {
        return Json(response);
    }

    if let Some(inventory) = state.cluster.cached_inventory(&node) {
        let summaries = inventory
            .into_iter()
            .map(|session| crate::protocol::WorkspaceSummary {
                display_name: session
                    .alias
                    .clone()
                    .unwrap_or_else(|| session.name.clone()),

                session: session.name,

                alias: session.alias,

                project: session.project,
            })
            .collect::<Vec<_>>();

        return Json(Response::ok("cached workspaces", summaries));
    }

    Json(response)
}

pub(crate) async fn update_workspace_alias(
    State(state): State<DaemonState>,

    Path(session): Path<String>,

    Query(query): Query<HashMap<String, String>>,

    Json(body): Json<Value>,
) -> Json<Response> {
    let node = match selected_node(&state, &query) {
        Ok(node) => node,

        Err(response) => return Json(response),
    };

    let alias = body
        .get("alias")
        .and_then(Value::as_str)
        .map(str::to_string);

    let request = RemoteRequest::SetWorkspaceAlias { session, alias };

    Json(
        state
            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))
            .await,
    )
}
