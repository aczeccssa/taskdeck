//! Workflow revisions, ordered run execution and summaries.

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
use super::workspaces::*;
pub(crate) async fn list_workflow_revisions(
    State(state): State<DaemonState>,

    Path(group): Path<String>,
) -> Json<Response> {
    if let Err(response) = require_workflow_leader(&state) {
        return Json(response);
    }

    Json(match state.store.workflow_group(&group) {
        Ok(Some(group)) => match state.store.workflow_revisions(&group.id) {
            Ok(revisions) => Response::ok(
                "workflow revisions",
                WorkflowRevisionsView {
                    group_id: group.id,

                    group_name: group.name,

                    revisions,
                },
            ),

            Err(error) => Response::error(format!("{error:#}")),
        },

        Ok(None) => Response::error(format!("workflow group '{group}' not found")),

        Err(error) => Response::error(format!("{error:#}")),
    })
}

pub(crate) async fn restore_workflow_revision(
    State(state): State<DaemonState>,

    Path((group, revision)): Path<(String, u64)>,
) -> Json<Response> {
    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = json!({"group_id": group, "revision": revision});

    let response = match require_workflow_leader(&state) {
        Ok(()) => match state.store.workflow_revisions(&group) {
            Ok(revisions) => match revisions.into_iter().find(|item| item.revision == revision) {
                Some(snapshot) => {
                    let input = WorkflowGroupInput {
                        name: snapshot.name.clone(),

                        members: snapshot.members.clone(),

                        graph: snapshot.graph.clone(),
                    };

                    let scoped = match validate_workflow_group_scope(&state, &input) {
                        Err(response) => Err(response),

                        Ok(()) => state
                            .store
                            .update_workflow_group(
                                &group,
                                input,
                                Some(&format!("restored from revision {revision}")),
                            )
                            .map_err(|error| Response::error(format!("{error:#}"))),
                    };

                    match scoped {
                        Ok(updated) => Response::ok(
                            "workflow group restored",
                            workflow_group_view(&state, updated),
                        ),

                        Err(response) => response,
                    }
                }

                None => Response::error(format!(
                    "revision {revision} of workflow group '{group}' not found"
                )),
            },

            Err(error) => Response::error(format!("{error:#}")),
        },

        Err(response) => response,
    };

    record_feature_http_audit(
        &state,
        "workflow_group",
        "workflow_group_restore_revision",
        "group_id",
        Some(&group),
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );

    Json(response)
}

#[derive(Deserialize)]

pub(crate) struct WorkflowRunBody {
    #[serde(default = "default_stop_on_failure")]
    stop_on_failure: bool,
}

pub(crate) fn default_stop_on_failure() -> bool {
    true
}

pub(crate) async fn run_workflow_group(
    State(state): State<DaemonState>,

    Path(group): Path<String>,

    body: Option<Json<WorkflowRunBody>>,
) -> Json<Response> {
    let started_at_ms = current_millis();

    let started = Instant::now();

    let stop_on_failure = body.map(|Json(body)| body.stop_on_failure).unwrap_or(true);

    let request = json!({"group_id": group, "stop_on_failure": stop_on_failure});

    let response = match require_workflow_leader(&state) {
        Ok(()) => match state.store.workflow_group(&group) {
            Ok(Some(group)) => match workflow_run_order(&group) {
                Ok(order) => {
                    let summary =
                        run_workflow_group_ordered(state.clone(), group, order, stop_on_failure)
                            .await;

                    Response::ok("workflow run completed", summary)
                }

                Err(error) => Response::error(error),
            },

            Ok(None) => Response::error(format!("workflow group '{group}' not found")),

            Err(error) => Response::error(format!("{error:#}")),
        },

        Err(response) => response,
    };

    record_feature_http_audit(
        &state,
        "workflow_group",
        "workflow_group_run",
        "group_id",
        Some(&group),
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );

    Json(response)
}

/// Topological member order following graph edges; ties break by member position.

pub(crate) fn workflow_run_order(group: &WorkflowGroup) -> std::result::Result<Vec<usize>, String> {
    let member_count = group.members.len();

    let mut indegree = vec![0usize; member_count];

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); member_count];

    for edge in &group.graph.edges {
        if edge.from >= member_count || edge.to >= member_count {
            return Err("workflow graph edge references a member that does not exist".to_string());
        }

        indegree[edge.to] += 1;

        children[edge.from].push(edge.to);
    }

    let mut ready: std::collections::BTreeSet<usize> = (0..member_count)
        .filter(|index| indegree[*index] == 0)
        .collect();

    let mut order = Vec::with_capacity(member_count);

    while let Some(index) = ready.pop_first() {
        order.push(index);

        for child in &children[index] {
            indegree[*child] -= 1;

            if indegree[*child] == 0 {
                ready.insert(*child);
            }
        }
    }

    if order.len() != member_count {
        return Err(
            "workflow graph contains a cycle; fix the orchestration edges before running"
                .to_string(),
        );
    }

    Ok(order)
}

pub(crate) async fn run_workflow_group_ordered(
    state: DaemonState,

    group: WorkflowGroup,

    order: Vec<usize>,

    stop_on_failure: bool,
) -> WorkflowGroupActionSummary {
    let view = workflow_group_view(&state, group.clone());

    let mut results = Vec::new();

    for index in order {
        let Some(member) = view.members.get(index) else {
            continue;
        };

        if !member.available {
            results.push(WorkflowGroupActionItem {
                node_id: member.member.node_id.clone(),

                node_name: member.node_name.clone(),

                session: member.member.session.clone(),

                workspace_display_name: member.workspace_display_name.clone(),

                task: member.member.task.clone(),

                status: WorkflowGroupActionItemStatus::Skipped,

                message: member
                    .skip_reason
                    .clone()
                    .unwrap_or_else(|| "skipped".to_string()),
            });

            if stop_on_failure {
                break;
            }

            continue;
        }

        let request = RemoteRequest::Action {
            session: member.member.session.clone(),

            task: Some(member.member.task.clone()),

            action: Action::Start,
        };

        let response = state
            .dispatch_node_with_audit(
                &member.member.node_id,
                request.clone(),
                web_audit_context(&state, &request),
            )
            .await;

        let succeeded = response.ok;

        results.push(WorkflowGroupActionItem {
            node_id: member.member.node_id.clone(),

            node_name: member.node_name.clone(),

            session: member.member.session.clone(),

            workspace_display_name: member.workspace_display_name.clone(),

            task: member.member.task.clone(),

            status: if succeeded {
                WorkflowGroupActionItemStatus::Success
            } else {
                WorkflowGroupActionItemStatus::Failed
            },

            message: response.message,
        });

        if !succeeded && stop_on_failure {
            break;
        }
    }

    summarize_workflow_results(group, results)
}

pub(crate) fn summarize_workflow_results(
    group: WorkflowGroup,

    results: Vec<WorkflowGroupActionItem>,
) -> WorkflowGroupActionSummary {
    let success_count = results
        .iter()
        .filter(|item| item.status == WorkflowGroupActionItemStatus::Success)
        .count();

    let failed_count = results
        .iter()
        .filter(|item| item.status == WorkflowGroupActionItemStatus::Failed)
        .count();

    let skipped_count = results
        .iter()
        .filter(|item| item.status == WorkflowGroupActionItemStatus::Skipped)
        .count();

    WorkflowGroupActionSummary {
        group_id: group.id,

        group_name: group.name,

        action: Action::Start,

        results,

        success_count,

        failed_count,

        skipped_count,
    }
}

// ---------------------------------------------------------------------------

// Resource quotas

// ---------------------------------------------------------------------------
