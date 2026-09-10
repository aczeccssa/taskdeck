//! Board CRUD, scope checks and audit.

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
use super::workspaces::*;
pub(crate) async fn list_boards(State(state): State<DaemonState>) -> Json<Response> {
    if let Err(response) = require_board_leader(&state) {
        return Json(response);
    }

    Json(match boards_view(&state) {
        Ok(view) => Response::ok("boards", view),

        Err(error) => Response::error(format!("{error:#}")),
    })
}

pub(crate) async fn get_board(
    State(state): State<DaemonState>,
    Path(board): Path<String>,
) -> Json<Response> {
    if let Err(response) = require_board_leader(&state) {
        return Json(response);
    }

    Json(match state.store.board(&board) {
        Ok(Some(board)) => Response::ok("board", board_view(&state, board)),

        Ok(None) => Response::error(format!("board '{board}' not found")),

        Err(error) => Response::error(format!("{error:#}")),
    })
}

pub(crate) async fn create_board(
    State(state): State<DaemonState>,

    Json(input): Json<BoardInput>,
) -> Json<Response> {
    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = serde_json::to_value(&input).unwrap_or_else(|_| json!({}));

    let response =
        match require_board_leader(&state).and_then(|_| validate_board_scope(&state, &input)) {
            Ok(()) => match state.store.create_board(input) {
                Ok(board) => Response::ok("board created", board_view(&state, board)),

                Err(error) => Response::error(format!("{error:#}")),
            },

            Err(response) => response,
        };

    record_board_http_audit(
        &state,
        "board_create",
        None,
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );

    Json(response)
}

pub(crate) async fn update_board(
    State(state): State<DaemonState>,

    Path(board): Path<String>,

    Json(input): Json<BoardInput>,
) -> Json<Response> {
    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = serde_json::to_value(&input).unwrap_or_else(|_| json!({}));

    let response =
        match require_board_leader(&state).and_then(|_| validate_board_scope(&state, &input)) {
            Ok(()) => match state.store.update_board(&board, input) {
                Ok(board) => Response::ok("board updated", board_view(&state, board)),

                Err(error) => Response::error(format!("{error:#}")),
            },

            Err(response) => response,
        };

    record_board_http_audit(
        &state,
        "board_update",
        Some(&board),
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );

    Json(response)
}

pub(crate) async fn delete_board(
    State(state): State<DaemonState>,

    Path(board): Path<String>,
) -> Json<Response> {
    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = json!({"id": board});

    let response = match require_board_leader(&state) {
        Ok(()) => match state.store.delete_board(&board) {
            Ok(true) => Response::empty("board deleted"),

            Ok(false) => Response::error(format!("board '{board}' not found")),

            Err(error) => Response::error(format!("{error:#}")),
        },

        Err(response) => response,
    };

    record_board_http_audit(
        &state,
        "board_delete",
        Some(&board),
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );

    Json(response)
}

pub(crate) fn require_board_leader(state: &DaemonState) -> std::result::Result<(), Response> {
    if state.public_settings().role == NodeRole::Leader {
        Ok(())
    } else {
        Err(Response::error_with_data(
            "boards are available on leader nodes only",
            json!({"kind": "validation_error", "status": 403}),
        ))
    }
}

pub(crate) fn validate_board_scope(
    state: &DaemonState,

    input: &BoardInput,
) -> std::result::Result<(), Response> {
    let settings = state.public_settings();

    let known_nodes = state
        .node_summaries()
        .into_iter()
        .map(|node| node.id)
        .collect::<HashSet<_>>();

    for card in &input.cards {
        let node_id = card.node_id.trim();

        if node_id == "self" && !settings.execution_enabled {
            return Err(Response::error_with_data(
                "pure master boards cannot include self executor",
                json!({"kind": "validation_error", "status": 400}),
            ));
        }

        if !known_nodes.contains(node_id) {
            return Err(Response::error_with_data(
                format!("board card node '{node_id}' is not known"),
                json!({"kind": "validation_error", "status": 400}),
            ));
        }
    }

    Ok(())
}

pub(crate) fn boards_view(state: &DaemonState) -> Result<BoardsView> {
    let boards = state.store.boards()?;

    let (nodes, inventories) = workflow_context(state);

    let targets = workflow_targets(&nodes, &inventories);

    let boards = boards
        .into_iter()
        .map(|board| resolve_board(board, &nodes, &inventories))
        .collect();

    Ok(BoardsView { boards, targets })
}

pub(crate) fn board_view(state: &DaemonState, board: Board) -> BoardView {
    let (nodes, inventories) = workflow_context(state);

    resolve_board(board, &nodes, &inventories)
}

pub(crate) fn resolve_board(
    board: Board,

    nodes: &[NodeSummary],

    inventories: &HashMap<String, Vec<SessionSnapshot>>,
) -> BoardView {
    let cards = board
        .cards
        .iter()
        .map(|card| {
            let node = nodes.iter().find(|node| node.id == card.node_id);

            let session = inventories
                .get(&card.node_id)
                .and_then(|sessions| sessions.iter().find(|session| session.name == card.session));

            let task_exists = session.is_some_and(|session| session.tasks.contains_key(&card.task));

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

            BoardCardView {
                card: card.clone(),

                node_name: node.map(|node| node.name.clone()),

                node_online: node.is_some_and(|node| node.online),

                workspace_alias: session.and_then(|session| session.alias.clone()),

                workspace_display_name: session
                    .and_then(|session| {
                        session.alias.clone().or_else(|| Some(session.name.clone()))
                    })
                    .unwrap_or_else(|| card.session.clone()),

                project: session.map(|session| session.project.clone()),

                task_exists,

                available: skip_reason.is_none(),

                skip_reason,
            }
        })
        .collect();

    BoardView {
        id: board.id,

        name: board.name,

        created_at_ms: board.created_at_ms,

        updated_at_ms: board.updated_at_ms,

        cards,
    }
}

pub(crate) fn record_board_http_audit(
    state: &DaemonState,

    operation: &str,

    board_id: Option<&str>,

    request: serde_json::Value,

    response: &Response,

    started_at_ms: u64,

    duration_ms: u64,
) {
    let response_value = serde_json::to_value(response)
        .unwrap_or_else(|error| json!({"ok": response.ok, "message": format!("{error}")}));

    let status = if response.ok {
        AuditStatus::Success
    } else {
        AuditStatus::Error
    };

    let _ = record_audit_value(
        state,
        AuditContext::new(AuditSource::Web, AuditTransport::Http),
        None,
        "board",
        operation,
        None,
        None,
        status,
        started_at_ms,
        duration_ms,
        request,
        response_value,
        json!({"board_id": board_id}),
        Some(state.public_settings().node_id),
    );
}

// ---------------------------------------------------------------------------

// Workflow revisions (version history)

// ---------------------------------------------------------------------------
