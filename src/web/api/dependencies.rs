//! Task dependency endpoints incl. cycle validation.

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
use super::workflow_groups::{ordered_task_labels, workflow_context, workflow_targets};
pub(crate) fn dependencies_view(

    state: &DaemonState,

    dependencies: Vec<crate::protocol::TaskDependency>,

) -> TaskDependenciesView {

    let (nodes, inventories) = workflow_context(state);

    let targets = workflow_targets(&nodes, &inventories);

    let dependencies = dependencies

        .into_iter()

        .map(|dependency| {

            let node = nodes

                .iter()

                .find(|node| node.id == dependency.depends_node_id);

            let session = inventories

                .get(&dependency.depends_node_id)

                .and_then(|sessions| {

                    sessions

                        .iter()

                        .find(|session| session.name == dependency.depends_session)

                });

            let target_exists =

                session.is_some_and(|session| session.tasks.contains_key(&dependency.depends_task));

            let target_available = node.is_some_and(|node| node.online) && target_exists;

            crate::protocol::TaskDependencyView {

                dependency,

                target_exists,

                target_available,

            }

        })

        .collect();

    TaskDependenciesView {

        dependencies,

        targets,

    }

}



pub(crate) fn validate_dependency_scope(

    state: &DaemonState,

    input: &TaskDependencyInput,

) -> std::result::Result<(), Response> {

    let settings = state.public_settings();

    let known_nodes = state

        .node_summaries()

        .into_iter()

        .map(|node| node.id)

        .collect::<HashSet<_>>();

    for (role, node_id) in [

        ("dependency task", &input.node_id),

        ("dependency target", &input.depends_node_id),

    ] {

        if node_id == "self" && !settings.execution_enabled {

            return Err(Response::error_with_data(

                "pure master dependencies cannot use the self executor",

                json!({"kind": "validation_error", "status": 400}),

            ));

        }

        if !known_nodes.contains(node_id) {

            return Err(Response::error_with_data(

                format!("{role} node '{node_id}' is not known"),

                json!({"kind": "validation_error", "status": 400}),

            ));

        }

    }

    Ok(())

}



pub(crate) fn dependency_creates_cycle(

    existing: &[crate::protocol::TaskDependency],

    input: &TaskDependencyInput,

) -> bool {

    type Target = (String, String, String);

    let key = |node: &str, session: &str, task: &str| {

        (node.to_string(), session.to_string(), task.to_string())

    };

    let mut edges: HashMap<Target, Vec<Target>> = HashMap::new();

    for dependency in existing {

        edges

            .entry(key(

                &dependency.node_id,

                &dependency.session,

                &dependency.task,

            ))

            .or_default()

            .push(key(

                &dependency.depends_node_id,

                &dependency.depends_session,

                &dependency.depends_task,

            ));

    }

    edges

        .entry(key(&input.node_id, &input.session, &input.task))

        .or_default()

        .push(key(

            &input.depends_node_id,

            &input.depends_session,

            &input.depends_task,

        ));

    let mut visiting: HashSet<Target> = HashSet::new();

    let mut visited: HashSet<Target> = HashSet::new();

    fn visit(

        node: &(String, String, String),

        edges: &HashMap<(String, String, String), Vec<(String, String, String)>>,

        visiting: &mut HashSet<(String, String, String)>,

        visited: &mut HashSet<(String, String, String)>,

    ) -> bool {

        if visiting.contains(node) {

            return true;

        }

        if visited.contains(node) {

            return false;

        }

        visiting.insert(node.clone());

        if let Some(children) = edges.get(node) {

            for child in children {

                if visit(child, edges, visiting, visited) {

                    return true;

                }

            }

        }

        visiting.remove(node);

        visited.insert(node.clone());

        false

    }

    let roots: Vec<Target> = edges.keys().cloned().collect();

    for root in roots {

        if visit(&root, &edges, &mut visiting, &mut visited) {

            return true;

        }

    }

    false

}



pub(crate) async fn list_dependencies(State(state): State<DaemonState>) -> Json<Response> {

    Json(match state.store.task_dependencies() {

        Ok(dependencies) => {

            Response::ok("task dependencies", dependencies_view(&state, dependencies))

        }

        Err(error) => Response::error(format!("{error:#}")),

    })

}



pub(crate) async fn create_dependency(

    State(state): State<DaemonState>,

    Json(input): Json<TaskDependencyInput>,

) -> Json<Response> {

    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = serde_json::to_value(&input).unwrap_or_else(|_| json!({}));

    // Start gates resolve dependencies by node id; persist "self" as the real id

    // after scope validation, which matches "self" against known nodes.

    let mut normalized = input.clone();

    let self_node_id = state.public_settings().node_id;

    if normalized.node_id == "self" {

        normalized.node_id = self_node_id.clone();

    }

    if normalized.depends_node_id == "self" {

        normalized.depends_node_id = self_node_id;

    }

    let response = match state.store.task_dependencies() {

        Ok(existing) => {

            if dependency_creates_cycle(&existing, &normalized) {

                Response::error("task dependency would create a cycle")

            } else {

                let scoped = match validate_dependency_scope(&state, &input) {

                    Err(response) => Err(response),

                    Ok(()) => state

                        .store

                        .create_task_dependency(normalized)

                        .map_err(|error| Response::error(format!("{error:#}"))),

                };

                match scoped {

                    Ok(dependency) => Response::ok(

                        "task dependency created",

                        dependencies_view(&state, vec![dependency])

                            .dependencies

                            .remove(0),

                    ),

                    Err(response) => response,

                }

            }

        }

        Err(error) => Response::error(format!("{error:#}")),

    };

    record_feature_http_audit(

        &state,

        "task_dependency",

        "task_dependency_create",

        "dependency_id",

        None,

        request,

        &response,

        started_at_ms,

        started.elapsed().as_millis() as u64,

    );

    Json(response)

}



pub(crate) async fn delete_dependency(

    State(state): State<DaemonState>,

    Path(dependency): Path<String>,

) -> Json<Response> {

    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = json!({"id": dependency});

    let response = match state.store.delete_task_dependency(&dependency) {

        Ok(true) => Response::empty("task dependency deleted"),

        Ok(false) => Response::error(format!("task dependency '{dependency}' not found")),

        Err(error) => Response::error(format!("{error:#}")),

    };

    record_feature_http_audit(

        &state,

        "task_dependency",

        "task_dependency_delete",

        "dependency_id",

        Some(&dependency),

        request,

        &response,

        started_at_ms,

        started.elapsed().as_millis() as u64,

    );

    Json(response)

}



// ---------------------------------------------------------------------------

// Node metrics (dashboard)

// ---------------------------------------------------------------------------
