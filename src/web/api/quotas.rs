//! Workspace quota endpoints.

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
use super::scaling::*;
use super::sessions::*;
use super::tokens::*;
use super::workflow_groups::*;
use super::workflow_groups::{ordered_task_labels, workflow_context, workflow_targets};
use super::workflow_runs::*;
use super::workspaces::*;
pub(crate) fn quota_sessions(state: &DaemonState) -> Vec<String> {
    let mut sessions: Vec<String> = state
        .node_summaries()
        .into_iter()
        .flat_map(|node| node.sessions)
        .collect();

    sessions.sort();

    sessions.dedup();

    sessions
}

pub(crate) async fn list_quotas(State(state): State<DaemonState>) -> Json<Response> {
    Json(match state.store.quotas() {
        Ok(quotas) => Response::ok(
            "quotas",
            WorkspaceQuotasView {
                quotas,

                sessions: quota_sessions(&state),
            },
        ),

        Err(error) => Response::error(format!("{error:#}")),
    })
}

pub(crate) async fn create_quota(
    State(state): State<DaemonState>,

    Json(input): Json<WorkspaceQuotaInput>,
) -> Json<Response> {
    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = serde_json::to_value(&input).unwrap_or_else(|_| json!({}));

    let node_id = state.public_settings().node_id;

    let response = match state.store.create_quota(&node_id, input) {
        Ok(quota) => Response::ok("quota created", quota),

        Err(error) => Response::error(format!("{error:#}")),
    };

    record_feature_http_audit(
        &state,
        "quota",
        "quota_create",
        "quota_id",
        None,
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );

    Json(response)
}

pub(crate) async fn update_quota(
    State(state): State<DaemonState>,

    Path(quota): Path<String>,

    Json(input): Json<WorkspaceQuotaInput>,
) -> Json<Response> {
    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = serde_json::to_value(&input).unwrap_or_else(|_| json!({}));

    let response = match state.store.update_quota(&quota, input) {
        Ok(quota) => Response::ok("quota updated", quota),

        Err(error) => Response::error(format!("{error:#}")),
    };

    record_feature_http_audit(
        &state,
        "quota",
        "quota_update",
        "quota_id",
        Some(&quota),
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );

    Json(response)
}

pub(crate) async fn delete_quota(
    State(state): State<DaemonState>,

    Path(quota): Path<String>,
) -> Json<Response> {
    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = json!({"id": quota});

    let response = match state.store.delete_quota(&quota) {
        Ok(true) => Response::empty("quota deleted"),

        Ok(false) => Response::error(format!("quota '{quota}' not found")),

        Err(error) => Response::error(format!("{error:#}")),
    };

    record_feature_http_audit(
        &state,
        "quota",
        "quota_delete",
        "quota_id",
        Some(&quota),
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );

    Json(response)
}

// ---------------------------------------------------------------------------

// Notifications and alert rules

// ---------------------------------------------------------------------------
