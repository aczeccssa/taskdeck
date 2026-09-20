//! Embedded MCP server over HTTP (tool schema + dispatch).

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

use super::api::audit::{mcp_audit_context, record_mcp_direct_audit};
use crate::web::current_millis;
use crate::web::optional_query_value;
use crate::web::parse_positive_usize;
pub(crate) async fn mcp(State(state): State<DaemonState>, Json(rpc): Json<Value>) -> AxumResponse {
    let started = Instant::now();

    let started_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let id = rpc.get("id").cloned().unwrap_or(Value::Null);

    let method = rpc.get("method").and_then(Value::as_str).unwrap_or("");

    if method.starts_with("notifications/") {
        return StatusCode::ACCEPTED.into_response();
    }

    let result = match method {
        "initialize" => json!({

            "protocolVersion": "2025-03-26",

            "capabilities": {"tools": {}},

            "serverInfo": {"name": "taskdeck", "version": crate::version::VERSION},

            "instructions": if state.public_settings().role == crate::state::NodeRole::Leader {

                "Use taskdeck_control with an explicit node to inspect and control this Taskdeck cluster."

            } else {

                "Use taskdeck_control to inspect and control tasks on this local Taskdeck worker."

            }

        }),

        "ping" => json!({}),

        "tools/list" => json!({"tools": [mcp_tool_definition(&state)]}),

        "tools/call" => {
            let params = rpc.get("params").cloned().unwrap_or_else(|| json!({}));

            match call_mcp_tool(state.clone(), params, started_at_ms).await {
                Ok(result) => result,

                Err(message) => json!({

                    "content": [{"type": "text", "text": message}],

                    "isError": true

                }),
            }
        }

        _ => {
            return Json(json!({

                "jsonrpc": "2.0", "id": id,

                "error": {"code": -32601, "message": format!("method not found: {method}")}

            }))
            .into_response();
        }
    };

    let success = !result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let response = json!({"jsonrpc": "2.0", "id": id, "result": result});

    if method == "tools/call" {
        let params = rpc.get("params").unwrap_or(&Value::Null);

        let target_node = params
            .get("arguments")
            .and_then(|arguments| arguments.get("node"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                (state.public_settings().role == crate::state::NodeRole::Worker)
                    .then(|| "self".to_string())
            });

        let record = McpCallRecord {
            id: 0,

            tool: params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),

            operation: params
                .get("arguments")
                .and_then(|arguments| arguments.get("action"))
                .and_then(Value::as_str)
                .map(str::to_string),

            started_at_ms,

            duration_ms: started.elapsed().as_millis() as u64,

            success,

            target_node,

            request: rpc,

            response: response.clone(),
        };

        if let Err(error) = state.store.record_mcp_call(record) {
            eprintln!("failed to persist MCP call: {error:#}");
        }
    }

    Json(response).into_response()
}

pub(crate) fn mcp_tool_definition(state: &DaemonState) -> Value {
    if state.public_settings().role == crate::state::NodeRole::Leader {
        json!({

            "name": "taskdeck_control",

            "description": "Inspect nodes, sessions, and discovered services or control a task on this Taskdeck cluster.",

            "inputSchema": {

                "type": "object",

                "properties": {

                    "action": {

                        "type": "string",

                        "enum": ["nodes", "sessions", "services", "status", "logs", "runs", "start", "stop", "restart", "pause", "resume"],

                        "description": "Cluster operation to perform."

                    },

                    "node": {"type": "string", "description": "Node ID. Required for targeted operations; use self for this standard leader."},

                    "session": {"type": "string", "description": "Session name on the selected node."},

                    "task": {"type": "string", "description": "Task label. Omit to target every task in a session."},

                    "tail": {"type": "integer", "minimum": 1, "maximum": 5000, "default": 200}

                },

                "required": ["action"],

                "additionalProperties": false

            }

        })
    } else {
        json!({

            "name": "taskdeck_control",

            "description": "List local Taskdeck sessions, inspect task status/logs, or control one local task/all tasks in a session.",

            "inputSchema": {

                "type": "object",

                "properties": {

                    "action": {

                        "type": "string",

                        "enum": ["sessions", "status", "logs", "runs", "start", "stop", "restart", "pause", "resume"],

                        "description": "Local operation to perform."

                    },

                    "session": {"type": "string", "description": "Local session name; required except for sessions."},

                    "task": {"type": "string", "description": "Task label. Omit for all tasks or a full session snapshot."},

                    "tail": {"type": "integer", "minimum": 1, "maximum": 5000, "default": 200}

                },

                "required": ["action"],

                "additionalProperties": false

            }

        })
    }
}

