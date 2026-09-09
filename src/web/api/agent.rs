//! Agent WebSocket upgrade endpoint.

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
use super::workflow_runs::*;
use super::workspaces::*;
pub(crate) async fn agent_connect(

    State(state): State<DaemonState>,

    upgrade: WebSocketUpgrade,

) -> AxumResponse {

    if !state

        .public_settings()

        .role

        .eq(&crate::state::NodeRole::Leader)

    {

        return StatusCode::FORBIDDEN.into_response();

    }

    upgrade

        .on_upgrade(move |socket| cluster::serve_agent_socket(state.cluster.clone(), socket))

        .into_response()

}
