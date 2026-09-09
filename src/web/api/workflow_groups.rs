//! Workflow group CRUD, scope checks and group action.

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
use super::workflow_runs::*;
use super::workspaces::*;
#[derive(Debug, Deserialize)]
pub(crate) struct WorkflowGroupActionBody {

    action: Action,

}



pub(crate) async fn list_workflow_groups(State(state): State<DaemonState>) -> Json<Response> {

    if let Err(response) = require_workflow_leader(&state) {

        return Json(response);

    }

    Json(match workflow_groups_view(&state) {

        Ok(view) => Response::ok("workflow groups", view),

        Err(error) => Response::error(format!("{error:#}")),

    })

}



pub(crate) async fn get_workflow_group(

    State(state): State<DaemonState>,

    Path(group): Path<String>,

) -> Json<Response> {

    if let Err(response) = require_workflow_leader(&state) {

        return Json(response);

    }

    Json(match state.store.workflow_group(&group) {

        Ok(Some(group)) => Response::ok("workflow group", workflow_group_view(&state, group)),

        Ok(None) => Response::error(format!("workflow group '{group}' not found")),

        Err(error) => Response::error(format!("{error:#}")),

    })

}



pub(crate) async fn create_workflow_group(

    State(state): State<DaemonState>,

    Json(input): Json<WorkflowGroupInput>,

) -> Json<Response> {

    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = serde_json::to_value(&input).unwrap_or_else(|_| json!({}));

    let response = match require_workflow_leader(&state)

        .and_then(|_| validate_workflow_group_scope(&state, &input))

    {

        Ok(()) => match state.store.create_workflow_group(input) {

            Ok(group) => Response::ok("workflow group created", workflow_group_view(&state, group)),

            Err(error) => Response::error(format!("{error:#}")),

        },

        Err(response) => response,

    };

    record_workflow_group_http_audit(

        &state,

        "workflow_group_create",

        None,

        request,

        &response,

        started_at_ms,

        started.elapsed().as_millis() as u64,

    );

    Json(response)

}



pub(crate) async fn update_workflow_group(

    State(state): State<DaemonState>,

    Path(group): Path<String>,

    Json(input): Json<WorkflowGroupInput>,

) -> Json<Response> {

    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = serde_json::to_value(&input).unwrap_or_else(|_| json!({}));

    let response = match require_workflow_leader(&state)

        .and_then(|_| validate_workflow_group_scope(&state, &input))

    {

        Ok(()) => match state.store.update_workflow_group(&group, input, None) {

            Ok(group) => Response::ok("workflow group updated", workflow_group_view(&state, group)),

            Err(error) => Response::error(format!("{error:#}")),

        },

        Err(response) => response,

    };

    record_workflow_group_http_audit(

        &state,

        "workflow_group_update",

        Some(&group),

        request,

        &response,

        started_at_ms,

        started.elapsed().as_millis() as u64,

    );

    Json(response)

}



pub(crate) async fn delete_workflow_group(

    State(state): State<DaemonState>,

    Path(group): Path<String>,

) -> Json<Response> {

    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = json!({"id": group});

    let response = match require_workflow_leader(&state) {

        Ok(()) => match state.store.delete_workflow_group(&group) {

            Ok(true) => Response::empty("workflow group deleted"),

            Ok(false) => Response::error(format!("workflow group '{group}' not found")),

            Err(error) => Response::error(format!("{error:#}")),

        },

        Err(response) => response,

    };

    record_workflow_group_http_audit(

        &state,

        "workflow_group_delete",

        Some(&group),

        request,

        &response,

        started_at_ms,

        started.elapsed().as_millis() as u64,

    );

    Json(response)

}



pub(crate) async fn workflow_group_action(

    State(state): State<DaemonState>,

    Path(group): Path<String>,

    Json(body): Json<WorkflowGroupActionBody>,

) -> Json<Response> {

    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = json!({"id": group, "action": body.action});

    let response = match require_workflow_leader(&state) {

        Ok(()) => match state.store.workflow_group(&group) {

            Ok(Some(group)) => {

                let summary = run_workflow_group_action(state.clone(), group, body.action).await;

                Response::ok("workflow group action completed", summary)

            }

            Ok(None) => Response::error(format!("workflow group '{group}' not found")),

            Err(error) => Response::error(format!("{error:#}")),

        },

        Err(response) => response,

    };

    record_workflow_group_http_audit(

        &state,

        "workflow_group_action",

        Some(&group),

        request,

        &response,

        started_at_ms,

        started.elapsed().as_millis() as u64,

    );

    Json(response)

}