pub(crate) async fn call_mcp_tool(
    state: DaemonState,

    params: Value,

    started_at_ms: u64,
) -> std::result::Result<Value, String> {
    let started = Instant::now();

    if params.get("name").and_then(Value::as_str) != Some("taskdeck_control") {
        let response = Response::error("unknown tool; expected taskdeck_control");

        record_mcp_direct_audit(
            &state,
            &params,
            "unknown",
            &response,
            started_at_ms,
            started.elapsed().as_millis() as u64,
            None,
            None,
            None,
        );

        return Err(response.message);
    }

    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let Some(operation) = arguments.get("action").and_then(Value::as_str) else {
        let response = Response::error("action is required");

        record_mcp_direct_audit(
            &state,
            &params,
            "missing_action",
            &response,
            started_at_ms,
            started.elapsed().as_millis() as u64,
            arguments.get("node").and_then(Value::as_str),
            arguments.get("session").and_then(Value::as_str),
            arguments.get("task").and_then(Value::as_str),
        );

        return Err(response.message);
    };

    let is_leader = state.public_settings().role == crate::state::NodeRole::Leader;

    if !is_leader && arguments.get("node").is_some() {
        let response = Response::error("worker MCP is local-only and does not accept node");

        record_mcp_direct_audit(
            &state,
            &params,
            operation,
            &response,
            started_at_ms,
            started.elapsed().as_millis() as u64,
            arguments.get("node").and_then(Value::as_str),
            None,
            None,
        );

        return Err(response.message);
    }

    let node = arguments.get("node").and_then(Value::as_str);

    let session = arguments
        .get("session")
        .and_then(Value::as_str)
        .map(str::to_string);

    let task = arguments
        .get("task")
        .and_then(Value::as_str)
        .map(str::to_string);

    let tail = arguments
        .get("tail")
        .and_then(Value::as_u64)
        .map(|v| v as usize);

    let response = match operation {
        "nodes" if is_leader => {
            let response = Response::ok("nodes", state.node_summaries());

            record_mcp_direct_audit(
                &state,
                &params,
                operation,
                &response,
                started_at_ms,
                started.elapsed().as_millis() as u64,
                node,
                session.as_deref(),
                task.as_deref(),
            );

            response
        }

        "sessions" if is_leader && node.is_none() => {
            let rows = state

                .node_summaries()

                .into_iter()

                .flat_map(|node| {

                    node.sessions.into_iter().map(move |session| {

                        json!({"node": node.id, "node_name": node.name, "session": session, "online": node.online})

                    })

                })

                .collect::<Vec<_>>();

            let response = Response::ok("cluster sessions", rows);

            record_mcp_direct_audit(
                &state,
                &params,
                operation,
                &response,
                started_at_ms,
                started.elapsed().as_millis() as u64,
                node,
                session.as_deref(),
                task.as_deref(),
            );

            response
        }

        "services" if is_leader => {
            let response = Response::ok("services", state.service_rows(node));

            record_mcp_direct_audit(
                &state,
                &params,
                operation,
                &response,
                started_at_ms,
                started.elapsed().as_millis() as u64,
                node,
                session.as_deref(),
                task.as_deref(),
            );

            response
        }

        "sessions" => {
            let request = RemoteRequest::ListSessions;

            state
                .dispatch_node_with_audit(
                    node.unwrap_or("self"),
                    request.clone(),
                    mcp_audit_context(&state, &request),
                )
                .await
        }

        "runs" => {
            let node = if is_leader {
                node.ok_or_else(|| "node is required for targeted leader operations".to_string())?
            } else {
                "self"
            };

            let request = RemoteRequest::ListTaskRuns {
                filter: crate::protocol::TaskRunFilter {
                    session: session.clone(),

                    task,

                    status: None,

                    trigger: None,

                    page: tail.unwrap_or(1),

                    page_size: 50,
                },
            };

            state
                .dispatch_node_with_audit(
                    node,
                    request.clone(),
                    mcp_audit_context(&state, &request),
                )
                .await
        }

        "status" | "logs" => {
            let node = if is_leader {
                node.ok_or_else(|| "node is required for targeted leader operations".to_string())?
            } else {
                "self"
            };

            let request = RemoteRequest::Snapshot {
                session: session.ok_or_else(|| "session is required".to_string())?,

                tail: Some(if operation == "status" {
                    20
                } else {
                    tail.unwrap_or(200)
                }),
            };

            state
                .dispatch_node_with_audit(
                    node,
                    request.clone(),
                    mcp_audit_context(&state, &request),
                )
                .await
        }

        "start" | "stop" | "restart" | "pause" | "resume" => {
            let node = if is_leader {
                node.ok_or_else(|| "node is required for targeted leader operations".to_string())?
            } else {
                "self"
            };

            let request = RemoteRequest::Action {
                session: session.ok_or_else(|| "session is required".to_string())?,

                task,

                action: match operation {
                    "start" => Action::Start,

                    "stop" => Action::Stop,

                    "restart" => Action::Restart,

                    "pause" => Action::Pause,

                    "resume" => Action::Resume,

                    _ => unreachable!(),
                },
            };

            state
                .dispatch_node_with_audit(
                    node,
                    request.clone(),
                    mcp_audit_context(&state, &request),
                )
                .await
        }

        _ => {
            let response = Response::error(format!("unsupported action: {operation}"));

            record_mcp_direct_audit(
                &state,
                &params,
                operation,
                &response,
                started_at_ms,
                started.elapsed().as_millis() as u64,
                node,
                session.as_deref(),
                task.as_deref(),
            );

            return Err(response.message);
        }
    };

    let text = serde_json::to_string_pretty(&response).map_err(|error| error.to_string())?;

    Ok(json!({

        "content": [{"type": "text", "text": text}],

        "structuredContent": response,

        "isError": !response.ok

    }))
}
