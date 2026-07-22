use std::collections::HashMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response as AxumResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::daemon::{DaemonState, dispatch_async};
use crate::protocol::{Action, McpCallRecord, Request, Response};

pub async fn serve(state: DaemonState, listener: tokio::net::TcpListener) -> Result<()> {
    let router = Router::new()
        .route("/", get(index))
        .route("/favicon.svg", get(favicon))
        .route("/favicon.ico", get(favicon))
        .route("/healthz", get(health))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{session}", get(session_snapshot))
        .route("/api/mcp-calls", get(list_mcp_calls))
        .route("/api/mcp-calls/{id}", get(mcp_call_detail))
        .route("/api/action", post(action))
        .route("/mcp", post(mcp))
        .with_state(state);
    axum::serve(listener, router)
        .await
        .context("Web server failed")
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn health() -> StatusCode {
    StatusCode::OK
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

async fn list_sessions(State(state): State<DaemonState>) -> Json<Response> {
    Json(dispatch_async(state, Request::ListSessions).await)
}

async fn session_snapshot(
    State(state): State<DaemonState>,
    Path(session): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Response> {
    let tail = query.get("tail").and_then(|value| value.parse().ok());
    Json(dispatch_async(state, Request::Snapshot { session, tail }).await)
}

#[derive(Deserialize)]
struct ActionBody {
    session: String,
    task: Option<String>,
    action: Action,
}

async fn action(State(state): State<DaemonState>, Json(body): Json<ActionBody>) -> Json<Response> {
    Json(
        dispatch_async(
            state,
            Request::Action {
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
    let limit = query
        .get("limit")
        .and_then(|value| value.parse().ok())
        .unwrap_or(200);
    let calls = state
        .recent_mcp_calls(limit)
        .into_iter()
        .map(|call| {
            json!({
                "id": call.id,
                "tool": call.tool,
                "operation": call.operation,
                "started_at_ms": call.started_at_ms,
                "duration_ms": call.duration_ms,
                "success": call.success,
                "input": call.request.pointer("/params/arguments").cloned().unwrap_or(Value::Null)
            })
        })
        .collect::<Vec<_>>();
    Json(Response::ok("MCP calls", calls))
}

async fn mcp_call_detail(State(state): State<DaemonState>, Path(id): Path<u64>) -> Json<Response> {
    match state.mcp_call(id) {
        Some(call) => Json(Response::ok("MCP call", call)),
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
            "instructions": "Use taskdeck_control to inspect and control tasks by global session name."
        }),
        "ping" => json!({}),
        "tools/list" => json!({"tools": [mcp_tool_definition()]}),
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

fn mcp_tool_definition() -> Value {
    json!({
        "name": "taskdeck_control",
        "description": "List Taskdeck sessions, inspect task status/logs, or control one task/all tasks in a session.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["sessions", "status", "logs", "start", "stop", "restart", "pause", "resume"],
                    "description": "Operation to perform."
                },
                "session": {"type": "string", "description": "Global session name; required except for sessions."},
                "task": {"type": "string", "description": "Task label. Omit for all tasks or a full session snapshot."},
                "tail": {"type": "integer", "minimum": 1, "maximum": 5000, "default": 200}
            },
            "required": ["action"],
            "additionalProperties": false
        }
    })
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
    let session = arguments.get("session").and_then(Value::as_str);
    let task = arguments
        .get("task")
        .and_then(Value::as_str)
        .map(str::to_string);
    let tail = arguments
        .get("tail")
        .and_then(Value::as_u64)
        .map(|v| v as usize);
    let request = match operation {
        "sessions" => Request::ListSessions,
        "status" | "logs" => Request::Snapshot {
            session: session
                .ok_or_else(|| "session is required".to_string())?
                .to_string(),
            tail: Some(if operation == "status" {
                20
            } else {
                tail.unwrap_or(200)
            }),
        },
        "start" | "stop" | "restart" | "pause" | "resume" => Request::Action {
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
        _ => return Err(format!("unsupported action: {operation}")),
    };
    let response = dispatch_async(state, request).await;
    let text = serde_json::to_string_pretty(&response).map_err(|error| error.to_string())?;
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": response,
        "isError": !response.ok
    }))
}

const INDEX_HTML: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="color-scheme" content="light dark"><link rel="icon" href="/favicon.svg" type="image/svg+xml">
<title>Taskdeck</title><style>
:root{color-scheme:light;--background:#fff;--foreground:#292929;--card:#fff;--secondary:#f6f6f6;--muted:#f4f4f4;--muted-foreground:#797979;--accent:#f0f0f0;--border:#e9e9e9;--ring:#7c9dff;--success:#05834e;--warning:#d96c00;--destructive:#d73220;--console:#151515;--console-text:#ececec;--console-muted:#929292;--sidebar:rgba(250,250,250,.84);--radius:8px;--shadow:0 12px 36px rgba(0,0,0,.08);--font-sans:"SF Pro Text",system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;--font-mono:ui-monospace,SFMono-Regular,Menlo,Monaco,Consolas,monospace}
:root[data-theme=dark]{color-scheme:dark;--background:#111;--foreground:#ebebeb;--card:#1b1b1b;--secondary:#222;--muted:#222;--muted-foreground:#a7a7a7;--accent:#393939;--border:#393939;--success:#3ac98a;--warning:#eda844;--destructive:#ef6253;--console:#090909;--console-text:#ededed;--console-muted:#858585;--sidebar:rgba(24,24,24,.86);--shadow:0 16px 42px rgba(0,0,0,.3)}
@media(prefers-color-scheme:dark){:root:not([data-theme=light]){color-scheme:dark;--background:#111;--foreground:#ebebeb;--card:#1b1b1b;--secondary:#222;--muted:#222;--muted-foreground:#a7a7a7;--accent:#393939;--border:#393939;--success:#3ac98a;--warning:#eda844;--destructive:#ef6253;--console:#090909;--console-text:#ededed;--console-muted:#858585;--sidebar:rgba(24,24,24,.86);--shadow:0 16px 42px rgba(0,0,0,.3)}}
*{box-sizing:border-box;scrollbar-width:thin;scrollbar-color:color-mix(in oklab,var(--muted-foreground) 36%,transparent) transparent}html,body{height:100%}body{margin:0;background:var(--background);color:var(--foreground);font:14px/1.5 var(--font-sans);-webkit-font-smoothing:antialiased}button,select{font:inherit;color:inherit}button{cursor:pointer}button:focus-visible,select:focus-visible{outline:2px solid color-mix(in oklab,var(--ring) 72%,transparent);outline-offset:2px}.shell{height:100%;display:grid;grid-template-columns:248px minmax(0,1fr);overflow:hidden}.sidebar{position:relative;z-index:3;display:flex;min-height:0;flex-direction:column;border-right:1px solid var(--border);background:var(--sidebar);padding:18px 12px 12px;backdrop-filter:blur(24px) saturate(160%)}.brand{display:flex;align-items:center;gap:10px;height:34px;padding:0 8px;margin-bottom:20px}.brand-mark{display:grid;width:27px;height:27px;place-items:center;border-radius:7px;background:var(--foreground);color:var(--background);box-shadow:inset 0 0 0 1px color-mix(in oklab,var(--background) 16%,transparent)}.brand-mark svg{width:16px}.brand strong{font-size:15px;font-weight:650;letter-spacing:-.015em}.brand small{display:block;color:var(--muted-foreground);font-size:10px;line-height:1.1;letter-spacing:.08em;text-transform:uppercase}.nav-label{margin:6px 10px;color:var(--muted-foreground);font-size:11px;font-weight:600;letter-spacing:.06em;text-transform:uppercase}.nav{display:grid;gap:3px}.nav-button{display:flex;align-items:center;gap:9px;width:100%;height:36px;border:0;border-radius:7px;background:transparent;padding:0 10px;text-align:left;font-weight:520}.nav-button svg,.icon-button svg,.button svg{width:16px;height:16px;flex:0 0 auto}.nav-button:hover{background:var(--secondary)}.nav-button.active{background:var(--accent);font-weight:620}.nav-button:active,.button:active,.icon-button:active,.tab:active{transform:scale(.975)}.nav-spacer{flex:1}.connection{display:flex;align-items:center;gap:9px;margin:10px 5px 2px;padding:10px 7px;color:var(--muted-foreground);font-size:12px}.connection-dot{width:7px;height:7px;border-radius:99px;background:var(--success);box-shadow:0 0 0 4px color-mix(in oklab,var(--success) 12%,transparent)}.main{min-width:0;min-height:0;display:grid;grid-template-rows:60px 1fr;background:var(--background)}.topbar{position:relative;z-index:2;display:flex;align-items:center;gap:12px;padding:0 28px;border-bottom:1px solid var(--border);background:color-mix(in oklab,var(--background) 88%,transparent);backdrop-filter:blur(18px) saturate(150%)}.topbar-title{min-width:0;margin-right:auto}.topbar-title strong{display:block;font-size:15px;font-weight:630;letter-spacing:-.01em}.topbar-title span{display:block;max-width:min(58vw,720px);overflow:hidden;color:var(--muted-foreground);font-size:11px;text-overflow:ellipsis;white-space:nowrap}.session-select{max-width:220px;height:34px;border:1px solid var(--border);border-radius:7px;background:var(--card);padding:0 30px 0 10px}.icon-button{display:grid;width:34px;height:34px;place-items:center;border:1px solid var(--border);border-radius:7px;background:var(--card)}.icon-button:hover{background:var(--secondary)}.view{min-height:0;overflow:auto}.view[hidden]{display:none}.workspace-view{display:grid;grid-template-rows:auto minmax(0,1fr);height:100%;min-height:0}.task-tabs{display:flex;gap:3px;overflow:auto;padding:14px 28px 0;border-bottom:1px solid var(--border)}.tab{position:relative;height:37px;border:0;background:transparent;padding:0 11px;color:var(--muted-foreground);white-space:nowrap}.tab:hover{color:var(--foreground)}.tab.active{color:var(--foreground);font-weight:600}.tab.active:after{position:absolute;right:10px;bottom:-1px;left:10px;height:2px;border-radius:2px 2px 0 0;background:var(--foreground);content:""}.task-pane{display:grid;grid-template-rows:auto minmax(0,1fr);min-height:0;padding:24px 28px 28px;gap:14px}.task-header{display:flex;align-items:center;gap:16px;min-width:0}.task-heading{min-width:0;margin-right:auto}.eyebrow{margin:0 0 2px;color:var(--muted-foreground);font-size:11px;font-weight:620;letter-spacing:.06em;text-transform:uppercase}.task-heading h1{overflow:hidden;margin:0;font-size:22px;line-height:1.2;font-weight:660;letter-spacing:-.025em;text-overflow:ellipsis;white-space:nowrap}.task-detail{display:flex;gap:8px;align-items:center;margin-top:5px;color:var(--muted-foreground);font-size:12px}.status{display:inline-flex;align-items:center;gap:6px;font-weight:620}.status:before{width:7px;height:7px;border-radius:99px;background:currentColor;content:""}.status.running{color:var(--success)}.status.paused{color:var(--warning)}.status.failed{color:var(--destructive)}.actions{display:flex;gap:7px;align-items:center}.button{display:inline-flex;height:34px;align-items:center;justify-content:center;gap:7px;border:1px solid var(--border);border-radius:7px;background:var(--card);padding:0 11px;font-weight:560;transition:background-color .15s,border-color .15s,transform .1s,opacity .15s}.button:hover{background:var(--secondary)}.button.primary{border-color:var(--foreground);background:var(--foreground);color:var(--background)}.button.danger{color:var(--destructive)}.button:disabled{cursor:default;opacity:.38}.console{min-height:260px;overflow:hidden;border:1px solid #252525;border-radius:var(--radius);background:var(--console);box-shadow:var(--shadow);color:var(--console-text)}.console-bar{display:flex;height:40px;align-items:center;gap:8px;padding:0 14px;border-bottom:1px solid #292929;color:var(--console-muted);font-size:11px}.traffic-lights{display:flex;gap:6px;margin-right:3px}.traffic-lights i{display:block;width:8px;height:8px;border-radius:99px;background:#424242}.traffic-lights i:first-child{background:#ff645b}.traffic-lights i:nth-child(2){background:#ffbe42}.traffic-lights i:last-child{background:#35c84a}.console-bar span:last-child{margin-left:auto}.logs{height:calc(100% - 40px);margin:0;overflow:auto;padding:17px 18px 24px;font:12px/1.65 var(--font-mono);white-space:pre-wrap;word-break:break-word}.logs .stderr{color:#ff7a70}.logs .system{color:#78b7ff}.empty{color:var(--muted-foreground)}.empty-state{display:grid;min-height:100%;place-items:center;padding:48px;text-align:center}.empty-state svg{width:30px;color:var(--muted-foreground)}.empty-state h2{margin:14px 0 4px;font-size:18px;letter-spacing:-.02em}.empty-state p{max-width:380px;margin:0;color:var(--muted-foreground)}
.docs{max-width:1040px;margin:0 auto;padding:48px 48px 80px}.docs-hero{display:grid;grid-template-columns:minmax(0,1fr) auto;gap:32px;align-items:end;padding-bottom:36px;border-bottom:1px solid var(--border)}.docs-hero h1{max-width:680px;margin:5px 0 12px;font-size:36px;line-height:1.08;font-weight:680;letter-spacing:-.04em}.docs-hero p{max-width:640px;margin:0;color:var(--muted-foreground);font-size:15px;line-height:1.65}.endpoint-badge{display:inline-flex;align-items:center;gap:8px;border:1px solid color-mix(in oklab,var(--success) 28%,var(--border));border-radius:99px;background:color-mix(in oklab,var(--success) 7%,var(--background));padding:6px 10px;color:var(--success);font:11px var(--font-mono)}.docs-section{display:grid;grid-template-columns:200px minmax(0,1fr);gap:44px;padding:36px 0;border-bottom:1px solid var(--border)}.docs-section:last-child{border:0}.docs-section>header h2{margin:0 0 5px;font-size:15px;letter-spacing:-.01em}.docs-section>header p{margin:0;color:var(--muted-foreground);font-size:12px}.docs-content h3{margin:0 0 7px;font-size:14px}.docs-content>p{margin:0 0 18px;color:var(--muted-foreground)}.code-block{position:relative;overflow:auto;margin:14px 0 20px;border:1px solid #292929;border-radius:var(--radius);background:var(--console);color:var(--console-text);padding:17px 54px 17px 18px;font:12px/1.65 var(--font-mono);white-space:pre}.copy{position:absolute;top:8px;right:8px;width:30px;height:30px;border:1px solid #333;border-radius:6px;background:#202020;color:#bcbcbc}.copy:hover{background:#2a2a2a;color:#fff}.operation-list{border-top:1px solid var(--border)}.operation{display:grid;grid-template-columns:120px minmax(0,1fr);gap:18px;padding:14px 0;border-bottom:1px solid var(--border)}.operation code,.inline-code{border-radius:5px;background:var(--secondary);padding:3px 6px;font:12px var(--font-mono)}.operation p{margin:0;color:var(--muted-foreground);font-size:13px}.note{display:flex;gap:10px;border-radius:var(--radius);background:var(--secondary);padding:14px 15px;color:var(--muted-foreground);font-size:12px}.note svg{width:16px;flex:0 0 auto;color:var(--foreground)}.parameter-table{width:100%;border-collapse:collapse;font-size:12px}.parameter-table th{text-align:left;color:var(--muted-foreground);font-weight:560}.parameter-table th,.parameter-table td{padding:10px 8px;border-bottom:1px solid var(--border)}.parameter-table th:first-child,.parameter-table td:first-child{padding-left:0}.toast{position:fixed;right:22px;bottom:22px;z-index:10;display:flex;align-items:center;gap:8px;border:1px solid var(--border);border-radius:8px;background:var(--card);padding:10px 13px;box-shadow:var(--shadow);font-size:12px;opacity:0;transform:translateY(10px);pointer-events:none}.toast.visible{opacity:1;transform:translateY(0)}
.mcp-calls{display:grid;grid-template-rows:auto minmax(0,1fr);height:100%;min-height:0;padding:26px 28px 28px;gap:16px}.calls-heading{display:flex;align-items:end;gap:16px}.calls-heading>div{margin-right:auto}.calls-heading h1{margin:0;font-size:22px;line-height:1.2;font-weight:660;letter-spacing:-.025em}.calls-heading p{margin:5px 0 0;color:var(--muted-foreground);font-size:12px}.calls-table-wrap{min-height:0;overflow:auto;border:1px solid var(--border);border-radius:var(--radius);background:var(--card)}.calls-table{width:100%;min-width:880px;border-collapse:collapse;table-layout:fixed}.calls-table th{position:sticky;top:0;z-index:1;background:color-mix(in oklab,var(--card) 94%,transparent);color:var(--muted-foreground);font-size:11px;font-weight:620;letter-spacing:.04em;text-align:left;text-transform:uppercase;backdrop-filter:blur(16px)}.calls-table th,.calls-table td{height:48px;padding:0 12px;border-bottom:1px solid var(--border);vertical-align:middle}.calls-table tbody tr:last-child td{border-bottom:0}.calls-table tbody tr:hover{background:var(--secondary)}.calls-table th:nth-child(1){width:22%}.calls-table th:nth-child(2){width:12%}.calls-table th:nth-child(3){width:11%}.calls-table th:nth-child(4){width:18%}.calls-table th:nth-child(5){width:10%}.calls-table th:nth-child(7){width:86px}.call-tool{overflow:hidden;font:12px var(--font-mono);text-overflow:ellipsis;white-space:nowrap}.call-operation,.request-preview{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.request-preview{color:var(--muted-foreground);font:11px var(--font-mono)}.call-time,.call-duration{color:var(--muted-foreground);font-size:12px;white-space:nowrap}.status-pill{display:inline-flex;align-items:center;gap:6px;border:1px solid color-mix(in oklab,currentColor 24%,var(--border));border-radius:99px;padding:3px 8px;font-size:11px;font-weight:650}.status-pill:before{width:6px;height:6px;border-radius:50%;background:currentColor;content:""}.status-pill.success,.status-success{color:var(--success)}.status-pill.error,.status-error{color:var(--destructive)}.detail-button{height:28px;border:1px solid var(--border);border-radius:6px;background:var(--card);padding:0 9px;font-size:11px;font-weight:600}.detail-button:hover{background:var(--accent)}.calls-empty td{height:220px!important;color:var(--muted-foreground);text-align:center}.call-dialog{width:min(960px,calc(100vw - 32px));max-height:min(820px,calc(100vh - 32px));overflow:hidden;border:1px solid var(--border);border-radius:12px;background:var(--background);color:var(--foreground);padding:0;box-shadow:0 28px 90px rgba(0,0,0,.24)}.call-dialog::backdrop{background:rgba(0,0,0,.34);backdrop-filter:blur(3px)}.dialog-shell{display:grid;grid-template-rows:auto minmax(0,1fr);max-height:min(820px,calc(100vh - 32px))}.dialog-header{display:flex;align-items:center;gap:14px;padding:18px 20px;border-bottom:1px solid var(--border)}.dialog-title{min-width:0;margin-right:auto}.dialog-title h2{overflow:hidden;margin:0;font-size:16px;font-weight:650;text-overflow:ellipsis;white-space:nowrap}.dialog-title p{margin:3px 0 0;color:var(--muted-foreground);font-size:11px}.dialog-close{display:grid;width:32px;height:32px;place-items:center;border:1px solid var(--border);border-radius:7px;background:var(--card)}.dialog-body{overflow:auto;padding:20px}.call-meta{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));margin-bottom:20px;border-block:1px solid var(--border)}.call-meta div{padding:12px 0}.call-meta span{display:block;color:var(--muted-foreground);font-size:10px;font-weight:620;letter-spacing:.05em;text-transform:uppercase}.call-meta strong{display:block;overflow:hidden;margin-top:3px;font-size:12px;font-weight:600;text-overflow:ellipsis;white-space:nowrap}.json-grid{display:grid;grid-template-columns:1fr 1fr;gap:14px}.json-panel{min-width:0}.json-panel h3{margin:0 0 7px;font-size:12px;font-weight:620}.json-panel pre{height:410px;margin:0;overflow:auto;border:1px solid #292929;border-radius:8px;background:var(--console);color:var(--console-text);padding:15px;font:11px/1.6 var(--font-mono);white-space:pre-wrap;word-break:break-word}
.calls-table{min-width:760px}.calls-table th:nth-child(1){width:26%}.calls-table th:nth-child(2){width:28%}.calls-table th:nth-child(3){width:12%}.calls-table th:nth-child(4){width:20%}.calls-table th:nth-child(5){width:10%}.calls-table th:nth-child(6){width:86px}.operation-cell strong,.operation-cell span{display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.operation-cell strong{font-size:13px;font-weight:620}.operation-cell span{margin-top:2px;color:var(--muted-foreground);font:10px var(--font-mono)}.target-cell{overflow:hidden}.target-cell strong,.target-cell span{display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.target-cell strong{font:12px var(--font-mono);font-weight:560}.target-cell span{margin-top:2px;color:var(--muted-foreground);font-size:10px}.call-dialog{width:min(560px,100vw);height:100dvh;max-height:none;margin:0 0 0 auto;border-block:0;border-right:0;border-radius:0;box-shadow:-24px 0 70px rgba(0,0,0,.18)}.dialog-shell{height:100dvh;max-height:none}.dialog-header{padding:22px 24px;background:var(--background)}.dialog-title p{margin:0 0 3px;font:10px var(--font-mono);letter-spacing:.02em}.dialog-title h2{font-size:18px;letter-spacing:-.02em}.detail-status-icon{display:grid;width:38px;height:38px;flex:0 0 auto;place-items:center;border-radius:50%;background:color-mix(in oklab,var(--success) 12%,var(--background));color:var(--success)}.detail-status-icon.error{background:color-mix(in oklab,var(--destructive) 11%,var(--background));color:var(--destructive)}.detail-status-icon svg{width:19px;height:19px}.dialog-body{padding:0 24px 40px}.call-overview{display:grid;grid-template-columns:1.4fr .8fr .8fr;padding:18px 0;border-bottom:1px solid var(--border)}.overview-item span{display:block;color:var(--muted-foreground);font-size:10px;font-weight:620;letter-spacing:.05em;text-transform:uppercase}.overview-item strong{display:block;overflow:hidden;margin-top:4px;font-size:12px;font-weight:600;text-overflow:ellipsis;white-space:nowrap}.detail-section{padding-top:28px}.detail-section>header{margin-bottom:12px}.detail-section>header span{color:var(--muted-foreground);font-size:10px;font-weight:650;letter-spacing:.07em;text-transform:uppercase}.detail-section>header h3{margin:2px 0 0;font-size:15px;font-weight:650;letter-spacing:-.01em}.field-list{border-top:1px solid var(--border)}.field-row{display:grid;grid-template-columns:126px minmax(0,1fr);gap:16px;align-items:start;padding:12px 0;border-bottom:1px solid var(--border)}.field-label{color:var(--muted-foreground);font-size:12px}.field-value{overflow-wrap:anywhere;font-size:12px;font-weight:560}.field-value.mono{font-family:var(--font-mono);font-size:11px}.outcome{position:relative;padding:15px 16px 15px 46px;border-radius:8px;background:var(--secondary)}.outcome-icon{position:absolute;top:16px;left:16px;display:grid;width:20px;height:20px;place-items:center;border-radius:50%;background:var(--success);color:#fff;font-size:12px;font-weight:700}.outcome.error .outcome-icon{background:var(--destructive)}.outcome strong{display:block;font-size:13px}.outcome p{margin:3px 0 0;color:var(--muted-foreground);font-size:12px;line-height:1.55}.result-data{margin-top:18px}.result-empty{padding:18px 0;color:var(--muted-foreground);font-size:12px}.value-list{display:grid;border-top:1px solid var(--border)}.value-list-item{padding:11px 0;border-bottom:1px solid var(--border);font:12px var(--font-mono)}.session-facts{margin-top:18px;border-top:1px solid var(--border)}.task-result{padding:16px 0;border-bottom:1px solid var(--border)}.task-result-header{display:flex;align-items:center;gap:9px}.task-result-header strong{min-width:0;overflow:hidden;margin-right:auto;font-size:13px;text-overflow:ellipsis;white-space:nowrap}.task-result-meta{display:grid;gap:4px;margin-top:9px;color:var(--muted-foreground);font-size:11px}.task-result-meta span{overflow-wrap:anywhere}.log-disclosure{margin-top:10px}.log-disclosure summary{cursor:pointer;color:var(--muted-foreground);font-size:11px}.human-logs{max-height:220px;overflow:auto;margin-top:8px;border-radius:7px;background:var(--console);padding:10px 12px;color:var(--console-text);font:10px/1.65 var(--font-mono)}.human-log{display:grid;grid-template-columns:52px minmax(0,1fr);gap:8px}.human-log span:first-child{color:var(--console-muted)}
@media(max-width:860px){.shell{grid-template-columns:76px minmax(0,1fr)}.sidebar{align-items:center;padding-inline:10px}.brand{padding:0;margin-bottom:18px}.brand>div:last-child,.nav-label,.nav-button span,.connection span{display:none}.nav{width:100%}.nav-button{justify-content:center;padding:0}.connection{padding:10px}.docs{padding-inline:28px}.docs-section{grid-template-columns:150px;gap:28px}.call-meta{grid-template-columns:1fr 1fr}.json-grid{grid-template-columns:1fr}.json-panel pre{height:300px}}
@media(max-width:620px){.shell{display:block;overflow:auto}.sidebar{position:sticky;top:0;z-index:5;height:54px;min-height:54px;flex-direction:row;border-right:0;border-bottom:1px solid var(--border);padding:8px 12px}.brand{height:auto;margin:0 8px 0 0}.nav-label,.nav-spacer,.connection{display:none}.nav{display:flex;width:auto}.nav-button{width:38px}.main{display:block}.topbar{position:sticky;top:54px;height:56px;padding:0 14px}.topbar:has(.session-select[hidden]){position:static;height:0;padding:0;border:0}.topbar:has(.session-select[hidden]) .icon-button{position:fixed;top:10px;right:12px;z-index:6}.topbar-title{display:none}.session-select{min-width:0;flex:1}.view{overflow:visible}.workspace-view{height:calc(100svh - 110px)}.task-tabs{padding-inline:14px}.task-pane{padding:20px 14px;gap:12px}.task-header{align-items:flex-start;flex-direction:column}.actions{width:100%;overflow:auto;padding-bottom:2px}.task-heading h1{font-size:19px}.console{min-height:360px}.mcp-calls{height:calc(100svh - 54px);padding:20px 14px}.calls-heading{align-items:flex-start;flex-direction:column}.docs{padding:32px 18px 56px}.docs-hero{display:block}.docs-hero h1{font-size:30px}.endpoint-badge{margin-top:20px}.docs-section{display:block;padding:28px 0}.docs-section>header{margin-bottom:20px}.operation{grid-template-columns:92px 1fr}.call-dialog{width:calc(100vw - 16px);max-height:calc(100vh - 16px)}.dialog-shell{max-height:calc(100vh - 16px)}.dialog-body{padding:16px}.call-meta{grid-template-columns:1fr 1fr}.json-panel pre{height:260px}}
@media(prefers-reduced-motion:no-preference){.view.active{animation:materialize .28s cubic-bezier(.2,.8,.2,1)}.toast{transition:opacity .18s,transform .28s cubic-bezier(.2,.8,.2,1)}@keyframes materialize{from{opacity:.25;transform:translateY(5px);filter:blur(3px)}to{opacity:1;transform:none;filter:none}}}
@media(prefers-reduced-motion:reduce){*,*:before,*:after{scroll-behavior:auto!important;animation-duration:.01ms!important;animation-iteration-count:1!important;transition-duration:.01ms!important}}
@media(prefers-reduced-transparency:reduce){.sidebar,.topbar{background:var(--background);backdrop-filter:none}}
@media(prefers-contrast:more){.sidebar,.topbar,.console,.button,.icon-button,.session-select{border-color:var(--foreground)}}
.status-pill.paused{color:var(--warning)}.call-dialog{max-width:none}
@media(max-width:620px){.call-dialog{width:100vw;height:100dvh;max-height:none}.dialog-shell{height:100dvh;max-height:none}.dialog-body{padding:0 18px 32px}.call-overview{grid-template-columns:1.4fr .7fr .7fr}.field-row{grid-template-columns:100px minmax(0,1fr)}}
</style></head><body><div class="shell">
<aside class="sidebar"><div class="brand"><div class="brand-mark" aria-hidden="true"><svg viewBox="0 0 20 20" fill="none"><path d="M4 5h12M4 10h8M4 15h5" stroke="currentColor" stroke-width="2" stroke-linecap="round"/><circle cx="15" cy="15" r="2" fill="currentColor"/></svg></div><div><strong>Taskdeck</strong><small>Control plane</small></div></div><div class="nav-label">Workspace</div><nav class="nav" aria-label="Primary"><button class="nav-button active" data-view="tasks" aria-label="Tasks"><svg viewBox="0 0 24 24" fill="none"><path d="M5 7h14M5 12h14M5 17h9" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg><span>Tasks</span></button><button class="nav-button" data-view="calls" aria-label="MCP Calls"><svg viewBox="0 0 24 24" fill="none"><path d="M4 13h3l2-6 4 11 2-5h5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg><span>MCP Calls</span></button><button class="nav-button" data-view="docs" aria-label="MCP Guide"><svg viewBox="0 0 24 24" fill="none"><path d="M7 4.5h8l3 3V19a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V5.5a1 1 0 0 1 1-1Z" stroke="currentColor" stroke-width="1.7"/><path d="M14.5 4.5V8h3.5M9 12h6M9 15.5h4" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg><span>MCP Guide</span></button></nav><div class="nav-spacer"></div><div class="connection"><i class="connection-dot"></i><span>Daemon connected</span></div></aside>
<main class="main"><header class="topbar"><div class="topbar-title"><strong id="page-title">Task workspace</strong><span id="meta">No sessions registered</span></div><select class="session-select" id="sessions" aria-label="Session"><option>No sessions</option></select><button class="icon-button" id="theme" aria-label="Change color theme" title="Change color theme"><svg viewBox="0 0 24 24" fill="none"><path d="M12 3v2M12 19v2M3 12h2M19 12h2M5.64 5.64l1.42 1.42M16.94 16.94l1.42 1.42M18.36 5.64l-1.42 1.42M7.06 16.94l-1.42 1.42" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/><circle cx="12" cy="12" r="4" stroke="currentColor" stroke-width="1.7"/></svg></button></header>
<section class="view active" id="tasks-view"><div class="workspace-view"><div class="task-tabs" id="tabs"></div><section class="task-pane" id="task-pane"><div class="empty-state"><div><svg viewBox="0 0 24 24" fill="none"><path d="M4 6h16M4 12h12M4 18h8" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg><h2>No active workspace</h2><p>Register a project from the CLI or TUI. Its tasks and live output will appear here automatically.</p></div></div></section></div></section>
<section class="view" id="calls-view" hidden><div class="mcp-calls"><header class="calls-heading"><div><h1>MCP calls</h1><p>Review what agents asked Taskdeck to do and how each request completed.</p></div><button class="button" id="refresh-calls"><svg viewBox="0 0 24 24" fill="none"><path d="M19 8V4m0 0h-4m4 0-3 3a7 7 0 1 0 2 8" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/></svg>Refresh</button></header><div class="calls-table-wrap"><table class="calls-table"><thead><tr><th>Operation</th><th>Target</th><th>Result</th><th>Started</th><th>Duration</th><th></th></tr></thead><tbody id="calls-body"><tr class="calls-empty"><td colspan="6">No MCP tool calls recorded yet.</td></tr></tbody></table></div></div></section>
<section class="view" id="docs-view" hidden><article class="docs"><header class="docs-hero"><div><div class="eyebrow">Streamable HTTP</div><h1>Operate Taskdeck through MCP.</h1><p>Connect an agent to the same persistent tasks you use from the CLI, TUI, and Web UI. One focused tool covers discovery, inspection, logs, and lifecycle control.</p></div><div class="endpoint-badge"><i class="connection-dot"></i><span id="endpoint-label">127.0.0.1:9837/mcp</span></div></header>
<section class="docs-section"><header><h2>Connect</h2><p>Add the local server to any HTTP-capable MCP client.</p></header><div class="docs-content"><h3>Client configuration</h3><p>The endpoint is bound locally and requires no credentials.</p><div class="code-block" data-copy><button class="copy" aria-label="Copy configuration" title="Copy"><svg viewBox="0 0 24 24" width="15" fill="none"><rect x="8" y="8" width="11" height="11" rx="2" stroke="currentColor" stroke-width="1.7"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" stroke="currentColor" stroke-width="1.7"/></svg></button><code id="client-config">{
  "mcpServers": {
    "taskdeck": {
      "type": "http",
      "url": "http://127.0.0.1:9837/mcp"
    }
  }
}</code></div><div class="note"><svg viewBox="0 0 24 24" fill="none"><circle cx="12" cy="12" r="9" stroke="currentColor" stroke-width="1.7"/><path d="M12 11v5M12 8v.01" stroke="currentColor" stroke-width="1.9" stroke-linecap="round"/></svg><span>Taskdeck has no authentication layer. Keep the daemon on a trusted machine and do not publish this endpoint through a public proxy.</span></div></div></section>
<section class="docs-section"><header><h2>Tool contract</h2><p><code class="inline-code">taskdeck_control</code> is the only exposed tool.</p></header><div class="docs-content"><table class="parameter-table"><thead><tr><th>Parameter</th><th>Required</th><th>Purpose</th></tr></thead><tbody><tr><td><code class="inline-code">action</code></td><td>Always</td><td>Operation to perform.</td></tr><tr><td><code class="inline-code">session</code></td><td>Except sessions</td><td>Global session name.</td></tr><tr><td><code class="inline-code">task</code></td><td>Optional</td><td>Target task; omit to address the full session.</td></tr><tr><td><code class="inline-code">tail</code></td><td>Optional</td><td>Log lines to return, from 1 to 5000.</td></tr></tbody></table></div></section>
<section class="docs-section"><header><h2>Operations</h2><p>Read state first, then make the smallest targeted change.</p></header><div class="docs-content"><div class="operation-list"><div class="operation"><code>sessions</code><p>List every registered global session. No session parameter is needed.</p></div><div class="operation"><code>status</code><p>Inspect task states and the latest output for one session.</p></div><div class="operation"><code>logs</code><p>Read a longer log tail; use <code class="inline-code">tail</code> to set its size.</p></div><div class="operation"><code>start</code><p>Start an idle or exited task, or every task when no target is supplied.</p></div><div class="operation"><code>pause</code><p>Pause the Unix process group without terminating it.</p></div><div class="operation"><code>resume</code><p>Continue a paused process group.</p></div><div class="operation"><code>restart</code><p>Stop and immediately start the selected target.</p></div><div class="operation"><code>stop</code><p>Request a graceful stop, then force termination after its configured timeout.</p></div></div></div></section>
<section class="docs-section"><header><h2>Examples</h2><p>Arguments passed to <code class="inline-code">taskdeck_control</code>.</p></header><div class="docs-content"><h3>Inspect a session</h3><div class="code-block" data-copy><button class="copy" aria-label="Copy example" title="Copy"><svg viewBox="0 0 24 24" width="15" fill="none"><rect x="8" y="8" width="11" height="11" rx="2" stroke="currentColor" stroke-width="1.7"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" stroke="currentColor" stroke-width="1.7"/></svg></button><code>{
  "action": "status",
  "session": "training-api"
}</code></div><h3>Restart one task</h3><div class="code-block" data-copy><button class="copy" aria-label="Copy example" title="Copy"><svg viewBox="0 0 24 24" width="15" fill="none"><rect x="8" y="8" width="11" height="11" rx="2" stroke="currentColor" stroke-width="1.7"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" stroke="currentColor" stroke-width="1.7"/></svg></button><code>{
  "action": "restart",
  "session": "training-api",
  "task": "Run Backend API"
}</code></div></div></section></article></section></main></div><dialog class="call-dialog" id="call-dialog"><div class="dialog-shell"><header class="dialog-header"><div class="detail-status-icon" id="detail-status-icon" aria-hidden="true"><svg viewBox="0 0 24 24" fill="none"><path d="m7.5 12 3 3 6-7" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg></div><div class="dialog-title"><p id="call-dialog-subtitle"></p><h2 id="call-dialog-title">MCP call details</h2></div><button class="dialog-close" id="close-call-dialog" aria-label="Close details"><svg viewBox="0 0 24 24" width="16" fill="none"><path d="m7 7 10 10M17 7 7 17" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg></button></header><div class="dialog-body"><div class="call-overview" id="call-overview"></div><section class="detail-section"><header><span>Request</span><h3>What the agent asked for</h3></header><div class="field-list" id="request-fields"></div></section><section class="detail-section"><header><span>Result</span><h3>What Taskdeck returned</h3></header><div id="response-summary"></div><div id="response-data"></div></section></div></div></dialog><div class="toast" id="toast"><i class="connection-dot"></i><span>Copied to clipboard</span></div>
<script>
const sessionsEl=document.querySelector('#sessions'),tabsEl=document.querySelector('#tabs'),taskPaneEl=document.querySelector('#task-pane'),callsBodyEl=document.querySelector('#calls-body'),callDialogEl=document.querySelector('#call-dialog'),metaEl=document.querySelector('#meta'),titleEl=document.querySelector('#page-title'),toastEl=document.querySelector('#toast');let currentTask=null,snapshot=null,mcpCalls=[],currentView='tasks',toastTimer;
const icons={play:'<svg viewBox="0 0 24 24" fill="none"><path d="m8 5 11 7-11 7V5Z" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"/></svg>',pause:'<svg viewBox="0 0 24 24" fill="none"><path d="M8 6v12M16 6v12" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>',restart:'<svg viewBox="0 0 24 24" fill="none"><path d="M19 8V4m0 0h-4m4 0-3 3a7 7 0 1 0 2 8" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/></svg>',stop:'<svg viewBox="0 0 24 24" fill="none"><rect x="6" y="6" width="12" height="12" rx="2" stroke="currentColor" stroke-width="1.7"/></svg>'};
async function json(url,opts){const response=await fetch(url,opts);return response.json()}
async function loadSessions(){try{const response=await json('/api/sessions'),names=response.data||[],selected=sessionsEl.value;sessionsEl.innerHTML=names.length?names.map(name=>`<option value="${escapeAttr(name)}">${escapeHtml(name)}</option>`).join(''):'<option value="">No sessions</option>';if(names.includes(selected))sessionsEl.value=selected;if(names.length&&(currentView==='tasks'||!snapshot))await loadSnapshot();else if(!names.length)clearWorkspace()}catch(error){if(currentView==='tasks')metaEl.textContent='Daemon connection unavailable'}}
async function loadSnapshot(){if(!sessionsEl.value)return;const response=await json('/api/sessions/'+encodeURIComponent(sessionsEl.value)+'?tail=1000');if(!response.ok){if(currentView==='tasks')metaEl.textContent=response.message;return}snapshot=response.data;const labels=Object.keys(snapshot.tasks);if(!labels.includes(currentTask))currentTask=labels[0];if(currentView==='tasks')metaEl.textContent=`${snapshot.project} · ${snapshot.source}`;tabsEl.innerHTML=labels.map(label=>`<button class="tab ${label===currentTask?'active':''}" data-task="${escapeAttr(label)}">${escapeHtml(label)}</button>`).join('');renderTask()}
function clearWorkspace(){snapshot=null;currentTask=null;if(currentView==='tasks')metaEl.textContent='No sessions registered';tabsEl.innerHTML='';taskPaneEl.innerHTML='<div class="empty-state"><div><svg viewBox="0 0 24 24" fill="none"><path d="M4 6h16M4 12h12M4 18h8" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg><h2>No active workspace</h2><p>Register a project from the CLI or TUI. Its tasks and live output will appear here automatically.</p></div></div>'}
function renderTask(){if(!snapshot||!currentTask)return;const task=snapshot.tasks[currentTask],status=task.status,canStart=['idle','exited','failed'].includes(status),canPause=status==='running',canResume=status==='paused',canStop=['running','paused'].includes(status);taskPaneEl.innerHTML=`<div class="task-header"><div class="task-heading"><p class="eyebrow">Selected task</p><h1>${escapeHtml(task.label)}</h1><div class="task-detail"><span class="status ${escapeAttr(status)}">${escapeHtml(status)}</span>${task.pid?`<span>PID ${task.pid}</span>`:''}<span>${escapeHtml(task.cwd)}</span></div></div><div class="actions"><button class="button primary" data-action="start" ${canStart?'':'disabled'}>${icons.play}Start</button><button class="button" data-action="pause" ${canPause?'':'disabled'}>${icons.pause}Pause</button><button class="button" data-action="resume" ${canResume?'':'disabled'}>${icons.play}Resume</button><button class="button" data-action="restart">${icons.restart}Restart</button><button class="button danger" data-action="stop" ${canStop?'':'disabled'}>${icons.stop}Stop</button></div></div><div class="console"><div class="console-bar"><div class="traffic-lights"><i></i><i></i><i></i></div><span>LIVE OUTPUT</span><span>${task.logs.length} lines</span></div><pre class="logs">${task.logs.length?task.logs.map(line=>`<span class="${escapeAttr(line.stream)}">${escapeHtml(line.text)}</span>`).join('\n'):'<span class="empty">No output yet.</span>'}</pre></div>`;const logs=taskPaneEl.querySelector('.logs');logs.scrollTop=logs.scrollHeight}
async function act(action,button){if(!currentTask||button.disabled)return;button.disabled=true;await json('/api/action',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({session:sessionsEl.value,task:currentTask,action})});await loadSnapshot()}
async function loadMcpCalls(){try{const response=await json('/api/mcp-calls?limit=200');if(!response.ok)throw new Error(response.message);const nextCalls=response.data||[];if(JSON.stringify(nextCalls)!==JSON.stringify(mcpCalls)){mcpCalls=nextCalls;renderMcpCalls()}if(currentView==='calls')metaEl.textContent=`${mcpCalls.length} retained call${mcpCalls.length===1?'':'s'} · latest first`}catch(error){if(currentView==='calls')metaEl.textContent='MCP call history unavailable'}}
function renderMcpCalls(){callsBodyEl.innerHTML=mcpCalls.length?mcpCalls.map(call=>{const started=new Date(call.started_at_ms).toLocaleString(),input=call.input||{},target=callTarget(input);return `<tr><td><div class="operation-cell"><strong>${escapeHtml(titleCase(call.operation||'MCP call'))}</strong><span>${escapeHtml(call.tool)}</span></div></td><td><div class="target-cell"><strong>${escapeHtml(target.primary)}</strong><span>${escapeHtml(target.secondary)}</span></div></td><td><span class="status-pill ${call.success?'success':'error'}">${call.success?'Success':'Error'}</span></td><td><span class="call-time">${escapeHtml(started)}</span></td><td><span class="call-duration">${call.duration_ms} ms</span></td><td><button class="detail-button" data-call-id="${call.id}">View</button></td></tr>`}).join(''):'<tr class="calls-empty"><td colspan="6">No MCP tool calls recorded yet.</td></tr>'}
function callTarget(input){if(input.action==='sessions')return{primary:'All sessions',secondary:'Global daemon'};if(input.task)return{primary:input.task,secondary:input.session||'Task target'};if(input.session)return{primary:input.session,secondary:input.tail?`Last ${input.tail} log lines`:'Full session'};return{primary:'Taskdeck daemon',secondary:'No target supplied'}}
function titleCase(value){return String(value).replace(/[_-]+/g,' ').replace(/\b\w/g,char=>char.toUpperCase())}
function requestFields(call){const input=call.request?.params?.arguments||{},fields=[['Operation',titleCase(input.action||call.operation||'Unknown'),false]];if(input.session)fields.push(['Session',input.session,true]);if(input.task)fields.push(['Task',input.task,false]);else if(['start','stop','restart','pause','resume'].includes(input.action))fields.push(['Task','All tasks in the session',false]);if(input.tail!=null)fields.push(['Log lines',input.tail,false]);Object.entries(input).filter(([key])=>!['action','session','task','tail'].includes(key)).forEach(([key,value])=>fields.push([titleCase(key),displayValue(value),typeof value==='string']));return fields.map(([label,value,mono])=>`<div class="field-row"><span class="field-label">${escapeHtml(label)}</span><strong class="field-value ${mono?'mono':''}">${escapeHtml(value)}</strong></div>`).join('')}
function displayValue(value){if(value==null)return 'Not set';if(Array.isArray(value))return value.map(displayValue).join(', ');if(typeof value==='object')return Object.entries(value).map(([key,item])=>`${titleCase(key)}: ${displayValue(item)}`).join(' · ');return String(value)}
function renderResultData(data){if(data==null)return '';if(Array.isArray(data))return `<div class="result-data">${data.length?`<div class="value-list">${data.map(item=>`<div class="value-list-item">${escapeHtml(displayValue(item))}</div>`).join('')}</div>`:'<div class="result-empty">No sessions are currently registered.</div>'}</div>`;if(typeof data==='object'&&data.tasks){const facts=[['Session',data.name],['Project',data.project],['Source',data.source]].filter(([,value])=>value!=null).map(([label,value])=>`<div class="field-row"><span class="field-label">${label}</span><strong class="field-value ${label!=='Source'?'mono':''}">${escapeHtml(value)}</strong></div>`).join(''),tasks=Object.entries(data.tasks).map(([name,task])=>renderTaskResult(name,task)).join('');return `<div class="result-data"><div class="session-facts">${facts}</div>${tasks||'<div class="result-empty">This session has no tasks.</div>'}</div>`}if(typeof data==='object'){const rows=Object.entries(data).map(([key,value])=>`<div class="field-row"><span class="field-label">${escapeHtml(titleCase(key))}</span><strong class="field-value">${escapeHtml(displayValue(value))}</strong></div>`).join('');return `<div class="result-data"><div class="field-list">${rows}</div></div>`}return `<div class="result-data"><div class="value-list-item">${escapeHtml(displayValue(data))}</div></div>`}
function renderTaskResult(name,task){const logs=Array.isArray(task.logs)?task.logs:[],status=task.status||'unknown',statusClass=status==='failed'?'error':status==='paused'?'paused':'success';return `<article class="task-result"><div class="task-result-header"><strong>${escapeHtml(name)}</strong><span class="status-pill ${statusClass}">${escapeHtml(titleCase(status))}</span></div><div class="task-result-meta">${task.pid?`<span>PID ${task.pid}</span>`:''}${task.command?`<span>${escapeHtml(task.command)}</span>`:''}${task.cwd?`<span>${escapeHtml(task.cwd)}</span>`:''}${task.last_exit?`<span>Last exit: ${escapeHtml(task.last_exit)}</span>`:''}</div>${logs.length?`<details class="log-disclosure"><summary>${logs.length} recent log line${logs.length===1?'':'s'}</summary><div class="human-logs">${logs.map(line=>`<div class="human-log"><span>${escapeHtml(line.stream)}</span><span>${escapeHtml(line.text)}</span></div>`).join('')}</div></details>`:''}</article>`}
async function openCallDetails(id){const response=await json('/api/mcp-calls/'+encodeURIComponent(id));if(!response.ok){showToast(response.message);return}const call=response.data,started=new Date(call.started_at_ms).toLocaleString(),result=call.response?.result||{},structured=result.structuredContent,message=structured?.message||result.content?.[0]?.text||(call.success?'The request completed successfully.':'The request could not be completed.'),icon=document.querySelector('#detail-status-icon');document.querySelector('#call-dialog-title').textContent=`${titleCase(call.operation||'MCP')} request`;document.querySelector('#call-dialog-subtitle').textContent=`${call.tool} · Call #${call.id}`;icon.classList.toggle('error',!call.success);icon.innerHTML=call.success?'<svg viewBox="0 0 24 24" fill="none"><path d="m7.5 12 3 3 6-7" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>':'<svg viewBox="0 0 24 24" fill="none"><path d="m8 8 8 8m0-8-8 8" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>';document.querySelector('#call-overview').innerHTML=`<div class="overview-item"><span>Started</span><strong>${escapeHtml(started)}</strong></div><div class="overview-item"><span>Duration</span><strong>${call.duration_ms} ms</strong></div><div class="overview-item"><span>Request ID</span><strong>${escapeHtml(call.request.id??'—')}</strong></div>`;document.querySelector('#request-fields').innerHTML=requestFields(call);document.querySelector('#response-summary').innerHTML=`<div class="outcome ${call.success?'':'error'}"><span class="outcome-icon">${call.success?'✓':'×'}</span><strong>${call.success?'Completed successfully':'Request failed'}</strong><p>${escapeHtml(message)}</p></div>`;document.querySelector('#response-data').innerHTML=renderResultData(structured?.data);callDialogEl.showModal()}
function setView(view){currentView=view;document.querySelectorAll('.nav-button').forEach(button=>button.classList.toggle('active',button.dataset.view===view));document.querySelectorAll('.view').forEach(panel=>{const active=panel.id===view+'-view';panel.hidden=!active;panel.classList.toggle('active',active)});titleEl.textContent=view==='docs'?'MCP Guide':view==='calls'?'MCP calls':'Task workspace';sessionsEl.hidden=view!=='tasks';metaEl.textContent=view==='docs'?'Connect agents to Taskdeck over Streamable HTTP':view==='calls'?`${mcpCalls.length} retained call${mcpCalls.length===1?'':'s'} · latest first`:snapshot?`${snapshot.project} · ${snapshot.source}`:'No sessions registered';if(view==='calls')loadMcpCalls()}
function setTheme(){const current=document.documentElement.dataset.theme||'system',next=current==='system'?'light':current==='light'?'dark':'system';if(next==='system')delete document.documentElement.dataset.theme;else document.documentElement.dataset.theme=next;localStorage.setItem('taskdeck-theme',next);showToast(`Theme: ${next}`)}
function applySavedTheme(){const value=localStorage.getItem('taskdeck-theme');if(value&&value!=='system')document.documentElement.dataset.theme=value}
function showToast(message){toastEl.querySelector('span').textContent=message;toastEl.classList.add('visible');clearTimeout(toastTimer);toastTimer=setTimeout(()=>toastEl.classList.remove('visible'),1500)}
async function copyBlock(block){const code=block.querySelector('code').textContent;await navigator.clipboard.writeText(code);showToast('Copied to clipboard')}
function escapeHtml(value){return String(value).replace(/[&<>"']/g,char=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]))}function escapeAttr(value){return escapeHtml(value)}
function updateEndpoint(){const endpoint=location.origin+'/mcp';document.querySelector('#endpoint-label').textContent=endpoint.replace(/^https?:\/\//,'');document.querySelector('#client-config').textContent=JSON.stringify({mcpServers:{taskdeck:{type:'http',url:endpoint}}},null,2)}
applySavedTheme();updateEndpoint();document.querySelector('.sidebar').onclick=event=>{const button=event.target.closest('[data-view]');if(button)setView(button.dataset.view)};sessionsEl.onchange=()=>{currentTask=null;loadSnapshot()};tabsEl.onclick=event=>{const button=event.target.closest('[data-task]');if(button){currentTask=button.dataset.task;renderTask();tabsEl.querySelectorAll('.tab').forEach(tab=>tab.classList.toggle('active',tab===button))}};taskPaneEl.onclick=event=>{const button=event.target.closest('[data-action]');if(button)act(button.dataset.action,button)};callsBodyEl.onclick=event=>{const button=event.target.closest('[data-call-id]');if(button)openCallDetails(button.dataset.callId)};document.querySelector('#refresh-calls').onclick=loadMcpCalls;document.querySelector('#close-call-dialog').onclick=()=>callDialogEl.close();callDialogEl.onclick=event=>{if(event.target===callDialogEl)callDialogEl.close()};document.querySelector('#theme').onclick=setTheme;document.querySelector('#docs-view').onclick=event=>{const button=event.target.closest('.copy');if(button)copyBlock(button.closest('[data-copy]'))};loadSessions();setInterval(()=>{if(currentView==='tasks')loadSnapshot();else if(currentView==='calls')loadMcpCalls()},1000);setInterval(loadSessions,5000);
</script></body></html>"#;

const FAVICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" role="img" aria-label="Taskdeck">
<rect width="32" height="32" rx="7" fill="#111315"/>
<path d="M8 9h16M8 16h10M8 23h7" fill="none" stroke="#56b6c2" stroke-width="3" stroke-linecap="round"/>
<circle cx="23" cy="23" r="3" fill="#51c878"/>
</svg>"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_exposes_one_control_tool() {
        let tool = mcp_tool_definition();
        assert_eq!(tool["name"], "taskdeck_control");
        assert_eq!(tool["inputSchema"]["required"][0], "action");
    }

    #[test]
    fn page_exposes_mcp_guide_and_adaptive_visual_system() {
        assert!(INDEX_HTML.contains("/favicon.svg"));
        assert!(INDEX_HTML.contains("data-view=\"docs\""));
        assert!(INDEX_HTML.contains("data-view=\"calls\""));
        assert!(INDEX_HTML.contains("/api/mcp-calls"));
        assert!(INDEX_HTML.contains("What the agent asked for"));
        assert!(!INDEX_HTML.contains("id=\"call-request\""));
        assert!(INDEX_HTML.contains("taskdeck_control"));
        assert!(INDEX_HTML.contains("location.origin+'/mcp'"));
        assert!(INDEX_HTML.contains("prefers-color-scheme:dark"));
        assert!(INDEX_HTML.contains("prefers-reduced-motion:reduce"));
        assert!(INDEX_HTML.contains("prefers-reduced-transparency:reduce"));
        assert!(FAVICON_SVG.contains("<svg"));
    }
}