pub(crate) fn require_workflow_leader(state: &DaemonState) -> std::result::Result<(), Response> {

    if state.public_settings().role == NodeRole::Leader {

        Ok(())

    } else {

        Err(Response::error_with_data(

            "workflow groups are available on leader nodes only",

            json!({"kind": "validation_error", "status": 403}),

        ))

    }

}



pub(crate) fn validate_workflow_group_scope(

    state: &DaemonState,

    input: &WorkflowGroupInput,

) -> std::result::Result<(), Response> {

    let settings = state.public_settings();

    let known_nodes = state

        .node_summaries()

        .into_iter()

        .map(|node| node.id)

        .collect::<HashSet<_>>();

    for member in &input.members {

        let node_id = member.node_id.trim();

        if node_id == "self" && !settings.execution_enabled {

            return Err(Response::error_with_data(

                "pure master workflow groups cannot include self executor",

                json!({"kind": "validation_error", "status": 400}),

            ));

        }

        if !known_nodes.contains(node_id) {

            return Err(Response::error_with_data(

                format!("workflow group member node '{node_id}' is not known"),

                json!({"kind": "validation_error", "status": 400}),

            ));

        }

    }

    Ok(())

}



pub(crate) fn workflow_groups_view(state: &DaemonState) -> Result<WorkflowGroupsView> {

    let groups = state.store.workflow_groups()?;

    let (nodes, inventories) = workflow_context(state);

    let targets = workflow_targets(&nodes, &inventories);

    let grouped_workspaces = groups

        .iter()

        .flat_map(|group| group.members.iter())

        .map(|member| (member.node_id.clone(), member.session.clone()))

        .collect::<HashSet<_>>();

    let ungrouped = targets

        .iter()

        .filter(|target| {

            !grouped_workspaces.contains(&(target.node_id.clone(), target.session.clone()))

        })

        .cloned()

        .collect();

    let groups = groups

        .iter()

        .map(|group| resolve_workflow_group(group, &nodes, &inventories))

        .collect();

    Ok(WorkflowGroupsView {

        groups,

        targets,

        ungrouped,

    })

}



pub(crate) fn workflow_group_view(state: &DaemonState, group: WorkflowGroup) -> WorkflowGroupView {

    let (nodes, inventories) = workflow_context(state);

    resolve_workflow_group(&group, &nodes, &inventories)

}



pub(crate) fn workflow_context(

    state: &DaemonState,

) -> (Vec<NodeSummary>, HashMap<String, Vec<SessionSnapshot>>) {

    let nodes = state.node_summaries();

    let inventories = nodes

        .iter()

        .map(|node| {

            let inventory = if node.id == "self" {

                state.local_inventory()

            } else {

                state.cluster.cached_inventory(&node.id).unwrap_or_default()

            };

            (node.id.clone(), inventory)

        })

        .collect();

    (nodes, inventories)

}



pub(crate) fn workflow_targets(

    nodes: &[NodeSummary],

    inventories: &HashMap<String, Vec<SessionSnapshot>>,

) -> Vec<WorkflowTargetView> {

    let mut targets = nodes

        .iter()

        .flat_map(|node| {

            inventories

                .get(&node.id)

                .into_iter()

                .flatten()

                .map(move |session| WorkflowTargetView {

                    node_id: node.id.clone(),

                    node_name: node.name.clone(),

                    node_online: node.online,

                    session: session.name.clone(),

                    workspace_alias: session.alias.clone(),

                    workspace_display_name: session

                        .alias

                        .clone()

                        .unwrap_or_else(|| session.name.clone()),

                    project: Some(session.project.clone()),

                    tasks: ordered_task_labels(session),

                })

        })

        .collect::<Vec<_>>();

    targets.sort_by(|left, right| {

        left.node_name

            .cmp(&right.node_name)

            .then_with(|| {

                left.workspace_display_name

                    .cmp(&right.workspace_display_name)

            })

            .then_with(|| left.session.cmp(&right.session))

    });

    targets

}



