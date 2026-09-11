//! Node metrics view and scaling-policy endpoints.

use std::collections::{HashMap, HashSet};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use axum::body::Body;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::rejection::JsonRejection;
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
use super::sessions::*;
use super::tokens::*;
use super::workflow_groups::*;
use super::workflow_groups::{ordered_task_labels, workflow_context, workflow_targets};
use super::workflow_runs::*;
use super::workspaces::*;
pub(crate) fn task_status_key(status: &crate::protocol::TaskStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{status:?}"))
}

pub(crate) async fn node_metrics(State(state): State<DaemonState>) -> Json<Response> {
    let nodes = state.node_summaries();

    let mut entries = Vec::new();

    let mut totals: std::collections::BTreeMap<String, u32> = Default::default();

    for node in nodes {
        let inventory = if node.id == "self" {
            state.local_inventory()
        } else {
            state.cluster.cached_inventory(&node.id).unwrap_or_default()
        };

        let session_count = inventory.len();

        let mut status_counts: std::collections::BTreeMap<String, u32> = Default::default();

        for session in &inventory {
            for task in session.tasks.values() {
                *status_counts
                    .entry(task_status_key(&task.status))
                    .or_insert(0) += 1;
            }
        }

        for (status, count) in &status_counts {
            *totals.entry(status.clone()).or_insert(0) += count;
        }

        let metrics_node_id = if node.id == "self" {
            state.public_settings().node_id.clone()
        } else {
            node.id.clone()
        };

        let samples = state
            .node_metrics
            .window(&metrics_node_id, crate::daemon::MAX_NODE_METRIC_SAMPLES);

        entries.push(crate::protocol::NodeMetricsEntryView {
            node_id: node.id.clone(),

            node_name: Some(node.name.clone()),

            online: node.online,

            is_self: node.is_self,

            current: samples.last().cloned(),

            samples,

            session_count,

            task_status_counts: status_counts,
        });
    }

    // Pure masters have no self executor, so node_summaries() omits this node;

    // the dashboard should still show the leader's own CPU/memory samples.

    let self_node_id = state.public_settings().node_id;

    if !entries.iter().any(|entry| entry.is_self) {
        let samples = state
            .node_metrics
            .window(&self_node_id, crate::daemon::MAX_NODE_METRIC_SAMPLES);

        entries.push(crate::protocol::NodeMetricsEntryView {
            node_id: "self".to_string(),

            node_name: Some(state.public_settings().name),

            online: true,

            is_self: true,

            current: samples.last().cloned(),

            samples,

            session_count: 0,

            task_status_counts: Default::default(),
        });
    }

    Json(Response::ok(
        "node metrics",
        NodeMetricsView {
            nodes: entries,

            task_status_counts: totals,
        },
    ))
}

// ---------------------------------------------------------------------------

// Auto-scaling policies

// ---------------------------------------------------------------------------

pub(crate) async fn list_scaling_policies(State(state): State<DaemonState>) -> Json<Response> {
    Json(match state.store.scaling_policies() {
        Ok(policies) => {
            let (nodes, inventories) = workflow_context(&state);

            let targets = workflow_targets(&nodes, &inventories);

            Response::ok(
                "scaling policies",
                ScalingPoliciesView { policies, targets },
            )
        }

        Err(error) => Response::error(format!("{error:#}")),
    })
}

pub(crate) async fn create_scaling_policy(
    State(state): State<DaemonState>,

    input: std::result::Result<Json<ScalingPolicyInput>, JsonRejection>,
) -> Json<Response> {
    let Json(input) = match input {
        Ok(input) => input,
        Err(error) => return Json(Response::error(format!("Invalid scaling policy: {}", error.body_text()))),
    };
    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = serde_json::to_value(&input).unwrap_or_else(|_| json!({}));

    let response = match state.store.create_scaling_policy(input) {
        Ok(policy) => Response::ok("scaling policy created", policy),

        Err(error) => Response::error(format!("{error:#}")),
    };

    record_feature_http_audit(
        &state,
        "scaling_policy",
        "scaling_policy_create",
        "policy_id",
        None,
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );

    Json(response)
}

pub(crate) async fn update_scaling_policy(
    State(state): State<DaemonState>,

    Path(policy): Path<String>,

    input: std::result::Result<Json<ScalingPolicyInput>, JsonRejection>,
) -> Json<Response> {
    let Json(input) = match input {
        Ok(input) => input,
        Err(error) => return Json(Response::error(format!("Invalid scaling policy: {}", error.body_text()))),
    };
    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = serde_json::to_value(&input).unwrap_or_else(|_| json!({}));

    let response = match state.store.update_scaling_policy(&policy, input) {
        Ok(policy) => Response::ok("scaling policy updated", policy),

        Err(error) => Response::error(format!("{error:#}")),
    };

    record_feature_http_audit(
        &state,
        "scaling_policy",
        "scaling_policy_update",
        "policy_id",
        Some(&policy),
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );

    Json(response)
}

pub(crate) async fn delete_scaling_policy(
    State(state): State<DaemonState>,

    Path(policy): Path<String>,
) -> Json<Response> {
    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = json!({"id": policy});

    let response = match state.store.delete_scaling_policy(&policy) {
        Ok(true) => Response::empty("scaling policy deleted"),

        Ok(false) => Response::error(format!("scaling policy '{policy}' not found")),

        Err(error) => Response::error(format!("{error:#}")),
    };

    record_feature_http_audit(
        &state,
        "scaling_policy",
        "scaling_policy_delete",
        "policy_id",
        Some(&policy),
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );

    Json(response)
}
