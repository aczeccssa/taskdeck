//! Board template endpoints.

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
pub(crate) async fn list_board_templates(State(state): State<DaemonState>) -> Json<Response> {
    Json(match state.store.board_templates() {
        Ok(templates) => Response::ok("board templates", BoardTemplatesView { templates }),

        Err(error) => Response::error(format!("{error:#}")),
    })
}

pub(crate) async fn create_board_template(
    State(state): State<DaemonState>,

    Json(input): Json<BoardTemplateInput>,
) -> Json<Response> {
    let started_at_ms = current_millis();

    let started = Instant::now();

    let mut input = input;

    if let Some(board_id) = input
        .source_board_id
        .clone()
        .filter(|board_id| !board_id.trim().is_empty())
    {
        input.cards = match state.store.board(&board_id) {
            Ok(Some(board)) => board
                .cards
                .into_iter()
                .map(|card| BoardCardInput {
                    node_id: card.node_id,

                    session: card.session,

                    task: card.task,

                    mode: card.mode,

                    pinned: card.pinned,
                })
                .collect(),

            Ok(None) => {
                return Json(Response::error(format!("board '{board_id}' not found")));
            }

            Err(error) => {
                return Json(Response::error(format!("{error:#}")));
            }
        };
    }

    let request = serde_json::to_value(&input).unwrap_or_else(|_| json!({}));

    let response = match state.store.create_board_template(input) {
        Ok(template) => Response::ok("board template created", template),

        Err(error) => Response::error(format!("{error:#}")),
    };

    record_feature_http_audit(
        &state,
        "board_template",
        "board_template_create",
        "template_id",
        None,
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );

    Json(response)
}

pub(crate) async fn import_board_template(
    State(state): State<DaemonState>,

    Json(export): Json<BoardTemplateExport>,
) -> Json<Response> {
    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = serde_json::to_value(&export).unwrap_or_else(|_| json!({}));

    let response = if export.kind != "taskdeck_board_template" {
        Response::error("not a taskdeck board template export")
    } else {
        let input = BoardTemplateInput {
            name: export.name.clone(),

            description: export.description.clone(),

            cards: export.cards.clone(),

            source_board_id: None,
        };

        match state.store.create_board_template(input) {
            Ok(template) => Response::ok("board template imported", template),

            Err(error) => Response::error(format!("{error:#}")),
        }
    };

    record_feature_http_audit(
        &state,
        "board_template",
        "board_template_import",
        "template_id",
        None,
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );

    Json(response)
}

pub(crate) async fn delete_board_template(
    State(state): State<DaemonState>,

    Path(template): Path<String>,
) -> Json<Response> {
    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = json!({"id": template});

    let response = match state.store.delete_board_template(&template) {
        Ok(true) => Response::empty("board template deleted"),

        Ok(false) => Response::error(format!("board template '{template}' not found")),

        Err(error) => Response::error(format!("{error:#}")),
    };

    record_feature_http_audit(
        &state,
        "board_template",
        "board_template_delete",
        "template_id",
        Some(&template),
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );

    Json(response)
}

pub(crate) async fn apply_board_template(
    State(state): State<DaemonState>,

    Path(template): Path<String>,

    Json(input): Json<BoardTemplateApplyInput>,
) -> Json<Response> {
    let started_at_ms = current_millis();

    let started = Instant::now();

    let request = json!({"template_id": template, "name": input.name});

    let response = match require_board_leader(&state) {
        Ok(()) => match state.store.board_template(&template) {
            Ok(Some(template)) => {
                let board_input = BoardInput {
                    name: input.name,

                    cards: template.cards.clone(),
                };

                let scoped = match validate_board_scope(&state, &board_input) {
                    Err(response) => Err(response),

                    Ok(()) => state
                        .store
                        .create_board(board_input)
                        .map_err(|error| Response::error(format!("{error:#}"))),
                };

                match scoped {
                    Ok(board) => {
                        Response::ok("board created from template", board_view(&state, board))
                    }

                    Err(response) => response,
                }
            }

            Ok(None) => Response::error(format!("board template '{template}' not found")),

            Err(error) => Response::error(format!("{error:#}")),
        },

        Err(response) => response,
    };

    record_feature_http_audit(
        &state,
        "board_template",
        "board_template_apply",
        "template_id",
        Some(&template),
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );

    Json(response)
}

pub(crate) async fn export_board_template(
    State(state): State<DaemonState>,

    Path(template): Path<String>,
) -> Json<Response> {
    Json(match state.store.board_template(&template) {
        Ok(Some(template)) => Response::ok(
            "board template export",
            BoardTemplateExport {
                kind: "taskdeck_board_template".to_string(),

                name: template.name,

                description: template.description,

                cards: template.cards,

                exported_at_ms: current_millis(),
            },
        ),

        Ok(None) => Response::error(format!("board template '{template}' not found")),

        Err(error) => Response::error(format!("{error:#}")),
    })
}

// ---------------------------------------------------------------------------

// Cross-workspace task dependencies

// ---------------------------------------------------------------------------