pub(crate) fn resolve_workflow_group(

    group: &WorkflowGroup,

    nodes: &[NodeSummary],

    inventories: &HashMap<String, Vec<SessionSnapshot>>,

) -> WorkflowGroupView {

    let members = group

        .members

        .iter()

        .map(|member| {

            let node = nodes.iter().find(|node| node.id == member.node_id);

            let session = inventories.get(&member.node_id).and_then(|sessions| {

                sessions

                    .iter()

                    .find(|session| session.name == member.session)

            });

            let task_exists =

                session.is_some_and(|session| session.tasks.contains_key(&member.task));

            let skip_reason = if node.is_none() {

                Some("node not known".to_string())

            } else if node.is_some_and(|node| !node.online) {

                Some("node offline".to_string())

            } else if session.is_none() {

                Some("workspace not found".to_string())

            } else if !task_exists {

                Some("task not found".to_string())

            } else {

                None

            };

            WorkflowGroupMemberView {

                member: member.clone(),

                node_name: node.map(|node| node.name.clone()),

                node_online: node.is_some_and(|node| node.online),

                workspace_alias: session.and_then(|session| session.alias.clone()),

                workspace_display_name: session

                    .and_then(|session| {

                        session.alias.clone().or_else(|| Some(session.name.clone()))

                    })

                    .unwrap_or_else(|| member.session.clone()),

                project: session.map(|session| session.project.clone()),

                task_exists,

                available: skip_reason.is_none(),

                skip_reason,

            }

        })

        .collect();

    WorkflowGroupView {

        id: group.id.clone(),

        name: group.name.clone(),

        created_at_ms: group.created_at_ms,

        updated_at_ms: group.updated_at_ms,

        members,

        graph: group.graph.clone(),

    }

}



pub(crate) fn ordered_task_labels(session: &SessionSnapshot) -> Vec<String> {

    let mut labels = Vec::new();

    let mut seen = HashSet::new();

    for label in &session.task_order {

        if session.tasks.contains_key(label) && seen.insert(label.clone()) {

            labels.push(label.clone());

        }

    }

    let mut remaining = session

        .tasks

        .keys()

        .filter(|label| !seen.contains(*label))

        .cloned()

        .collect::<Vec<_>>();

    remaining.sort();

    labels.extend(remaining);

    labels

}



pub(crate) async fn run_workflow_group_action(

    state: DaemonState,

    group: WorkflowGroup,

    action: Action,

) -> WorkflowGroupActionSummary {

    let view = workflow_group_view(&state, group.clone());

    let mut results = Vec::new();

    for member in view.members {

        if !member.available {

            results.push(WorkflowGroupActionItem {

                node_id: member.member.node_id,

                node_name: member.node_name,

                session: member.member.session,

                workspace_display_name: member.workspace_display_name,

                task: member.member.task,

                status: WorkflowGroupActionItemStatus::Skipped,

                message: member.skip_reason.unwrap_or_else(|| "skipped".to_string()),

            });

            continue;

        }



        let request = RemoteRequest::Action {

            session: member.member.session.clone(),

            task: Some(member.member.task.clone()),

            action,

        };

        let response = state

            .dispatch_node_with_audit(

                &member.member.node_id,

                request.clone(),

                web_audit_context(&state, &request),

            )

            .await;

        results.push(WorkflowGroupActionItem {

            node_id: member.member.node_id,

            node_name: member.node_name,

            session: member.member.session,

            workspace_display_name: member.workspace_display_name,

            task: member.member.task,

            status: if response.ok {

                WorkflowGroupActionItemStatus::Success

            } else {

                WorkflowGroupActionItemStatus::Failed

            },

            message: response.message,

        });

    }

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

        action,

        results,

        success_count,

        failed_count,

        skipped_count,

    }

}



pub(crate) fn record_workflow_group_http_audit(

    state: &DaemonState,

    operation: &str,

    group_id: Option<&str>,

    request: serde_json::Value,

    response: &Response,

    started_at_ms: u64,

    duration_ms: u64,

) {

    let response_value = serde_json::to_value(response)

        .unwrap_or_else(|error| json!({"ok": response.ok, "message": format!("{error}")}));

    let failed_count = response

        .data

        .as_ref()

        .and_then(|data| data.get("failed_count"))

        .and_then(Value::as_u64)

        .unwrap_or(0);

    let status = if response.ok && failed_count == 0 {

        AuditStatus::Success

    } else {

        AuditStatus::Error

    };

    let _ = record_audit_value(

        state,

        AuditContext::new(AuditSource::Web, AuditTransport::Http),

        None,

        "workflow_group",

        operation,

        None,

        None,

        status,

        started_at_ms,

        duration_ms,

        request,

        response_value,

        json!({"group_id": group_id}),

        Some(state.public_settings().node_id),

    );

}
