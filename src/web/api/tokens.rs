//! API token endpoints.

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
use super::workflow_groups::*;
use super::workflow_runs::*;
use super::workspaces::*;
pub(crate) async fn list_api_tokens(State(state): State<DaemonState>) -> Json<Response> {
    Json(match state.store.api_tokens() {
        Ok(tokens) => Response::ok("api tokens", ApiTokensView { tokens }),

        Err(error) => Response::error(format!("{error:#}")),
    })
}

pub(crate) async fn create_api_token(
    State(state): State<DaemonState>,

    Json(input): Json<ApiTokenInput>,
) -> Json<Response> {
    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = serde_json::to_value(&input).unwrap_or_else(|_| json!({}));

    let response = match state.store.create_api_token(&input.name) {
        Ok(created) => Response::ok("api token created", created),

        Err(error) => Response::error(format!("{error:#}")),
    };

    record_feature_http_audit(
        &state,
        "api_token",
        "api_token_create",
        "token_id",
        None,
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );

    Json(response)
}

pub(crate) async fn revoke_api_token(
    State(state): State<DaemonState>,

    Path(token): Path<String>,
) -> Json<Response> {
    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = json!({"id": token});

    let response = match state.store.revoke_api_token(&token) {
        Ok(true) => Response::empty("api token revoked"),

        Ok(false) => Response::error(format!("api token '{token}' not found")),

        Err(error) => Response::error(format!("{error:#}")),
    };

    record_feature_http_audit(
        &state,
        "api_token",
        "api_token_revoke",
        "token_id",
        Some(&token),
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );

    Json(response)
}

// ---------------------------------------------------------------------------

// Board templates

// ---------------------------------------------------------------------------
