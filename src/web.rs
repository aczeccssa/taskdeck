use std::collections::HashMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response as AxumResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::cluster::{self, RemoteRequest};
use crate::daemon::{DaemonState, McpCallHistoryEntry};
use crate::protocol::{
    Action, EditableTaskInput, McpCallListItem, McpCallListPage, McpCallRecord, Response,
    casefold_search_text,
};

pub async fn serve(state: DaemonState, listener: tokio::net::TcpListener) -> Result<()> {
    axum::serve(listener, app(state))
        .await
        .context("Web server failed")
}

fn app(state: DaemonState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/assets/styles.css", get(styles))
        .route("/assets/app.js", get(script))
        .route("/favicon.svg", get(favicon))
        .route("/favicon.ico", get(favicon))
        .route("/healthz", get(health))
        .route("/api/agent/connect", get(agent_connect))
        .route("/api/nodes", get(list_nodes))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{session}", get(session_snapshot))
        .route("/api/sessions/{session}/tasks/{task}/logs", get(task_logs))
        .route(
            "/api/sessions/{session}/tasks/{task}/metrics",
            get(task_metrics),
        )
        .route(
            "/api/sessions/{session}/config",
            get(session_config).put(update_session_config),
        )
        .route("/api/mcp-calls", get(list_mcp_calls))
        .route("/api/mcp-calls/{id}", get(mcp_call_detail))
        .route("/api/action", post(action))
        .route("/mcp", post(mcp))
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn styles() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        STYLES_CSS,
    )
}

async fn script() -> impl IntoResponse {
    (
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        APP_JS,
    )
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn agent_connect(
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

async fn favicon() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        FAVICON_SVG,
    )
}

async fn list_nodes(State(state): State<DaemonState>) -> Json<Response> {
    Json(Response::ok("nodes", state.node_summaries()))
}

fn selected_node(
    state: &DaemonState,
    query: &HashMap<String, String>,
) -> std::result::Result<String, Response> {
    if let Some(node) = query.get("node").filter(|node| !node.trim().is_empty()) {
        return Ok(node.clone());
    }
    if state.public_settings().role == crate::state::NodeRole::Worker {
        Ok("self".to_string())
    } else {
        Err(Response::error_with_data(
            "node is required for leader requests",
            json!({"kind": "validation_error", "status": 400}),
        ))
    }
}

