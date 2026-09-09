//! Notification and notification-rule endpoints.

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
use super::quotas::*;
use super::scaling::*;
use super::sessions::*;
use super::tokens::*;
use super::workflow_groups::*;
use super::workflow_runs::*;
use super::workspaces::*;
pub(crate) async fn list_notifications(

    State(state): State<DaemonState>,

    Query(query): Query<HashMap<String, String>>,

) -> Json<Response> {

    let limit = parse_positive_usize(&query, "limit", 200)

        .unwrap_or(200)

        .min(1000);

    Json(match state.store.notifications(limit) {

        Ok(notifications) => {

            let unread_count = state.store.unread_notification_count().unwrap_or(0);

            Response::ok(

                "notifications",

                NotificationsView {

                    notifications,

                    unread_count,

                },

            )

        }

        Err(error) => Response::error(format!("{error:#}")),

    })

}



pub(crate) async fn mark_notifications_read(

    State(state): State<DaemonState>,

    Json(input): Json<NotificationMarkReadInput>,

) -> Json<Response> {

    let response = match (input.all, input.id) {

        (true, _) | (false, None) => state.store.mark_notifications_read(None).map(|changed| {

            Response::ok(

                format!("marked {changed} notifications read"),

                json!({"changed": changed}),

            )

        }),

        (false, Some(id)) => state

            .store

            .mark_notifications_read(Some(id))

            .map(|changed| {

                Response::ok(

                    format!("marked {changed} notification read"),

                    json!({"changed": changed}),

                )

            }),

    };

    Json(match response {

        Ok(response) => response,

        Err(error) => Response::error(format!("{error:#}")),

    })

}



pub(crate) async fn list_notification_rules(State(state): State<DaemonState>) -> Json<Response> {

    Json(match state.store.notification_rules() {

        Ok(rules) => Response::ok("notification rules", rules),

        Err(error) => Response::error(format!("{error:#}")),

    })

}



pub(crate) async fn create_notification_rule(

    State(state): State<DaemonState>,

    Json(input): Json<NotificationRuleInput>,

) -> Json<Response> {

    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = serde_json::to_value(&input).unwrap_or_else(|_| json!({}));

    let response = match state.store.create_notification_rule(input) {

        Ok(rule) => Response::ok("notification rule created", rule),

        Err(error) => Response::error(format!("{error:#}")),

    };

    record_feature_http_audit(

        &state,

        "notification_rule",

        "notification_rule_create",

        "rule_id",

        None,

        request,

        &response,

        started_at_ms,

        started.elapsed().as_millis() as u64,

    );

    Json(response)

}



pub(crate) async fn update_notification_rule(

    State(state): State<DaemonState>,

    Path(rule): Path<String>,

    Json(input): Json<NotificationRuleInput>,

) -> Json<Response> {

    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = serde_json::to_value(&input).unwrap_or_else(|_| json!({}));

    let response = match state.store.update_notification_rule(&rule, input) {

        Ok(rule) => Response::ok("notification rule updated", rule),

        Err(error) => Response::error(format!("{error:#}")),

    };

    record_feature_http_audit(

        &state,

        "notification_rule",

        "notification_rule_update",

        "rule_id",

        Some(&rule),

        request,

        &response,

        started_at_ms,

        started.elapsed().as_millis() as u64,

    );

    Json(response)

}



pub(crate) async fn delete_notification_rule(

    State(state): State<DaemonState>,

    Path(rule): Path<String>,

) -> Json<Response> {

    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = json!({"id": rule});

    let response = match state.store.delete_notification_rule(&rule) {

        Ok(true) => Response::empty("notification rule deleted"),

        Ok(false) => Response::error(format!("notification rule '{rule}' not found")),

        Err(error) => Response::error(format!("{error:#}")),

    };

    record_feature_http_audit(

        &state,

        "notification_rule",

        "notification_rule_delete",

        "rule_id",

        Some(&rule),

        request,

        &response,

        started_at_ms,

        started.elapsed().as_millis() as u64,

    );

    Json(response)

}



// ---------------------------------------------------------------------------

// API tokens (external integrations)

// ---------------------------------------------------------------------------
