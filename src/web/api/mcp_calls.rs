//! MCP call history endpoints.

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
use super::nodes::*;
use super::notifications::*;
use super::quotas::*;
use super::scaling::*;
use super::sessions::*;
use super::tokens::*;
use super::workflow_groups::*;
use super::workflow_runs::*;
use super::workspaces::*;
pub(crate) async fn list_mcp_calls(

    State(state): State<DaemonState>,

    Query(query): Query<HashMap<String, String>>,

) -> Json<Response> {

    let query = match McpCallListQuery::parse(&query) {

        Ok(query) => query,

        Err(response) => return Json(response),

    };

    match state.store.list_mcp_calls(

        query.q.as_deref(),

        query.operation.as_deref(),

        match query.status {

            McpCallStatusFilter::All => None,

            McpCallStatusFilter::Success => Some(true),

            McpCallStatusFilter::Error => Some(false),

        },

        query.session.as_deref(),

        query.task.as_deref(),

        query.page,

        query.page_size,

    ) {

        Ok(page) => Json(Response::ok("MCP calls", page)),

        Err(error) => Json(Response::error(format!("{error:#}"))),

    }

}



#[derive(Debug, Clone, Copy, PartialEq, Eq)]

enum McpCallStatusFilter {

    All,

    Success,

    Error,

}



#[derive(Debug, Clone)]

pub(crate) struct McpCallListQuery {

    q: Option<String>,

    operation: Option<String>,

    status: McpCallStatusFilter,

    session: Option<String>,

    task: Option<String>,

    page: usize,

    page_size: usize,

}



impl McpCallListQuery {

    fn parse(query: &HashMap<String, String>) -> std::result::Result<Self, Response> {

        Ok(Self {

            q: optional_query_value(query, "q").map(|value| casefold_search_text(&value)),

            operation: optional_query_value(query, "operation"),

            status: parse_mcp_call_status(query)?,

            session: optional_query_value(query, "session"),

            task: optional_query_value(query, "task"),

            page: parse_positive_usize(query, "page", 1)?,

            page_size: parse_mcp_call_page_size(query)?,

        })

    }

}

pub(crate) fn parse_mcp_call_status(

    query: &HashMap<String, String>,

) -> std::result::Result<McpCallStatusFilter, Response> {

    match optional_query_value(query, "status").as_deref() {

        None | Some("all") => Ok(McpCallStatusFilter::All),

        Some("success") => Ok(McpCallStatusFilter::Success),

        Some("error") => Ok(McpCallStatusFilter::Error),

        Some(_) => Err(Response::error_with_data(

            "invalid status",

            json!({

                "kind": "validation_error",

                "status": 400,

            }),

        )),

    }

}



pub(crate) fn parse_mcp_call_page_size(

    query: &HashMap<String, String>,

) -> std::result::Result<usize, Response> {

    const SUPPORTED: [usize; 3] = [20, 50, 100];

    let requested = match optional_query_value(query, "page_size")

        .or_else(|| optional_query_value(query, "limit"))

    {

        None => return Ok(20),

        Some(value) => value

            .parse::<usize>()

            .ok()

            .filter(|value| *value > 0)

            .ok_or_else(|| {

                Response::error_with_data(

                    "invalid page_size",

                    json!({

                        "kind": "validation_error",

                        "status": 400,

                    }),

                )

            })?,

    };

    Ok(*SUPPORTED

        .iter()

        .min_by_key(|size| (requested.abs_diff(**size), **size))

        .expect("supported page sizes"))

}



pub(crate) async fn mcp_call_detail(State(state): State<DaemonState>, Path(id): Path<u64>) -> Json<Response> {

    match state.store.mcp_call_detail(id) {

        Ok(Some(record)) => Json(Response::ok("MCP call", record)),

        Ok(None) => Json(Response::error(format!("MCP call '{id}' not found"))),

        Err(error) => Json(Response::error(format!("{error:#}"))),

    }

}