async fn list_sessions(
    State(state): State<DaemonState>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Response> {
    let node = match selected_node(&state, &query) {
        Ok(node) => node,
        Err(response) => return Json(response),
    };
    Json(
        state
            .dispatch_node(&node, RemoteRequest::ListSessions)
            .await,
    )
}

async fn session_snapshot(
    State(state): State<DaemonState>,
    Path(session): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Response> {
    let node = match selected_node(&state, &query) {
        Ok(node) => node,
        Err(response) => return Json(response),
    };
    let tail = query.get("tail").and_then(|value| value.parse().ok());
    Json(
        state
            .dispatch_node(&node, RemoteRequest::Snapshot { session, tail })
            .await,
    )
}

async fn task_logs(
    State(state): State<DaemonState>,
    Path((session, task)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Response> {
    let node = match selected_node(&state, &query) {
        Ok(node) => node,
        Err(response) => return Json(response),
    };
    let after = match query.get("after") {
        Some(value) => match value.parse::<u64>() {
            Ok(value) => Some(value),
            Err(_) => {
                return Json(Response::error_with_data(
                    "invalid log cursor",
                    json!({"kind": "validation_error", "status": 400}),
                ));
            }
        },
        None => None,
    };
    let limit = match query.get("limit") {
        Some(value) => match value.parse::<usize>() {
            Ok(value) if value > 0 => value.clamp(1, 5_000),
            _ => {
                return Json(Response::error_with_data(
                    "invalid log limit",
                    json!({"kind": "validation_error", "status": 400}),
                ));
            }
        },
        None => 1_000,
    };
    Json(
        state
            .dispatch_node(
                &node,
                RemoteRequest::TaskLogs {
                    session,
                    task,
                    after,
                    limit,
                },
            )
            .await,
    )
}

fn parse_metrics_window_seconds(
    query: &HashMap<String, String>,
) -> std::result::Result<usize, Response> {
    match query.get("window") {
        None => Ok(600),
        Some(value) => value
            .parse::<usize>()
            .map(|window| window.clamp(1, 600))
            .map_err(|_| {
                Response::error_with_data(
                    "invalid metrics window",
                    json!({
                        "kind": "validation_error",
                        "status": 400,
                    }),
                )
            }),
    }
}

async fn task_metrics(
    State(state): State<DaemonState>,
    Path((session, task)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Response> {
    let node = match selected_node(&state, &query) {
        Ok(node) => node,
        Err(response) => return Json(response),
    };
    let window_seconds = match parse_metrics_window_seconds(&query) {
        Ok(window_seconds) => window_seconds,
        Err(response) => return Json(response),
    };
    Json(
        state
            .dispatch_node(
                &node,
                RemoteRequest::TaskMetrics {
                    session,
                    task,
                    window_seconds,
                },
            )
            .await,
    )
}

async fn session_config(
    State(state): State<DaemonState>,
    Path(session): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Response> {
    let node = match selected_node(&state, &query) {
        Ok(node) => node,
        Err(response) => return Json(response),
    };
    Json(
        state
            .dispatch_node(&node, RemoteRequest::GetSessionConfig { session })
            .await,
    )
}

#[derive(Deserialize)]
struct UpdateSessionConfigBody {
    revision: String,
    tasks: Vec<EditableTaskInput>,
}

async fn update_session_config(
    State(state): State<DaemonState>,
    Path(session): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    Json(body): Json<UpdateSessionConfigBody>,
) -> Json<Response> {
    let node = match selected_node(&state, &query) {
        Ok(node) => node,
        Err(response) => return Json(response),
    };
    Json(
        state
            .dispatch_node(
                &node,
                RemoteRequest::PutSessionConfig {
                    session,
                    revision: body.revision,
                    tasks: body.tasks,
                },
            )
            .await,
    )
}

#[derive(Deserialize)]
struct ActionBody {
    node: Option<String>,
    session: String,
    task: Option<String>,
    action: Action,
}

async fn action(State(state): State<DaemonState>, Json(body): Json<ActionBody>) -> Json<Response> {
    let node = match body.node {
        Some(node) if !node.trim().is_empty() => node,
        _ if state.public_settings().role == crate::state::NodeRole::Worker => "self".to_string(),
        _ => {
            return Json(Response::error_with_data(
                "node is required for leader actions",
                json!({"kind": "validation_error", "status": 400}),
            ));
        }
    };
    Json(
        state
            .dispatch_node(
                &node,
                RemoteRequest::Action {
                    session: body.session,
                    task: body.task,
                    action: body.action,
                },
            )
            .await,
    )
}

async fn list_mcp_calls(
    State(state): State<DaemonState>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Response> {
    let query = match McpCallListQuery::parse(&query) {
        Ok(query) => query,
        Err(response) => return Json(response),
    };
    let calls = state.recent_mcp_calls(crate::daemon::MAX_MCP_CALLS);
    let filtered = calls
        .iter()
        .filter(|call| query.matches(call.as_ref()))
        .collect::<Vec<_>>();
    let total = filtered.len();
    let total_pages = if total == 0 {
        0
    } else {
        total.div_ceil(query.page_size)
    };
    let start = query.page.saturating_sub(1).saturating_mul(query.page_size);
    let items = if start >= total {
        Vec::new()
    } else {
        filtered
            .into_iter()
            .skip(start)
            .take(query.page_size)
            .map(|call| mcp_call_summary(call.as_ref()))
            .collect()
    };
    Json(Response::ok(
        "MCP calls",
        McpCallListPage {
            items,
            page: query.page,
            page_size: query.page_size,
            total,
            total_pages,
            has_next: query.page < total_pages,
            has_previous: query.page > 1 && total > 0,
        },
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpCallStatusFilter {
    All,
    Success,
    Error,
}

#[derive(Debug, Clone)]
struct McpCallListQuery {
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

    fn matches(&self, call: &McpCallHistoryEntry) -> bool {
        self.matches_operation(call.record.operation.as_deref())
            && self.matches_status(call.record.success)
            && self.matches_exact(self.session.as_deref(), call.session.as_deref())
            && self.matches_exact(self.task.as_deref(), call.task.as_deref())
            && self.matches_search(call)
    }

    fn matches_operation(&self, operation: Option<&str>) -> bool {
        self.matches_exact(self.operation.as_deref(), operation)
    }

    fn matches_status(&self, success: bool) -> bool {
        match self.status {
            McpCallStatusFilter::All => true,
            McpCallStatusFilter::Success => success,
            McpCallStatusFilter::Error => !success,
        }
    }

    fn matches_exact(&self, expected: Option<&str>, actual: Option<&str>) -> bool {
        match expected {
            Some(expected) => actual == Some(expected),
            None => true,
        }
    }

    fn matches_search(&self, call: &McpCallHistoryEntry) -> bool {
        let Some(needle) = self.q.as_deref() else {
            return true;
        };
        call.searchable_text.contains(needle)
    }
}

fn optional_query_value(query: &HashMap<String, String>, key: &str) -> Option<String> {
    query.get(key).and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn parse_positive_usize(
    query: &HashMap<String, String>,
    key: &str,
    default: usize,
) -> std::result::Result<usize, Response> {
    match optional_query_value(query, key) {
        None => Ok(default),
        Some(value) => value
            .parse::<usize>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                Response::error_with_data(
                    format!("invalid {key}"),
                    json!({
                        "kind": "validation_error",
                        "status": 400,
                    }),
                )
            }),
    }
}

fn parse_mcp_call_status(
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

fn parse_mcp_call_page_size(
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

fn mcp_call_summary(call: &McpCallHistoryEntry) -> McpCallListItem {
    McpCallListItem {
        id: call.record.id,
        tool: call.record.tool.clone(),
        operation: call.record.operation.clone(),
        started_at_ms: call.record.started_at_ms,
        duration_ms: call.record.duration_ms,
        success: call.record.success,
        input: call.input.clone(),
    }
}

async fn mcp_call_detail(State(state): State<DaemonState>, Path(id): Path<u64>) -> Json<Response> {
    match state.mcp_call(id) {
        Some(call) => Json(Response::ok("MCP call", &call.record)),
        None => Json(Response::error(format!("MCP call '{id}' not found"))),
    }
}

async fn mcp(State(state): State<DaemonState>, Json(rpc): Json<Value>) -> AxumResponse {
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
            "serverInfo": {"name": "taskdeck", "version": env!("CARGO_PKG_VERSION")},
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
            match call_mcp_tool(state.clone(), params).await {
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
        state.record_mcp_call(McpCallRecord {
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
            request: rpc,
            response: response.clone(),
        });
    }
    Json(response).into_response()
}

fn mcp_tool_definition(state: &DaemonState) -> Value {
    if state.public_settings().role == crate::state::NodeRole::Leader {
        json!({
            "name": "taskdeck_control",
            "description": "Inspect nodes, sessions, and discovered services or control a task on this Taskdeck cluster.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["nodes", "sessions", "services", "status", "logs", "start", "stop", "restart", "pause", "resume"],
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
                        "enum": ["sessions", "status", "logs", "start", "stop", "restart", "pause", "resume"],
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

async fn call_mcp_tool(state: DaemonState, params: Value) -> std::result::Result<Value, String> {
    if params.get("name").and_then(Value::as_str) != Some("taskdeck_control") {
        return Err("unknown tool; expected taskdeck_control".to_string());
    }
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let operation = arguments
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| "action is required".to_string())?;
    let is_leader = state.public_settings().role == crate::state::NodeRole::Leader;
    if !is_leader && arguments.get("node").is_some() {
        return Err("worker MCP is local-only and does not accept node".to_string());
    }
    let node = arguments.get("node").and_then(Value::as_str);
    let session = arguments.get("session").and_then(Value::as_str);
    let task = arguments
        .get("task")
        .and_then(Value::as_str)
        .map(str::to_string);
    let tail = arguments
        .get("tail")
        .and_then(Value::as_u64)
        .map(|v| v as usize);
    let response = match operation {
        "nodes" if is_leader => Response::ok("nodes", state.node_summaries()),
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
            Response::ok("cluster sessions", rows)
        }
        "services" if is_leader => Response::ok("services", state.service_rows(node)),
        "sessions" => {
            state
                .dispatch_node(node.unwrap_or("self"), RemoteRequest::ListSessions)
                .await
        }
        "status" | "logs" => {
            let node = if is_leader {
                node.ok_or_else(|| "node is required for targeted leader operations".to_string())?
            } else {
                "self"
            };
            state
                .dispatch_node(
                    node,
                    RemoteRequest::Snapshot {
                        session: session
                            .ok_or_else(|| "session is required".to_string())?
                            .to_string(),
                        tail: Some(if operation == "status" {
                            20
                        } else {
                            tail.unwrap_or(200)
                        }),
                    },
                )
                .await
        }
        "start" | "stop" | "restart" | "pause" | "resume" => {
            let node = if is_leader {
                node.ok_or_else(|| "node is required for targeted leader operations".to_string())?
            } else {
                "self"
            };
            state
                .dispatch_node(
                    node,
                    RemoteRequest::Action {
                        session: session
                            .ok_or_else(|| "session is required".to_string())?
                            .to_string(),
                        task,
                        action: match operation {
                            "start" => Action::Start,
                            "stop" => Action::Stop,
                            "restart" => Action::Restart,
                            "pause" => Action::Pause,
                            "resume" => Action::Resume,
                            _ => unreachable!(),
                        },
                    },
                )
                .await
        }
        _ => return Err(format!("unsupported action: {operation}")),
    };
    let text = serde_json::to_string_pretty(&response).map_err(|error| error.to_string())?;
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": response,
        "isError": !response.ok
    }))
}

const INDEX_HTML: &str = include_str!("web/index.html");
const STYLES_CSS: &str = include_str!("web/styles.css");
const APP_JS: &str = include_str!("web/app.js");

const FAVICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" role="img" aria-label="Taskdeck">
<rect width="32" height="32" rx="7" fill="#111315"/>
<path d="M8 9h16M8 16h10M8 23h7" fill="none" stroke="#56b6c2" stroke-width="3" stroke-linecap="round"/>
<circle cx="23" cy="23" r="3" fill="#51c878"/>
</svg>"##;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use axum::body::{Body, to_bytes};
    use axum::http::Request as HttpRequest;
    use serde_json::json;
    use tower::util::ServiceExt;

    use super::*;
    use crate::config::{ProjectDefinition, TaskSpec};
    use crate::runtime::SessionRuntime;

    #[test]
    fn mcp_exposes_one_control_tool() {
        let state = DaemonState::new();
        let tool = mcp_tool_definition(&state);
        assert_eq!(tool["name"], "taskdeck_control");
        assert_eq!(tool["inputSchema"]["required"][0], "action");
        assert!(tool["inputSchema"]["properties"].get("node").is_none());
    }

    #[test]
    fn leader_mcp_schema_exposes_cluster_targeting() {
        let state = DaemonState::new();
        let settings = state
            .store
            .configure(crate::state::NodeSettingsUpdate {
                role: Some(crate::state::NodeRole::Leader),
                ..crate::state::NodeSettingsUpdate::default()
            })
            .unwrap();
        *state.settings.lock().expect("node settings lock") = settings;
        let tool = mcp_tool_definition(&state);
        assert!(tool["inputSchema"]["properties"].get("node").is_some());
        assert!(
            tool["inputSchema"]["properties"]["action"]["enum"]
                .as_array()
                .unwrap()
                .contains(&json!("nodes"))
        );
    }

    #[test]
    fn page_uses_split_assets_and_exposes_workspace_controls() {
        assert!(INDEX_HTML.contains("/favicon.svg"));
        assert!(INDEX_HTML.contains("/assets/styles.css"));
        assert!(INDEX_HTML.contains("/assets/app.js"));
        assert!(INDEX_HTML.contains("data-view=\"docs\""));
        assert!(INDEX_HTML.contains("data-view=\"calls\""));
        assert!(INDEX_HTML.contains("id=\"config-dialog\""));
        assert!(!INDEX_HTML.contains("<style>"));
        assert!(!INDEX_HTML.contains("<script>const"));
        assert!(STYLES_CSS.contains("prefers-color-scheme: dark"));
        assert!(STYLES_CSS.contains("prefers-reduced-motion: reduce"));
        assert!(STYLES_CSS.contains("sidebar-collapsed"));
        assert!(APP_JS.contains("/api/mcp-calls"));
        assert!(APP_JS.contains("/config"));
        assert!(APP_JS.contains("/logs?"));
        assert!(INDEX_HTML.contains("id=\"nodes\""));
        assert!(APP_JS.contains("new URLSearchParams({ window: \"600\" })"));
        assert!(APP_JS.contains("requestFullscreen"));
        assert!(APP_JS.contains("taskdeck-log-tail"));
        assert!(FAVICON_SVG.contains("<svg"));
    }

    fn metrics_test_state() -> DaemonState {
        let state = DaemonState::new();
        state.sessions.lock().expect("sessions lock").insert(
            "demo".to_string(),
            SessionRuntime::new(ProjectDefinition {
                session: "demo".to_string(),
                project: PathBuf::from("/tmp"),
                source: "taskdeck.yaml".to_string(),
                tasks: BTreeMap::from([(
                    "api".to_string(),
                    TaskSpec {
                        label: "api".to_string(),
                        program: "sleep".to_string(),
                        args: vec!["60".to_string()],
                        cwd: PathBuf::from("/tmp"),
                        env: BTreeMap::new(),
                        shell: false,
                        auto_start: false,
                        stop_timeout_ms: 500,
                    },
                )]),
            }),
        );
        state
    }

    #[tokio::test]
    async fn task_logs_route_returns_incremental_payload_and_validates_queries() {
        let app = app(metrics_test_state());

        let response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/sessions/demo/tasks/api/logs?limit=100")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let response: Response = serde_json::from_slice(&body).unwrap();
        assert!(response.ok);
        assert!(
            response.data.as_ref().unwrap()["generation"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(response.data.as_ref().unwrap()["reset"], false);
        assert_eq!(response.data.as_ref().unwrap()["lines"], json!([]));

        for uri in [
            "/api/sessions/demo/tasks/api/logs?after=not-a-sequence",
            "/api/sessions/demo/tasks/api/logs?limit=0",
        ] {
            let response = app
                .clone()
                .oneshot(HttpRequest::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let response: Response = serde_json::from_slice(&body).unwrap();
            assert!(!response.ok);
            assert_eq!(response.data.as_ref().unwrap()["status"], 400);
        }
    }

    #[tokio::test]
    async fn task_metrics_route_clamps_valid_windows_and_envelopes_invalid_queries() {
        let app = app(metrics_test_state());

        let response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/sessions/demo/tasks/api/metrics?window=0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let response: Response = serde_json::from_slice(&body).unwrap();
        assert!(response.ok);
        assert_eq!(response.data.as_ref().unwrap()["window_seconds"], 1);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/sessions/demo/tasks/api/metrics?window=soon")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let response: Response = serde_json::from_slice(&body).unwrap();
        assert!(!response.ok);
        assert_eq!(response.data.as_ref().unwrap()["status"], 400);
    }

    fn mcp_call_test_state() -> DaemonState {
        let state = DaemonState::new();
        for call in [
            test_mcp_call(
                "taskdeck_control",
                Some("inspect"),
                true,
                10,
                json!({
                    "session": "alpha",
                    "task": "api",
                    "note": "Needle from input"
                }),
            ),
            test_mcp_call(
                "shell_exec",
                Some("run"),
                false,
                20,
                json!({
                    "session": "beta",
                    "task": "worker",
                    "command": "echo no-match"
                }),
            ),
            test_mcp_call(
                "report_writer",
                Some("export"),
                true,
                30,
                json!({
                    "session": "gamma",
                    "task": "etl",
                    "payload": {"mode": "FULL"}
                }),
            ),
        ] {
            state.record_mcp_call(call);
        }
        state
    }

    fn test_mcp_call(
        tool: &str,
        operation: Option<&str>,
        success: bool,
        started_at_ms: u64,
        arguments: Value,
    ) -> McpCallRecord {
        McpCallRecord {
            id: 0,
            tool: tool.to_string(),
            operation: operation.map(ToOwned::to_owned),
            started_at_ms,
            duration_ms: started_at_ms + 5,
            success,
            request: json!({
                "params": {
                    "arguments": arguments
                }
            }),
            response: json!({
                "result": {
                    "isError": !success
                }
            }),
        }
    }

    async fn list_mcp_calls_response(
        uri: &str,
        state: DaemonState,
    ) -> (Response, Option<McpCallListPage>) {
        let response = app(state)
            .oneshot(HttpRequest::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let response: Response = serde_json::from_slice(&body).unwrap();
        let page = response
            .data
            .clone()
            .and_then(|data| serde_json::from_value(data).ok());
        (response, page)
    }

    #[tokio::test]
    async fn mcp_calls_route_searches_across_targets_and_serialized_input() {
        let (_, tool_page) =
            list_mcp_calls_response("/api/mcp-calls?q=report", mcp_call_test_state()).await;
        assert_eq!(tool_page.unwrap().items[0].tool, "report_writer");

        let (_, session_page) =
            list_mcp_calls_response("/api/mcp-calls?q=beta", mcp_call_test_state()).await;
        assert_eq!(session_page.unwrap().items[0].input["session"], "beta");

        let (_, task_page) =
            list_mcp_calls_response("/api/mcp-calls?q=worker", mcp_call_test_state()).await;
        assert_eq!(task_page.unwrap().items[0].input["task"], "worker");

        let (_, input_page) =
            list_mcp_calls_response("/api/mcp-calls?q=needle", mcp_call_test_state()).await;
        assert_eq!(
            input_page.unwrap().items[0].input["note"],
            "Needle from input"
        );
    }

    #[tokio::test]
    async fn mcp_calls_route_uses_unicode_casefold_search() {
        let state = DaemonState::new();
        state.record_mcp_call(test_mcp_call(
            "taskdeck_control",
            Some("inspect"),
            true,
            40,
            json!({
                "session": "unicode-a",
                "task": "Straße",
                "note": "ignored"
            }),
        ));
        state.record_mcp_call(test_mcp_call(
            "taskdeck_control",
            Some("inspect"),
            true,
            41,
            json!({
                "session": "unicode-b",
                "task": "ος",
                "note": "sigma"
            }),
        ));

        let (_, strasse_page) =
            list_mcp_calls_response("/api/mcp-calls?q=STRASSE", state.clone()).await;
        let strasse_page = strasse_page.unwrap();
        assert_eq!(strasse_page.total, 1);
        assert_eq!(strasse_page.items[0].input["task"], "Straße");

        let (_, sigma_page) = list_mcp_calls_response("/api/mcp-calls?q=οσ", state).await;
        let sigma_page = sigma_page.unwrap();
        assert_eq!(sigma_page.total, 1);
        assert_eq!(sigma_page.items[0].input["task"], "ος");
    }

    #[tokio::test]
    async fn mcp_calls_route_does_not_search_response_payload_and_detail_keeps_full_record() {
        let state = DaemonState::new();
        state.record_mcp_call(McpCallRecord {
            id: 0,
            tool: "taskdeck_control".to_string(),
            operation: Some("inspect".to_string()),
            started_at_ms: 77,
            duration_ms: 9,
            success: true,
            request: json!({
                "id": 123,
                "params": {
                    "arguments": {
                        "session": "alpha",
                        "task": "api",
                        "note": "visible"
                    }
                }
            }),
            response: json!({
                "jsonrpc": "2.0",
                "id": 123,
                "result": {
                    "content": [{"type": "text", "text": "HiddenResponseNeedle"}]
                }
            }),
        });

        let app = app(state.clone());
        let list_response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/mcp-calls?q=HiddenResponseNeedle")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list_body = to_bytes(list_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let list_response: Response = serde_json::from_slice(&list_body).unwrap();
        let list_data = list_response.data.clone().unwrap();
        assert_eq!(list_data["total"], 0);

        let visible_response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/mcp-calls?q=visible")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let visible_body = to_bytes(visible_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let visible_response: Response = serde_json::from_slice(&visible_body).unwrap();
        let visible_data = visible_response.data.clone().unwrap();
        assert!(visible_data["items"][0].get("response").is_none());
        assert!(visible_data["items"][0].get("request").is_none());
        assert_eq!(visible_data["items"][0]["input"]["note"], "visible");

        let detail_response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/mcp-calls/1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let detail_body = to_bytes(detail_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let detail_response: Response = serde_json::from_slice(&detail_body).unwrap();
        let detail_data = detail_response.data.unwrap();
        assert_eq!(
            detail_data["response"]["result"]["content"][0]["text"],
            "HiddenResponseNeedle"
        );
        assert_eq!(
            detail_data["request"]["params"]["arguments"]["note"],
            "visible"
        );
    }

    #[tokio::test]
    async fn mcp_calls_route_applies_exact_and_combined_filters() {
        let (_, operation_page) =
            list_mcp_calls_response("/api/mcp-calls?operation=export", mcp_call_test_state()).await;
        assert_eq!(operation_page.unwrap().items.len(), 1);

        let (_, status_page) =
            list_mcp_calls_response("/api/mcp-calls?status=error", mcp_call_test_state()).await;
        let status_page = status_page.unwrap();
        assert_eq!(status_page.items.len(), 1);
        assert!(!status_page.items[0].success);

        let (_, session_page) =
            list_mcp_calls_response("/api/mcp-calls?session=alpha", mcp_call_test_state()).await;
        assert_eq!(session_page.unwrap().items[0].input["session"], "alpha");

        let (_, task_page) =
            list_mcp_calls_response("/api/mcp-calls?task=etl", mcp_call_test_state()).await;
        assert_eq!(task_page.unwrap().items[0].input["task"], "etl");

        let (_, combined_page) = list_mcp_calls_response(
            "/api/mcp-calls?status=success&operation=inspect&session=alpha&task=api&q=needle",
            mcp_call_test_state(),
        )
        .await;
        let combined_page = combined_page.unwrap();
        assert_eq!(combined_page.total, 1);
        assert_eq!(combined_page.items[0].tool, "taskdeck_control");
    }

    #[tokio::test]
    async fn mcp_calls_route_rejects_invalid_status_and_page_inputs() {
        let (status_response, _) =
            list_mcp_calls_response("/api/mcp-calls?status=maybe", mcp_call_test_state()).await;
        assert!(!status_response.ok);
        assert_eq!(status_response.data.as_ref().unwrap()["status"], 400);

        let (page_response, _) =
            list_mcp_calls_response("/api/mcp-calls?page=zero", mcp_call_test_state()).await;
        assert!(!page_response.ok);
        assert_eq!(page_response.data.as_ref().unwrap()["status"], 400);

        let (page_size_response, _) =
            list_mcp_calls_response("/api/mcp-calls?page_size=0", mcp_call_test_state()).await;
        assert!(!page_size_response.ok);
        assert_eq!(page_size_response.data.as_ref().unwrap()["status"], 400);
    }

    #[tokio::test]
    async fn mcp_calls_route_paginates_and_snaps_page_sizes() {
        let state = DaemonState::new();
        for index in 0..61 {
            state.record_mcp_call(test_mcp_call(
                "taskdeck_control",
                Some("inspect"),
                index % 2 == 0,
                index as u64,
                json!({
                    "session": format!("session-{index}"),
                    "task": format!("task-{index}")
                }),
            ));
        }

        let (_, first_page) =
            list_mcp_calls_response("/api/mcp-calls?page=1&page_size=35", state.clone()).await;
        let first_page = first_page.unwrap();
        assert_eq!(first_page.page_size, 20);
        assert_eq!(first_page.total, 61);
        assert_eq!(first_page.total_pages, 4);
        assert_eq!(first_page.items.len(), 20);
        assert_eq!(first_page.items[0].started_at_ms, 60);
        assert_eq!(first_page.items[19].started_at_ms, 41);
        assert!(first_page.has_next);
        assert!(!first_page.has_previous);

        let (_, second_page) =
            list_mcp_calls_response("/api/mcp-calls?page=2&page_size=75", state.clone()).await;
        let second_page = second_page.unwrap();
        assert_eq!(second_page.page_size, 50);
        assert_eq!(second_page.items.len(), 11);
        assert_eq!(second_page.items[0].started_at_ms, 10);
        assert_eq!(second_page.items[10].started_at_ms, 0);
        assert!(!second_page.has_next);
        assert!(second_page.has_previous);

        let (_, empty_page) =
            list_mcp_calls_response("/api/mcp-calls?page=5&page_size=20", state).await;
        let empty_page = empty_page.unwrap();
        assert!(empty_page.items.is_empty());
        assert_eq!(empty_page.page, 5);
        assert_eq!(empty_page.total, 61);
        assert_eq!(empty_page.total_pages, 4);
        assert!(!empty_page.has_next);
        assert!(empty_page.has_previous);
    }

    #[tokio::test]
    async fn mcp_calls_route_defaults_and_keeps_newest_first_order() {
        let (_, page) = list_mcp_calls_response("/api/mcp-calls", mcp_call_test_state()).await;
        let page = page.unwrap();
        assert_eq!(page.page, 1);
        assert_eq!(page.page_size, 20);
        assert_eq!(page.total, 3);
        assert_eq!(page.total_pages, 1);
        assert_eq!(page.items.len(), 3);
        assert_eq!(
            page.items
                .iter()
                .map(|item| item.started_at_ms)
                .collect::<Vec<_>>(),
            vec![30, 20, 10]
        );
    }
}
