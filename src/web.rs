use std::collections::{HashMap, HashSet};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use axum::body::Body;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Redirect, Response as AxumResponse};
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
        .route(
            "/api/nodes/{node}/settings",
            get(node_settings).put(update_node_settings),
        )
        .route(
            "/api/nodes/self/service",
            get(node_service).post(node_service_action),
        )
        .route("/api/workspaces", get(list_workspaces))
        .route(
            "/api/workspaces/{session}/alias",
            put(update_workspace_alias),
        )
        .route(
            "/api/workflow-groups",
            get(list_workflow_groups).post(create_workflow_group),
        )
        .route(
            "/api/workflow-groups/{group}",
            get(get_workflow_group)
                .put(update_workflow_group)
                .delete(delete_workflow_group),
        )
        .route(
            "/api/workflow-groups/{group}/actions",
            post(workflow_group_action),
        )
        .route(
            "/api/workflow-groups/{group}/revisions",
            get(list_workflow_revisions),
        )
        .route(
            "/api/workflow-groups/{group}/revisions/{revision}/restore",
            post(restore_workflow_revision),
        )
        .route("/api/workflow-groups/{group}/run", post(run_workflow_group))
        .route("/api/quotas", get(list_quotas).post(create_quota))
        .route(
            "/api/quotas/{quota}",
            put(update_quota).delete(delete_quota),
        )
        .route("/api/notifications", get(list_notifications))
        .route("/api/notifications/read", post(mark_notifications_read))
        .route(
            "/api/notification-rules",
            get(list_notification_rules).post(create_notification_rule),
        )
        .route(
            "/api/notification-rules/{rule}",
            put(update_notification_rule).delete(delete_notification_rule),
        )
        .route("/api/tokens", get(list_api_tokens).post(create_api_token))
        .route("/api/tokens/{token}", delete(revoke_api_token))
        .route(
            "/api/board-templates",
            get(list_board_templates).post(create_board_template),
        )
        .route("/api/board-templates/import", post(import_board_template))
        .route(
            "/api/board-templates/{template}",
            delete(delete_board_template),
        )
        .route(
            "/api/board-templates/{template}/apply",
            post(apply_board_template),
        )
        .route(
            "/api/board-templates/{template}/export",
            get(export_board_template),
        )
        .route(
            "/api/dependencies",
            get(list_dependencies).post(create_dependency),
        )
        .route("/api/dependencies/{dependency}", delete(delete_dependency))
        .route("/api/node-metrics", get(node_metrics))
        .route(
            "/api/scaling-policies",
            get(list_scaling_policies).post(create_scaling_policy),
        )
        .route(
            "/api/scaling-policies/{policy}",
            put(update_scaling_policy).delete(delete_scaling_policy),
        )
        .route("/api/boards", get(list_boards).post(create_board))
        .route(
            "/api/boards/{board}",
            get(get_board).put(update_board).delete(delete_board),
        )
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{session}", get(session_snapshot))
        .route("/api/sessions/{session}/tasks/{task}/logs", get(task_logs))
        .route(
            "/api/sessions/{session}/tasks/{task}/metrics",
            get(task_metrics),
        )
        .route(
            "/api/sessions/{session}/tasks/{task}/history",
            delete(clear_task_history),
        )
        .route(
            "/api/sessions/{session}/config",
            get(session_config).put(update_session_config),
        )
        .route("/api/audit", get(list_audit))
        .route("/api/audit/{audit_id}", get(audit_detail))
        .route("/api/mcp-calls", get(list_mcp_calls))
        .route("/api/mcp-calls/{id}", get(mcp_call_detail))
        .route("/api/action", post(action))
        .route("/mcp", post(mcp))
        .route("/api/task-runs", get(list_task_runs))
        .route("/api/events", get(list_events_route))
        .route("/login", get(login_page).post(login_submit))
        .route("/logout", post(logout))
        .route("/me", get(auth_status))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
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

#[derive(Deserialize)]
struct LoginBody {
    access_key: String,
}

pub const AUTH_COOKIE: &str = "taskdeck_session";

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn session_cookie(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::COOKIE)?.to_str().ok()?;
    value.split(';').find_map(|part| {
        let part = part.trim();
        part.strip_prefix(AUTH_COOKIE)
            .and_then(|rest| rest.strip_prefix('='))
            .filter(|token| !token.is_empty())
            .map(|token| token.to_string())
    })
}

async fn auth_middleware(
    State(state): State<DaemonState>,
    request: axum::http::Request<Body>,
    next: Next,
) -> AxumResponse {
    let settings = match state.store.auth_settings() {
        Ok(settings) => settings,
        Err(error) => return Json(Response::error(format!("{error:#}"))).into_response(),
    };
    if !settings.enabled {
        return next.run(request).await;
    }
    let path = request.uri().path();
    let method = request.method().clone();
    let headers = request.headers().clone();
    let exempt = path == "/health"
        || path == "/healthz"
        || path.starts_with("/assets/")
        || path == "/favicon.svg"
        || path == "/favicon.ico"
        || path == "/me";
    let exempt = exempt
        || (path == "/login" && method == "GET")
        || (path == "/login" && method == "POST")
        || path == "/api/agent/connect";
    let authenticated = session_cookie(&headers)
        .is_some_and(|token| state.store.valid_auth_session(Some(&token)))
        || bearer_token(&headers).is_some_and(|key| {
            if key.starts_with("tdk_") {
                state.store.verify_api_token(&key).unwrap_or(false)
            } else {
                state.store.verify_access_key(&key).unwrap_or(false)
            }
        });
    if exempt || authenticated {
        return next.run(request).await;
    }
    if path == "/" && method.as_str() == "GET" {
        return Html(LOGIN_HTML).into_response();
    }
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"kind":"unauthorized","status":401})),
    )
        .into_response()
}

async fn login_page() -> Html<String> {
    Html(LOGIN_HTML.replace("%LOGIN_ERROR%", ""))
}

async fn login_submit(
    State(state): State<DaemonState>,
    Form(body): Form<LoginBody>,
) -> AxumResponse {
    let enabled = state
        .store
        .auth_settings()
        .map(|settings| settings.enabled)
        .unwrap_or(false);
    if !enabled {
        return Redirect::to("/").into_response();
    }
    match state.store.verify_access_key(&body.access_key) {
        Ok(true) => {}
        _ => {
            return Html(LOGIN_HTML.replace(
                "%LOGIN_ERROR%",
                "<p class=\"form-error\">Invalid access key</p>",
            ))
            .into_response();
        }
    };
    match state.store.create_auth_session() {
        Ok(token) => {
            let mut response = Redirect::to("/").into_response();
            response.headers_mut().insert(
                header::SET_COOKIE,
                format!("{AUTH_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax")
                    .parse()
                    .expect("valid cookie"),
            );
            response
        }
        Err(_) => {
            Html(LOGIN_HTML.replace("%LOGIN_ERROR%", "<p class=\"form-error\">Login failed</p>"))
                .into_response()
        }
    }
}

async fn logout(State(state): State<DaemonState>, headers: HeaderMap) -> AxumResponse {
    state
        .store
        .delete_auth_session(session_cookie(&headers).as_deref());
    let mut response = Redirect::to("/login").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        format!("{AUTH_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0")
            .parse()
            .expect("valid cookie"),
    );
    response
}

async fn auth_status(State(state): State<DaemonState>, headers: HeaderMap) -> Json<Response> {
    let settings = match state.store.auth_settings() {
        Ok(v) => v,
        Err(e) => return Json(Response::error(format!("{e:#}"))),
    };
    let authenticated = settings.enabled
        && (session_cookie(&headers)
            .is_some_and(|token| state.store.valid_auth_session(Some(&token)))
            || bearer_token(&headers)
                .is_some_and(|key| state.store.verify_access_key(&key).unwrap_or(false)));
    Json(Response::ok(
        "authentication status",
        json!({"enabled":settings.enabled,"configured":settings.password_hash.is_some(),"authenticated":!settings.enabled||authenticated}),
    ))
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

async fn node_settings(
    State(state): State<DaemonState>,
    Path(node): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Response> {
    dispatch_selected_node(state, &node, &query, RemoteRequest::GetNodeSettings).await
}

#[derive(Debug, Deserialize)]
struct ServiceActionBody {
    action: ServiceAction,
    scope: ServiceScope,
    #[serde(default)]
    home: Option<String>,
}

async fn node_service(
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Response>, Json<Response>> {
    let scope = match query.get("scope").map(String::as_str) {
        Some("system") => ServiceScope::System,
        _ => ServiceScope::User,
    };
    let result = tokio::task::spawn_blocking(move || service_status(scope))
        .await
        .map_err(|error| {
            Json(Response::error(format!(
                "service status task failed: {error}"
            )))
        })?;
    match result {
        Ok(status) => Ok(Json(Response::ok("service status", status))),
        Err(error) => Err(Json(Response::error(format!("{error:#}")))),
    }
}

async fn node_service_action(
    State(state): State<DaemonState>,
    Json(body): Json<ServiceActionBody>,
) -> Result<Json<Response>, Json<Response>> {
    let started = std::time::Instant::now();
    let home = body.home.clone().map(std::path::PathBuf::from);
    let home_for_audit = home.clone();
    let scope = body.scope;
    let action = body.action;
    let result = tokio::task::spawn_blocking(move || service_control(scope, action, home))
        .await
        .map_err(|error| Json(Response::error(format!("service task failed: {error}"))))?;
    let response = match result {
        Ok(status) => Response::ok(
            match action {
                ServiceAction::Status => "service status",
                ServiceAction::Install => "service installed",
                ServiceAction::Uninstall => "service uninstalled",
                ServiceAction::Start => "service started",
                ServiceAction::Stop => "service stopped",
            },
            status,
        ),
        Err(error) => Response::error(format!("{error:#}")),
    };
    let response_value = serde_json::to_value(&response).unwrap_or_else(
        |error| serde_json::json!({"ok": response.ok, "message": format!("{error}")}),
    );
    let _ = record_audit_value(
        &state,
        AuditContext::new(AuditSource::Web, AuditTransport::Http),
        None,
        "service_control",
        match action {
            ServiceAction::Status => "status",
            ServiceAction::Install => "install",
            ServiceAction::Uninstall => "uninstall",
            ServiceAction::Start => "start",
            ServiceAction::Stop => "stop",
        },
        None,
        None,
        AuditStatus::from_ok(response.ok),
        current_millis(),
        started.elapsed().as_millis() as u64,
        serde_json::json!({"action": action, "scope": scope, "home": home_for_audit}),
        response_value,
        serde_json::json!({"node":"self"}),
        Some(state.public_settings().node_id),
    );
    if response.ok {
        Ok(Json(response))
    } else {
        Err(Json(response))
    }
}

fn current_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

async fn list_workspaces(
    State(state): State<DaemonState>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Response> {
    let node = match selected_node(&state, &query) {
        Ok(node) => node,
        Err(response) => return Json(response),
    };
    let request = RemoteRequest::ListWorkspaces;
    let response = state
        .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))
        .await;
    if response.ok || node == "self" {
        return Json(response);
    }
    if let Some(inventory) = state.cluster.cached_inventory(&node) {
        let summaries = inventory
            .into_iter()
            .map(|session| crate::protocol::WorkspaceSummary {
                display_name: session
                    .alias
                    .clone()
                    .unwrap_or_else(|| session.name.clone()),
                session: session.name,
                alias: session.alias,
                project: session.project,
            })
            .collect::<Vec<_>>();
        return Json(Response::ok("cached workspaces", summaries));
    }
    Json(response)
}

async fn update_workspace_alias(
    State(state): State<DaemonState>,
    Path(session): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    Json(body): Json<Value>,
) -> Json<Response> {
    let node = match selected_node(&state, &query) {
        Ok(node) => node,
        Err(response) => return Json(response),
    };
    let alias = body
        .get("alias")
        .and_then(Value::as_str)
        .map(str::to_string);
    let request = RemoteRequest::SetWorkspaceAlias { session, alias };
    Json(
        state
            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))
            .await,
    )
}

#[derive(Debug, Deserialize)]
struct WorkflowGroupActionBody {
    action: Action,
}

async fn list_workflow_groups(State(state): State<DaemonState>) -> Json<Response> {
    if let Err(response) = require_workflow_leader(&state) {
        return Json(response);
    }
    Json(match workflow_groups_view(&state) {
        Ok(view) => Response::ok("workflow groups", view),
        Err(error) => Response::error(format!("{error:#}")),
    })
}

async fn get_workflow_group(
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

async fn create_workflow_group(
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

async fn update_workflow_group(
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

async fn delete_workflow_group(
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

async fn workflow_group_action(
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

fn require_workflow_leader(state: &DaemonState) -> std::result::Result<(), Response> {
    if state.public_settings().role == NodeRole::Leader {
        Ok(())
    } else {
        Err(Response::error_with_data(
            "workflow groups are available on leader nodes only",
            json!({"kind": "validation_error", "status": 403}),
        ))
    }
}

fn validate_workflow_group_scope(
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

fn workflow_groups_view(state: &DaemonState) -> Result<WorkflowGroupsView> {
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

fn workflow_group_view(state: &DaemonState, group: WorkflowGroup) -> WorkflowGroupView {
    let (nodes, inventories) = workflow_context(state);
    resolve_workflow_group(&group, &nodes, &inventories)
}

fn workflow_context(
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

fn workflow_targets(
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

fn resolve_workflow_group(
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

fn ordered_task_labels(session: &SessionSnapshot) -> Vec<String> {
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

async fn run_workflow_group_action(
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

fn record_workflow_group_http_audit(
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

async fn list_boards(State(state): State<DaemonState>) -> Json<Response> {
    if let Err(response) = require_board_leader(&state) {
        return Json(response);
    }
    Json(match boards_view(&state) {
        Ok(view) => Response::ok("boards", view),
        Err(error) => Response::error(format!("{error:#}")),
    })
}

async fn get_board(State(state): State<DaemonState>, Path(board): Path<String>) -> Json<Response> {
    if let Err(response) = require_board_leader(&state) {
        return Json(response);
    }
    Json(match state.store.board(&board) {
        Ok(Some(board)) => Response::ok("board", board_view(&state, board)),
        Ok(None) => Response::error(format!("board '{board}' not found")),
        Err(error) => Response::error(format!("{error:#}")),
    })
}

async fn create_board(
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

async fn update_board(
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

async fn delete_board(
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

fn require_board_leader(state: &DaemonState) -> std::result::Result<(), Response> {
    if state.public_settings().role == NodeRole::Leader {
        Ok(())
    } else {
        Err(Response::error_with_data(
            "boards are available on leader nodes only",
            json!({"kind": "validation_error", "status": 403}),
        ))
    }
}

fn validate_board_scope(
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

fn boards_view(state: &DaemonState) -> Result<BoardsView> {
    let boards = state.store.boards()?;
    let (nodes, inventories) = workflow_context(state);
    let targets = workflow_targets(&nodes, &inventories);
    let boards = boards
        .into_iter()
        .map(|board| resolve_board(board, &nodes, &inventories))
        .collect();
    Ok(BoardsView { boards, targets })
}

fn board_view(state: &DaemonState, board: Board) -> BoardView {
    let (nodes, inventories) = workflow_context(state);
    resolve_board(board, &nodes, &inventories)
}

fn resolve_board(
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

fn record_board_http_audit(
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

async fn list_workflow_revisions(
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

async fn restore_workflow_revision(
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
struct WorkflowRunBody {
    #[serde(default = "default_stop_on_failure")]
    stop_on_failure: bool,
}

fn default_stop_on_failure() -> bool {
    true
}

async fn run_workflow_group(
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
fn workflow_run_order(group: &WorkflowGroup) -> std::result::Result<Vec<usize>, String> {
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

async fn run_workflow_group_ordered(
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

fn summarize_workflow_results(
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

fn quota_sessions(state: &DaemonState) -> Vec<String> {
    let mut sessions: Vec<String> = state
        .node_summaries()
        .into_iter()
        .flat_map(|node| node.sessions)
        .collect();
    sessions.sort();
    sessions.dedup();
    sessions
}

async fn list_quotas(State(state): State<DaemonState>) -> Json<Response> {
    Json(match state.store.quotas() {
        Ok(quotas) => Response::ok(
            "quotas",
            WorkspaceQuotasView {
                quotas,
                sessions: quota_sessions(&state),
            },
        ),
        Err(error) => Response::error(format!("{error:#}")),
    })
}

async fn create_quota(
    State(state): State<DaemonState>,
    Json(input): Json<WorkspaceQuotaInput>,
) -> Json<Response> {
    let started_at_ms = current_millis();
    let started = Instant::now();
    let request = serde_json::to_value(&input).unwrap_or_else(|_| json!({}));
    let node_id = state.public_settings().node_id;
    let response = match state.store.create_quota(&node_id, input) {
        Ok(quota) => Response::ok("quota created", quota),
        Err(error) => Response::error(format!("{error:#}")),
    };
    record_feature_http_audit(
        &state,
        "quota",
        "quota_create",
        "quota_id",
        None,
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );
    Json(response)
}

async fn update_quota(
    State(state): State<DaemonState>,
    Path(quota): Path<String>,
    Json(input): Json<WorkspaceQuotaInput>,
) -> Json<Response> {
    let started_at_ms = current_millis();
    let started = Instant::now();
    let request = serde_json::to_value(&input).unwrap_or_else(|_| json!({}));
    let response = match state.store.update_quota(&quota, input) {
        Ok(quota) => Response::ok("quota updated", quota),
        Err(error) => Response::error(format!("{error:#}")),
    };
    record_feature_http_audit(
        &state,
        "quota",
        "quota_update",
        "quota_id",
        Some(&quota),
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );
    Json(response)
}

async fn delete_quota(
    State(state): State<DaemonState>,
    Path(quota): Path<String>,
) -> Json<Response> {
    let started_at_ms = current_millis();
    let started = Instant::now();
    let request = json!({"id": quota});
    let response = match state.store.delete_quota(&quota) {
        Ok(true) => Response::empty("quota deleted"),
        Ok(false) => Response::error(format!("quota '{quota}' not found")),
        Err(error) => Response::error(format!("{error:#}")),
    };
    record_feature_http_audit(
        &state,
        "quota",
        "quota_delete",
        "quota_id",
        Some(&quota),
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );
    Json(response)
}

// ---------------------------------------------------------------------------
// Notifications and alert rules
// ---------------------------------------------------------------------------

async fn list_notifications(
    State(state): State<DaemonState>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Response> {
    let limit = parse_positive_usize(&query, "limit", 200)
        .unwrap_or(200)
        .min(1000);
    Json(match state.store.notifications(limit) {
        Ok(notifications) => {
            let unread_count = state.store.unread_notification_count().unwrap_or(0);
            Response::ok(
                "notifications",
                NotificationsView {
                    notifications,
                    unread_count,
                },
            )
        }
        Err(error) => Response::error(format!("{error:#}")),
    })
}

async fn mark_notifications_read(
    State(state): State<DaemonState>,
    Json(input): Json<NotificationMarkReadInput>,
) -> Json<Response> {
    let response = match (input.all, input.id) {
        (true, _) | (false, None) => state.store.mark_notifications_read(None).map(|changed| {
            Response::ok(
                format!("marked {changed} notifications read"),
                json!({"changed": changed}),
            )
        }),
        (false, Some(id)) => state
            .store
            .mark_notifications_read(Some(id))
            .map(|changed| {
                Response::ok(
                    format!("marked {changed} notification read"),
                    json!({"changed": changed}),
                )
            }),
    };
    Json(match response {
        Ok(response) => response,
        Err(error) => Response::error(format!("{error:#}")),
    })
}

async fn list_notification_rules(State(state): State<DaemonState>) -> Json<Response> {
    Json(match state.store.notification_rules() {
        Ok(rules) => Response::ok("notification rules", rules),
        Err(error) => Response::error(format!("{error:#}")),
    })
}

async fn create_notification_rule(
    State(state): State<DaemonState>,
    Json(input): Json<NotificationRuleInput>,
) -> Json<Response> {
    let started_at_ms = current_millis();
    let started = Instant::now();
    let request = serde_json::to_value(&input).unwrap_or_else(|_| json!({}));
    let response = match state.store.create_notification_rule(input) {
        Ok(rule) => Response::ok("notification rule created", rule),
        Err(error) => Response::error(format!("{error:#}")),
    };
    record_feature_http_audit(
        &state,
        "notification_rule",
        "notification_rule_create",
        "rule_id",
        None,
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );
    Json(response)
}

async fn update_notification_rule(
    State(state): State<DaemonState>,
    Path(rule): Path<String>,
    Json(input): Json<NotificationRuleInput>,
) -> Json<Response> {
    let started_at_ms = current_millis();
    let started = Instant::now();
    let request = serde_json::to_value(&input).unwrap_or_else(|_| json!({}));
    let response = match state.store.update_notification_rule(&rule, input) {
        Ok(rule) => Response::ok("notification rule updated", rule),
        Err(error) => Response::error(format!("{error:#}")),
    };
    record_feature_http_audit(
        &state,
        "notification_rule",
        "notification_rule_update",
        "rule_id",
        Some(&rule),
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );
    Json(response)
}

async fn delete_notification_rule(
    State(state): State<DaemonState>,
    Path(rule): Path<String>,
) -> Json<Response> {
    let started_at_ms = current_millis();
    let started = Instant::now();
    let request = json!({"id": rule});
    let response = match state.store.delete_notification_rule(&rule) {
        Ok(true) => Response::empty("notification rule deleted"),
        Ok(false) => Response::error(format!("notification rule '{rule}' not found")),
        Err(error) => Response::error(format!("{error:#}")),
    };
    record_feature_http_audit(
        &state,
        "notification_rule",
        "notification_rule_delete",
        "rule_id",
        Some(&rule),
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );
    Json(response)
}

// ---------------------------------------------------------------------------
// API tokens (external integrations)
// ---------------------------------------------------------------------------

async fn list_api_tokens(State(state): State<DaemonState>) -> Json<Response> {
    Json(match state.store.api_tokens() {
        Ok(tokens) => Response::ok("api tokens", ApiTokensView { tokens }),
        Err(error) => Response::error(format!("{error:#}")),
    })
}

async fn create_api_token(
    State(state): State<DaemonState>,
    Json(input): Json<ApiTokenInput>,
) -> Json<Response> {
    let started_at_ms = current_millis();
    let started = Instant::now();
    let request = serde_json::to_value(&input).unwrap_or_else(|_| json!({}));
    let response = match state.store.create_api_token(&input.name) {
        Ok(created) => Response::ok("api token created", created),
        Err(error) => Response::error(format!("{error:#}")),
    };
    record_feature_http_audit(
        &state,
        "api_token",
        "api_token_create",
        "token_id",
        None,
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );
    Json(response)
}

async fn revoke_api_token(
    State(state): State<DaemonState>,
    Path(token): Path<String>,
) -> Json<Response> {
    let started_at_ms = current_millis();
    let started = Instant::now();
    let request = json!({"id": token});
    let response = match state.store.revoke_api_token(&token) {
        Ok(true) => Response::empty("api token revoked"),
        Ok(false) => Response::error(format!("api token '{token}' not found")),
        Err(error) => Response::error(format!("{error:#}")),
    };
    record_feature_http_audit(
        &state,
        "api_token",
        "api_token_revoke",
        "token_id",
        Some(&token),
        request,
        &response,
        started_at_ms,
        started.elapsed().as_millis() as u64,
    );
    Json(response)
}

// ---------------------------------------------------------------------------
// Board templates
// ---------------------------------------------------------------------------

async fn list_board_templates(State(state): State<DaemonState>) -> Json<Response> {
    Json(match state.store.board_templates() {
        Ok(templates) => Response::ok("board templates", BoardTemplatesView { templates }),
        Err(error) => Response::error(format!("{error:#}")),
    })
}

async fn create_board_template(
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

async fn import_board_template(
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

async fn delete_board_template(
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

async fn apply_board_template(
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

async fn export_board_template(
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

fn dependencies_view(
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

fn validate_dependency_scope(
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

fn dependency_creates_cycle(
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

async fn list_dependencies(State(state): State<DaemonState>) -> Json<Response> {
    Json(match state.store.task_dependencies() {
        Ok(dependencies) => {
            Response::ok("task dependencies", dependencies_view(&state, dependencies))
        }
        Err(error) => Response::error(format!("{error:#}")),
    })
}

async fn create_dependency(
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

async fn delete_dependency(
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

fn task_status_key(status: &crate::protocol::TaskStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{status:?}"))
}

async fn node_metrics(State(state): State<DaemonState>) -> Json<Response> {
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

async fn list_scaling_policies(State(state): State<DaemonState>) -> Json<Response> {
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

async fn create_scaling_policy(
    State(state): State<DaemonState>,
    Json(input): Json<ScalingPolicyInput>,
) -> Json<Response> {
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

async fn update_scaling_policy(
    State(state): State<DaemonState>,
    Path(policy): Path<String>,
    Json(input): Json<ScalingPolicyInput>,
) -> Json<Response> {
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

async fn delete_scaling_policy(
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

#[allow(clippy::too_many_arguments)]
fn record_feature_http_audit(
    state: &DaemonState,
    request_kind: &str,
    operation: &str,
    entity_key: &str,
    entity_id: Option<&str>,
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
    let mut details = json!({});
    if let Some(entity_id) = entity_id {
        details[entity_key] = json!(entity_id);
    }
    let _ = record_audit_value(
        state,
        AuditContext::new(AuditSource::Web, AuditTransport::Http),
        None,
        request_kind,
        operation,
        None,
        None,
        status,
        started_at_ms,
        duration_ms,
        request,
        response_value,
        details,
        Some(state.public_settings().node_id),
    );
}

async fn update_node_settings(
    State(state): State<DaemonState>,
    Path(node): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    Json(patch): Json<crate::protocol::NodeSettingsPatch>,
) -> Json<Response> {
    dispatch_selected_node(
        state,
        &node,
        &query,
        RemoteRequest::PutNodeSettings { patch },
    )
    .await
}

async fn dispatch_selected_node(
    state: DaemonState,
    requested_node: &str,
    query: &HashMap<String, String>,
    request: RemoteRequest,
) -> Json<Response> {
    let inferred = query
        .get("node")
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| requested_node.to_string());
    let response = if inferred == "self" {
        let audit = AuditContext::new(AuditSource::Web, AuditTransport::Http)
            .with_request_defaults(&request.clone().into_local())
            .with_origin_node(state.public_settings().node_id);
        crate::daemon::dispatch_async_with_audit(state.clone(), request.into_local(), Some(audit))
            .await
    } else if state.public_settings().role == crate::state::NodeRole::Worker {
        Response::error("worker node settings are local-only")
    } else {
        state
            .dispatch_node_with_audit(
                &inferred,
                request.clone(),
                web_audit_context(&state, &request),
            )
            .await
    };
    Json(response)
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

fn audit_context_for_remote_request(
    state: &DaemonState,
    source: AuditSource,
    transport: AuditTransport,
    request: &RemoteRequest,
) -> AuditContext {
    let local_request = request.clone().into_local();
    AuditContext::new(source, transport)
        .with_request_defaults(&local_request)
        .with_origin_node(state.public_settings().node_id)
}

fn web_audit_context(state: &DaemonState, request: &RemoteRequest) -> AuditContext {
    audit_context_for_remote_request(state, AuditSource::Web, AuditTransport::Http, request)
}

fn mcp_audit_context(state: &DaemonState, request: &RemoteRequest) -> AuditContext {
    audit_context_for_remote_request(state, AuditSource::Mcp, AuditTransport::Mcp, request)
}

#[allow(clippy::too_many_arguments)]
fn record_mcp_direct_audit(
    state: &DaemonState,
    params: &Value,
    operation: &str,
    response: &Response,
    started_at_ms: u64,
    duration_ms: u64,
    node: Option<&str>,
    session: Option<&str>,
    task: Option<&str>,
) {
    let mut context = AuditContext::new(AuditSource::Mcp, AuditTransport::Mcp);
    context.origin_node_id = Some(state.public_settings().node_id);
    let response_value = serde_json::to_value(response).unwrap_or_else(
        |error| json!({"serialization_error": error.to_string(), "ok": response.ok}),
    );
    let _ = record_audit_value(
        state,
        context,
        None,
        "mcp_tools_call",
        operation,
        session,
        task,
        AuditStatus::from_ok(response.ok),
        started_at_ms,
        duration_ms,
        params.clone(),
        response_value,
        json!({"node": node}),
        node.map(str::to_string)
            .or_else(|| Some(state.public_settings().node_id)),
    );
}

async fn list_sessions(
    State(state): State<DaemonState>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Response> {
    let node = match selected_node(&state, &query) {
        Ok(node) => node,
        Err(response) => return Json(response),
    };
    let request = RemoteRequest::ListSessions;
    Json(
        state
            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))
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
    let request = RemoteRequest::Snapshot { session, tail };
    Json(
        state
            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))
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
    let request = RemoteRequest::TaskLogs {
        session,
        task,
        after,
        limit,
    };
    Json(
        state
            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))
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
    let request = RemoteRequest::TaskMetrics {
        session,
        task,
        window_seconds,
    };
    Json(
        state
            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))
            .await,
    )
}

async fn clear_task_history(
    State(state): State<DaemonState>,
    Path((session, task)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Response> {
    let node = match selected_node(&state, &query) {
        Ok(node) => node,
        Err(response) => return Json(response),
    };
    let request = RemoteRequest::ClearTaskHistory { session, task };
    Json(
        state
            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))
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
    let request = RemoteRequest::GetSessionConfig { session };
    Json(
        state
            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))
            .await,
    )
}

#[derive(Deserialize)]
struct UpdateSessionConfigBody {
    revision: String,
    #[serde(default)]
    workspace_env: Option<std::collections::BTreeMap<String, String>>,
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
    let request = RemoteRequest::PutSessionConfig {
        session,
        revision: body.revision,
        workspace_env: body.workspace_env,
        tasks: body.tasks,
    };
    Json(
        state
            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))
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
    let request = RemoteRequest::Action {
        session: body.session,
        task: body.task,
        action: body.action,
    };
    Json(
        state
            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))
            .await,
    )
}

async fn list_audit(
    State(state): State<DaemonState>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Response> {
    let filter = match AuditFilter::parse(&query) {
        Ok(filter) => filter,
        Err(response) => return Json(response),
    };
    match state.store.list_audit(&filter) {
        Ok(page) => Json(Response::ok("audit records", page)),
        Err(error) => Json(Response::error(format!("{error:#}"))),
    }
}

async fn audit_detail(
    State(state): State<DaemonState>,
    Path(audit_id): Path<String>,
) -> Json<Response> {
    match state.store.audit_detail(&audit_id) {
        Ok(Some(record)) => Json(Response::ok("audit record", record)),
        Ok(None) => Json(Response::error_with_data(
            "audit record not found",
            json!({"kind": "not_found", "status": 404}),
        )),
        Err(error) => Json(Response::error(format!("{error:#}"))),
    }
}

async fn list_mcp_calls(
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

async fn mcp_call_detail(State(state): State<DaemonState>, Path(id): Path<u64>) -> Json<Response> {
    match state.store.mcp_call_detail(id) {
        Ok(Some(record)) => Json(Response::ok("MCP call", record)),
        Ok(None) => Json(Response::error(format!("MCP call '{id}' not found"))),
        Err(error) => Json(Response::error(format!("{error:#}"))),
    }
}

async fn list_task_runs(
    State(state): State<DaemonState>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Response> {
    let filter = match TaskRunFilter::parse(&query) {
        Ok(v) => v,
        Err(response) => return Json(response),
    };
    let node = match selected_node(&state, &query) {
        Ok(node) => node,
        Err(response) => return Json(response),
    };
    let request = RemoteRequest::ListTaskRuns { filter };
    Json(
        state
            .dispatch_node_with_audit(&node, request.clone(), web_audit_context(&state, &request))
            .await,
    )
}

async fn list_events_route(
    State(state): State<DaemonState>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Response> {
    let filter = match EventFilter::parse(&query) {
        Ok(v) => v,
        Err(response) => return Json(response),
    };
    match state.store.list_events(&filter) {
        Ok(page) => Json(Response::ok("events", page)),
        Err(error) => Json(Response::error(format!("{error:#}"))),
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

async fn call_mcp_tool(
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

const INDEX_HTML: &str = include_str!("web/index.html");
const STYLES_CSS: &str = include_str!("web/styles.css");
const APP_JS: &str = include_str!("web/app.js");

const LOGIN_HTML: &str = r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Taskdeck</title>
<style>:root{color-scheme:dark}body{margin:0;min-height:100vh;display:grid;place-items:center;background:#111315;color:#e8eaed;font-family:Inter,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}.card{width:min(380px,92vw);background:#191d20;border:1px solid #2b3237;padding:28px;border-radius:16px}h1{font-size:22px;margin:0 0 6px}p{margin:0;color:#9ba4a9;font-size:14px}form{display:flex;flex-direction:column;gap:14px;margin-top:24px}input{border-radius:10px;background:#22272b;border:1px solid #333b41;color:#fff;padding:11px 12px}button{background:#51c878;color:#04140a;border:0;border-radius:10px;padding:12px;font-weight:700}.form-error{color:#ff8080;margin-top:18px}</style></head>
<body><main class="card"><h1>Taskdeck</h1><p>Enter your access key to continue.</p>%LOGIN_ERROR%<form method="post" action="/login"><label for="access_key">Access key</label><input id="access_key" name="access_key" type="password" autocomplete="current-password" required autofocus><button>Unlock</button></form></main></body></html>"#;

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
        assert!(INDEX_HTML.contains("data-view=\"audit\""));
        assert!(INDEX_HTML.contains("data-view=\"workflows\""));
        assert!(INDEX_HTML.contains("id=\"workflows-view\""));
        assert!(INDEX_HTML.contains("id=\"workflow-groups\""));
        assert!(INDEX_HTML.contains("id=\"workflow-members\""));
        assert!(INDEX_HTML.contains("id=\"ungrouped-workspaces\""));
        assert!(INDEX_HTML.contains("id=\"audit-view\""));
        assert!(INDEX_HTML.contains("id=\"audit-dialog\""));
        assert!(INDEX_HTML.contains("id=\"config-dialog\""));
        assert!(INDEX_HTML.contains("data-view=\"settings\""));
        assert!(INDEX_HTML.contains("id=\"node-settings-form\""));
        assert!(INDEX_HTML.contains("id=\"alias-form\""));
        assert!(INDEX_HTML.contains("id=\"service-form\""));
        assert!(APP_JS.contains("loadNodeSettings"));
        assert!(APP_JS.contains("/api/workspaces"));
        assert!(APP_JS.contains("/api/workflow-groups"));
        assert!(APP_JS.contains("loadWorkflowGroups"));
        assert!(APP_JS.contains("data-workflow-action"));
        assert!(APP_JS.contains("/api/nodes/self/service"));
        assert!(!INDEX_HTML.contains("<style>"));
        assert!(!INDEX_HTML.contains("<script>const"));
        assert!(STYLES_CSS.contains("prefers-color-scheme: dark"));
        assert!(STYLES_CSS.contains("prefers-reduced-motion: reduce"));
        assert!(STYLES_CSS.contains("sidebar-collapsed"));
        assert!(STYLES_CSS.contains("workflow-layout"));
        assert!(STYLES_CSS.contains("workflow-result"));
        assert!(APP_JS.contains("/api/mcp-calls"));
        assert!(APP_JS.contains("/api/audit"));
        assert!(APP_JS.contains("loadAudit"));
        assert!(APP_JS.contains("Loading audit records"));
        assert!(APP_JS.contains("error-row"));
        assert!(APP_JS.contains("auditSyncLabel"));
        assert!(APP_JS.contains("/config"));
        assert!(APP_JS.contains("/logs?"));
        assert!(INDEX_HTML.contains("id=\"nodes\""));
        assert!(APP_JS.contains("new URLSearchParams({ window: \"600\" })"));
        assert!(APP_JS.contains("requestFullscreen"));
        assert!(APP_JS.contains("taskdeck-log-tail"));
        assert!(APP_JS.contains("taskdeck-seen-exits"));
        assert!(APP_JS.contains("restart_markers_ms"));
        assert!(INDEX_HTML.contains("data-call-mode=\"result\""));
        assert!(FAVICON_SVG.contains("<svg"));
    }

    #[test]
    fn mcp_drawer_preserves_human_readable_details_alongside_raw_payloads() {
        for id in [
            "detail-status-icon",
            "call-overview",
            "request-fields",
            "response-summary",
            "response-data",
            "call-request",
            "call-response",
        ] {
            assert!(
                INDEX_HTML.contains(&format!("id=\"{id}\"")),
                "missing MCP detail element #{id}"
            );
        }
        assert!(INDEX_HTML.contains("data-call-mode=\"result\""));
        assert!(INDEX_HTML.contains("data-call-mode=\"raw\""));
        assert!(STYLES_CSS.contains(".detail-status-icon"));
        assert!(STYLES_CSS.contains(".call-overview"));
        assert!(STYLES_CSS.contains(".field-row"));
        assert!(STYLES_CSS.contains(".outcome"));
        assert!(APP_JS.contains("requestFields(call, target)"));
        assert!(APP_JS.contains("renderResultData("));
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
                        clear_logs_on_restart: false,

                        schedule: None,
                    },
                )]),
                task_order: vec!["api".to_string()],
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
    async fn task_history_route_replaces_log_generation() {
        let app = app(metrics_test_state());
        let read_generation = |body: axum::body::Bytes| async move {
            let response: Response = serde_json::from_slice(&body).unwrap();
            response.data.unwrap()["generation"].as_u64().unwrap()
        };
        let before = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/sessions/demo/tasks/api/logs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let before = read_generation(to_bytes(before.into_body(), usize::MAX).await.unwrap()).await;

        let cleared = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("DELETE")
                    .uri("/api/sessions/demo/tasks/api/history")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let cleared: Response =
            serde_json::from_slice(&to_bytes(cleared.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert!(cleared.ok);

        let after = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/sessions/demo/tasks/api/logs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let after = read_generation(to_bytes(after.into_body(), usize::MAX).await.unwrap()).await;
        assert_ne!(before, after);
    }

    #[tokio::test]
    async fn worker_mcp_audit_records_self_target() {
        let state = DaemonState::new();
        let response = app(state.clone())
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "tools/call",
                        "params": {"name": "taskdeck_control", "arguments": {"action": "sessions"}}
                    }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response.status().is_success());
        assert_eq!(
            state
                .store
                .mcp_call_detail(1)
                .unwrap()
                .unwrap()
                .target_node
                .as_deref(),
            Some("self")
        );
    }

    #[tokio::test]
    async fn leader_mcp_audit_preserves_self_and_worker_targets() {
        let state = DaemonState::new();
        let settings = state
            .store
            .configure(crate::state::NodeSettingsUpdate {
                role: Some(crate::state::NodeRole::Leader),
                ..crate::state::NodeSettingsUpdate::default()
            })
            .unwrap();
        *state.settings.lock().expect("node settings lock") = settings;
        let app = app(state.clone());

        for (id, node) in [(1, "self"), (2, "worker-7")] {
            let response = app
                .clone()
                .oneshot(
                    HttpRequest::builder()
                        .method("POST")
                        .uri("/mcp")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "method": "tools/call",
                                "params": {
                                    "name": "taskdeck_control",
                                    "arguments": {"action": "sessions", "node": node}
                                }
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(response.status().is_success());
        }

        assert_eq!(
            state
                .store
                .mcp_call_detail(1)
                .unwrap()
                .unwrap()
                .target_node
                .as_deref(),
            Some("self")
        );
        assert_eq!(
            state
                .store
                .mcp_call_detail(2)
                .unwrap()
                .unwrap()
                .target_node
                .as_deref(),
            Some("worker-7")
        );
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

    fn audit_test_state() -> DaemonState {
        let state = DaemonState::new();
        for record in [
            test_audit_record(
                "audit-cli",
                AuditSource::Cli,
                AuditTransport::Ipc,
                AuditStatus::Success,
                10,
                "worker-1",
                "start",
                Some("alpha"),
                Some("api"),
                json!({"type":"action","note":"Needle Straße","authorization":"Bearer secret"}),
            ),
            test_audit_record(
                "audit-web",
                AuditSource::Web,
                AuditTransport::Http,
                AuditStatus::Error,
                20,
                "leader-1",
                "restart",
                Some("beta"),
                Some("web"),
                json!({"type":"action","body":{"token":"secret"}}),
            ),
            test_audit_record(
                "audit-scheduler",
                AuditSource::Scheduler,
                AuditTransport::Internal,
                AuditStatus::Success,
                30,
                "worker-2",
                "start",
                Some("gamma"),
                Some("etl"),
                json!({"type":"scheduler"}),
            ),
        ] {
            state.store.record_audit(record).unwrap();
        }
        state
    }

    #[allow(clippy::too_many_arguments)]
    fn test_audit_record(
        audit_id: &str,
        source: AuditSource,
        transport: AuditTransport,
        status: AuditStatus,
        timestamp_ms: u64,
        node_id: &str,
        operation: &str,
        session: Option<&str>,
        task: Option<&str>,
        request: Value,
    ) -> crate::protocol::AuditRecord {
        let success = matches!(status, AuditStatus::Success | AuditStatus::Started);
        crate::protocol::AuditRecord {
            audit_id: audit_id.to_string(),
            correlation_id: format!("corr-{audit_id}"),
            timestamp_ms,
            duration_ms: 7,
            source,
            transport,
            origin_node_id: Some(node_id.to_string()),
            executor_node_id: Some(node_id.to_string()),
            request_kind: if source == AuditSource::Scheduler {
                "scheduler"
            } else {
                "action"
            }
            .to_string(),
            operation: operation.to_string(),
            session: session.map(str::to_string),
            task: task.map(str::to_string),
            status,
            success,
            error: (!success).then(|| "boom".to_string()),
            request,
            response: json!({"ok": success, "message": if success { "ok" } else { "boom" }}),
            details: json!({"test": true}),
            replicated_at_ms: Some(timestamp_ms + 100),
        }
    }

    async fn list_audit_response(
        uri: &str,
        state: DaemonState,
    ) -> (Response, Option<crate::protocol::AuditListPage>) {
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
    async fn audit_route_filters_searches_paginates_and_returns_redacted_details() {
        let (_, page) = list_audit_response("/api/audit", audit_test_state()).await;
        let page = page.unwrap();
        assert_eq!(page.page, 1);
        assert_eq!(page.page_size, 20);
        assert_eq!(page.total, 3);
        assert_eq!(page.items[0].audit_id, "audit-scheduler");
        assert_eq!(page.items[1].audit_id, "audit-web");
        assert_eq!(page.items[2].audit_id, "audit-cli");

        let (_, search_page) =
            list_audit_response("/api/audit?q=STRASSE", audit_test_state()).await;
        let search_page = search_page.unwrap();
        assert_eq!(search_page.total, 1);
        assert_eq!(search_page.items[0].audit_id, "audit-cli");

        let (_, filtered_page) = list_audit_response(
            "/api/audit?source=web&status=error&node=leader-1&operation=restart&session=beta&task=web",
            audit_test_state(),
        )
        .await;
        let filtered_page = filtered_page.unwrap();
        assert_eq!(filtered_page.total, 1);
        assert_eq!(filtered_page.items[0].audit_id, "audit-web");
        assert!(!filtered_page.items[0].success);

        let detail = app(audit_test_state())
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/audit/audit-cli")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(detail.into_body(), usize::MAX).await.unwrap();
        let detail: Response = serde_json::from_slice(&body).unwrap();
        assert!(detail.ok);
        let record: crate::protocol::AuditRecord =
            serde_json::from_value(detail.data.unwrap()).unwrap();
        assert_eq!(record.request["authorization"], "[REDACTED]");
        assert_eq!(record.origin_node_id.as_deref(), Some("worker-1"));
        assert_eq!(record.executor_node_id.as_deref(), Some("worker-1"));

        let missing = app(audit_test_state())
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/audit/not-found")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(missing.into_body(), usize::MAX).await.unwrap();
        let missing: Response = serde_json::from_slice(&body).unwrap();
        assert!(!missing.ok);
        assert_eq!(missing.data.as_ref().unwrap()["status"], 404);

        let (invalid, _) = list_audit_response("/api/audit?source=nope", audit_test_state()).await;
        assert!(!invalid.ok);
        assert_eq!(invalid.data.as_ref().unwrap()["status"], 400);
    }

    #[tokio::test]
    async fn mcp_missing_action_is_recorded_in_unified_audit() {
        let state = DaemonState::new();
        let response = app(state.clone())
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "jsonrpc": "2.0",
                            "id": 7,
                            "method": "tools/call",
                            "params": {"name": "taskdeck_control", "arguments": {}}
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response.status().is_success());
        let (_, page) = list_audit_response(
            "/api/audit?source=mcp&status=error&operation=missing_action",
            state.clone(),
        )
        .await;
        let page = page.unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].request_kind, "mcp_tools_call");
        assert!(!state.store.mcp_call_detail(1).unwrap().unwrap().success);
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
            let _ = state.store.record_mcp_call(call);
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
            target_node: None,
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
        let _ = state.store.record_mcp_call(test_mcp_call(
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
        let _ = state.store.record_mcp_call(test_mcp_call(
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
        let _ = state.store.record_mcp_call(McpCallRecord {
            id: 0,
            tool: "taskdeck_control".to_string(),
            operation: Some("inspect".to_string()),
            started_at_ms: 77,
            duration_ms: 9,
            success: true,
            target_node: None,
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
            let _ = state.store.record_mcp_call(test_mcp_call(
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

    async fn http_route(
        state: DaemonState,
        method: &str,
        uri: &str,
        headers: &[(header::HeaderName, &str)],
        body: Option<&str>,
    ) -> axum::response::Response {
        let app = app(state);
        let mut builder = HttpRequest::builder().method(method).uri(uri);
        for (name, value) in headers {
            builder = builder.header(name, *value);
        }
        let body = Body::from(body.unwrap_or_default().to_owned());
        app.oneshot(builder.body(body).unwrap()).await.unwrap()
    }

    fn workflow_task_spec(label: &str) -> TaskSpec {
        TaskSpec {
            label: label.to_string(),
            program: "true".to_string(),
            args: Vec::new(),
            cwd: PathBuf::from("/tmp"),
            env: BTreeMap::new(),
            shell: false,
            auto_start: false,
            stop_timeout_ms: 500,
            clear_logs_on_restart: false,
            schedule: None,
        }
    }

    fn workflow_definition(session: &str, project: &str, tasks: &[&str]) -> ProjectDefinition {
        ProjectDefinition {
            session: session.to_string(),
            project: PathBuf::from(project),
            source: "taskdeck.yaml".to_string(),
            tasks: tasks
                .iter()
                .map(|label| ((*label).to_string(), workflow_task_spec(label)))
                .collect(),
            task_order: tasks.iter().map(|label| (*label).to_string()).collect(),
        }
    }

    fn insert_workflow_session(
        state: &DaemonState,
        session: &str,
        alias: Option<&str>,
        project: &str,
        tasks: &[&str],
    ) {
        state
            .store
            .upsert_registration(session, &PathBuf::from(project))
            .unwrap();
        if let Some(alias) = alias {
            state
                .store
                .set_registration_alias(session, Some(alias))
                .unwrap();
        }
        state.sessions.lock().expect("sessions lock").insert(
            session.to_string(),
            SessionRuntime::new(workflow_definition(session, project, tasks)),
        );
    }

    fn workflow_leader_state() -> DaemonState {
        let mut state = DaemonState::new();
        let settings = state
            .store
            .configure(crate::state::NodeSettingsUpdate {
                role: Some(crate::state::NodeRole::Leader),
                ..Default::default()
            })
            .unwrap();
        *state.settings.lock().expect("node settings lock") = settings;
        insert_workflow_session(&state, "api", Some("Backend API"), "/tmp/api", &["dev"]);
        insert_workflow_session(&state, "web", None, "/tmp/web", &["dev"]);

        let mut remote = SessionRuntime::new(workflow_definition(
            "worker-api",
            "/tmp/worker-api",
            &["deploy"],
        ));
        let remote_snapshot = remote.snapshot(0).unwrap();
        state
            .store
            .upsert_worker(
                "worker-7",
                "Worker 7",
                current_millis(),
                &serde_json::to_string(&vec![remote_snapshot]).unwrap(),
            )
            .unwrap();
        state.cluster = crate::cluster::LeaderCluster::new(state.store.clone(), None).unwrap();
        state
    }

    #[tokio::test]
    async fn workflow_groups_api_crud_resolves_targets_and_ungrouped() {
        let state = workflow_leader_state();
        let list = http_route(state.clone(), "GET", "/api/workflow-groups", &[], None).await;
        assert_eq!(list.status(), StatusCode::OK);
        let body = to_bytes(list.into_body(), usize::MAX).await.unwrap();
        let parsed: Response = serde_json::from_slice(&body).unwrap();
        let data = parsed.data.unwrap();
        assert_eq!(data["groups"].as_array().unwrap().len(), 0);
        assert_eq!(data["targets"].as_array().unwrap().len(), 3);
        assert_eq!(data["ungrouped"].as_array().unwrap().len(), 3);

        let body = json!({
            "name": "Release train",
            "members": [
                {"node_id":"self", "session":"api", "task":"dev"},
                {"node_id":"worker-7", "session":"worker-api", "task":"deploy"}
            ]
        })
        .to_string();
        let created = http_route(
            state.clone(),
            "POST",
            "/api/workflow-groups",
            &[(header::CONTENT_TYPE, "application/json")],
            Some(&body),
        )
        .await;
        assert_eq!(created.status(), StatusCode::OK);
        let body = to_bytes(created.into_body(), usize::MAX).await.unwrap();
        let parsed: Response = serde_json::from_slice(&body).unwrap();
        assert!(parsed.ok);
        let group = parsed.data.unwrap();
        let group_id = group["id"].as_str().unwrap().to_string();
        assert_eq!(group["members"][0]["workspace_display_name"], "Backend API");
        assert_eq!(group["members"][0]["available"], true);
        assert_eq!(group["members"][1]["node_online"], false);
        assert_eq!(group["members"][1]["skip_reason"], "node offline");

        let list = http_route(state.clone(), "GET", "/api/workflow-groups", &[], None).await;
        let body = to_bytes(list.into_body(), usize::MAX).await.unwrap();
        let parsed: Response = serde_json::from_slice(&body).unwrap();
        let data = parsed.data.unwrap();
        assert_eq!(data["groups"].as_array().unwrap().len(), 1);
        assert_eq!(data["ungrouped"].as_array().unwrap().len(), 1);
        assert_eq!(data["ungrouped"][0]["session"], "web");

        let update = json!({
            "name": "Frontend train",
            "members": [{"node_id":"self", "session":"web", "task":"dev"}]
        })
        .to_string();
        let updated = http_route(
            state.clone(),
            "PUT",
            &format!("/api/workflow-groups/{group_id}"),
            &[(header::CONTENT_TYPE, "application/json")],
            Some(&update),
        )
        .await;
        let body = to_bytes(updated.into_body(), usize::MAX).await.unwrap();
        let parsed: Response = serde_json::from_slice(&body).unwrap();
        assert!(parsed.ok);
        assert_eq!(parsed.data.unwrap()["name"], "Frontend train");

        let deleted = http_route(
            state,
            "DELETE",
            &format!("/api/workflow-groups/{group_id}"),
            &[],
            None,
        )
        .await;
        let body = to_bytes(deleted.into_body(), usize::MAX).await.unwrap();
        let parsed: Response = serde_json::from_slice(&body).unwrap();
        assert!(parsed.ok);
    }

    #[tokio::test]
    async fn workflow_groups_api_enforces_leader_scope_and_pure_master_self_rule() {
        let worker = DaemonState::new();
        let response = http_route(worker, "GET", "/api/workflow-groups", &[], None).await;
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let parsed: Response = serde_json::from_slice(&body).unwrap();
        assert!(!parsed.ok);
        assert_eq!(parsed.data.as_ref().unwrap()["status"], 403);

        let mut state = DaemonState::new();
        let settings = state
            .store
            .configure(crate::state::NodeSettingsUpdate {
                role: Some(crate::state::NodeRole::Leader),
                leader_mode: Some(crate::state::LeaderMode::PureMaster),
                ..Default::default()
            })
            .unwrap();
        *state.settings.lock().expect("node settings lock") = settings;
        state.cluster = crate::cluster::LeaderCluster::new(state.store.clone(), None).unwrap();
        let body = json!({
            "name": "Invalid",
            "members": [{"node_id":"self", "session":"api", "task":"dev"}]
        })
        .to_string();
        let response = http_route(
            state,
            "POST",
            "/api/workflow-groups",
            &[(header::CONTENT_TYPE, "application/json")],
            Some(&body),
        )
        .await;
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let parsed: Response = serde_json::from_slice(&body).unwrap();
        assert!(!parsed.ok);
        assert!(parsed.message.contains("pure master"));
    }

    #[tokio::test]
    async fn workflow_group_action_is_best_effort_and_audited() {
        let state = workflow_leader_state();
        let group = state
            .store
            .create_workflow_group(crate::protocol::WorkflowGroupInput {
                name: "Deploy".to_string(),
                members: vec![
                    crate::protocol::WorkflowGroupMember {
                        node_id: "self".to_string(),
                        session: "api".to_string(),
                        task: "dev".to_string(),
                    },
                    crate::protocol::WorkflowGroupMember {
                        node_id: "worker-7".to_string(),
                        session: "worker-api".to_string(),
                        task: "deploy".to_string(),
                    },
                    crate::protocol::WorkflowGroupMember {
                        node_id: "self".to_string(),
                        session: "api".to_string(),
                        task: "missing".to_string(),
                    },
                ],
                graph: crate::protocol::WorkflowGraph::default(),
            })
            .unwrap();
        let body = json!({"action":"stop"}).to_string();
        let response = http_route(
            state.clone(),
            "POST",
            &format!("/api/workflow-groups/{}/actions", group.id),
            &[(header::CONTENT_TYPE, "application/json")],
            Some(&body),
        )
        .await;
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let parsed: Response = serde_json::from_slice(&body).unwrap();
        assert!(parsed.ok);
        let data = parsed.data.unwrap();
        assert_eq!(data["success_count"], 1);
        assert_eq!(data["failed_count"], 0);
        assert_eq!(data["skipped_count"], 2);
        assert_eq!(data["results"][0]["status"], "success");
        assert_eq!(data["results"][1]["message"], "node offline");
        assert_eq!(data["results"][2]["message"], "task not found");

        let body = json!({"action":"pause"}).to_string();
        let response = http_route(
            state.clone(),
            "POST",
            &format!("/api/workflow-groups/{}/actions", group.id),
            &[(header::CONTENT_TYPE, "application/json")],
            Some(&body),
        )
        .await;
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let parsed: Response = serde_json::from_slice(&body).unwrap();
        assert!(parsed.ok);
        let data = parsed.data.unwrap();
        assert_eq!(data["failed_count"], 1);
        assert_eq!(data["results"][0]["status"], "failed");

        let page = state
            .store
            .list_audit(&crate::protocol::AuditFilter {
                q: None,
                source: Some("web".to_string()),
                status: None,
                node: None,
                session: None,
                task: None,
                operation: Some("workflow_group_action".to_string()),
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(page.total, 2);
    }

    #[tokio::test]
    async fn workspaces_api_lists_and_updates_aliases_without_changing_session_id() {
        let state = DaemonState::new();
        state
            .store
            .upsert_registration("api", &PathBuf::from("/tmp/api"))
            .unwrap();
        let response = http_route(state.clone(), "GET", "/api/workspaces", &[], None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let parsed: Response = serde_json::from_slice(&body).unwrap();
        let data = parsed.data.unwrap();
        assert_eq!(data[0]["session"], "api");
        assert_eq!(data[0]["display_name"], "api");
        let response = http_route(
            state,
            "PUT",
            "/api/workspaces/api/alias",
            &[(header::CONTENT_TYPE, "application/json")],
            Some(r#"{"alias":"Backend API"}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let parsed: Response = serde_json::from_slice(&body).unwrap();
        let data = parsed.data.unwrap();
        assert_eq!(data["session"], "api");
        assert_eq!(data["alias"], "Backend API");
        assert_eq!(data["display_name"], "Backend API");
    }

    #[tokio::test]
    async fn node_settings_api_keeps_token_hidden_and_reports_restart() {
        let state = DaemonState::new();
        let response =
            http_route(state.clone(), "GET", "/api/nodes/self/settings", &[], None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let parsed: Response = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.data.unwrap()["has_enrollment_token"], false);
        let body = serde_json::json!({
            "bind_host": "127.0.0.1",
            "web_port": 9937,
            "enrollment_token": {"mode":"set","value":"secret"}
        })
        .to_string();
        let response = http_route(
            state,
            "PUT",
            "/api/nodes/self/settings",
            &[(header::CONTENT_TYPE, "application/json")],
            Some(&body),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&body).to_string();
        assert!(!text.contains("secret"));
        let parsed: Response = serde_json::from_slice(&body).unwrap();
        let data = parsed.data.unwrap();
        assert_eq!(data["restart_required"], true);
        assert_eq!(data["settings"]["has_enrollment_token"], true);
    }

    #[tokio::test]
    async fn workspaces_route_uses_cached_aliases_for_offline_worker() {
        let mut state = DaemonState::new();
        let settings = state
            .store
            .configure(crate::state::NodeSettingsUpdate {
                role: Some(crate::state::NodeRole::Leader),
                ..Default::default()
            })
            .unwrap();
        *state.settings.lock().expect("node settings lock") = settings;
        let snapshot = crate::protocol::SessionSnapshot {
            name: "api".to_string(),
            alias: Some("Backend API".to_string()),
            project: PathBuf::from("/tmp/api"),
            source: "taskdeck.yaml".to_string(),
            tasks: Default::default(),
            task_order: Vec::new(),
        };
        state
            .store
            .upsert_worker(
                "worker-7",
                "Worker",
                current_millis(),
                &serde_json::to_string(&vec![snapshot]).unwrap(),
            )
            .unwrap();
        state.cluster = crate::cluster::LeaderCluster::new(state.store.clone(), None).unwrap();
        let response = http_route(state, "GET", "/api/workspaces?node=worker-7", &[], None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let parsed: Response = serde_json::from_slice(&body).unwrap();
        let data = parsed.data.unwrap();
        assert_eq!(data[0]["session"], "api");
        assert_eq!(data[0]["alias"], "Backend API");
        assert_eq!(data[0]["display_name"], "Backend API");
    }

    #[tokio::test]
    async fn node_settings_and_service_actions_are_audited_with_token_redaction() {
        let state = DaemonState::new();
        let body = serde_json::json!({
            "web_port": 9937,
            "enrollment_token": {"mode":"set","value":"secret"}
        })
        .to_string();
        let response = http_route(
            state.clone(),
            "PUT",
            "/api/nodes/self/settings",
            &[(header::CONTENT_TYPE, "application/json")],
            Some(&body),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = serde_json::json!({"action":"status","scope":"user"}).to_string();
        let response = http_route(
            state.clone(),
            "POST",
            "/api/nodes/self/service",
            &[(header::CONTENT_TYPE, "application/json")],
            Some(&body),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let settings_page = state
            .store
            .list_audit(&AuditFilter {
                q: None,
                source: Some("web".to_string()),
                status: None,
                node: None,
                session: None,
                task: None,
                operation: Some("put_node_settings".to_string()),
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(settings_page.total, 1);
        let settings_detail = state
            .store
            .audit_detail(&settings_page.items[0].audit_id)
            .unwrap()
            .unwrap();
        let request_text = serde_json::to_string(&settings_detail.request).unwrap();
        assert!(!request_text.contains("secret"));
        let service_page = state
            .store
            .list_audit(&AuditFilter {
                q: None,
                source: Some("web".to_string()),
                status: None,
                node: None,
                session: None,
                task: None,
                operation: Some("status".to_string()),
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(service_page.total, 1);
        assert_eq!(service_page.items[0].operation, "status");
    }

    #[tokio::test]
    async fn auth_middleware_allows_disabled_and_protects_enabled_control_plane() {
        let state = DaemonState::new();
        let response = http_route(state.clone(), "GET", "/api/nodes", &[], None).await;
        assert_eq!(response.status(), StatusCode::OK);

        state.store.set_access_key("test-access-key").unwrap();
        state.store.configure_auth(true).unwrap();
        let unauthorized = http_route(state.clone(), "GET", "/api/nodes", &[], None).await;
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        let login = http_route(state, "GET", "/", &[], None).await;
        assert_eq!(login.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn login_page_hides_the_error_placeholder_until_login_fails() {
        let state = DaemonState::new();
        state.store.set_access_key("test-access-key").unwrap();
        state.store.configure_auth(true).unwrap();
        let response = http_route(state, "GET", "/login", &[], None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8_lossy(&body);
        assert!(body.contains("Enter your access key"));
        assert!(!body.contains("%LOGIN_ERROR%"));
    }

    #[tokio::test]
    async fn auth_login_creates_a_session_cookie_accepted_by_api() {
        let state = DaemonState::new();
        state.store.set_access_key("test-access-key").unwrap();
        state.store.configure_auth(true).unwrap();
        let wrong = async_body_login(state.clone(), "bad").await;
        assert_eq!(wrong.status(), StatusCode::OK); // login page with error body
        let correct = async_body_login(state.clone(), "test-access-key").await;
        assert_eq!(correct.status(), StatusCode::SEE_OTHER);
        let cookie = correct
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .unwrap();
        let token = cookie
            .split(';')
            .next()
            .unwrap()
            .split('=')
            .nth(1)
            .unwrap()
            .to_string();
        let name = header::HeaderName::from_static("cookie");
        let cookie_value = format!("{AUTH_COOKIE}={token}");
        let response = http_route(
            state,
            "GET",
            "/api/nodes",
            &[(name, cookie_value.as_str())],
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    async fn async_body_login(state: DaemonState, key: &str) -> axum::response::Response {
        let body = Body::from(format!("access_key={key}"));
        let request = HttpRequest::builder()
            .method("POST")
            .uri("/login")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body)
            .unwrap();
        app(state).oneshot(request).await.unwrap()
    }

    async fn post_json(state: DaemonState, uri: &str, body: serde_json::Value) -> Response {
        let response = http_route(
            state,
            "POST",
            uri,
            &[(header::CONTENT_TYPE, "application/json")],
            Some(&body.to_string()),
        )
        .await;
        parse_json_response(response).await
    }

    async fn get_json(state: DaemonState, uri: &str) -> Response {
        let response = http_route(state, "GET", uri, &[], None).await;
        parse_json_response(response).await
    }

    async fn request_json(
        state: DaemonState,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> Response {
        let response = http_route(
            state,
            method,
            uri,
            &[(header::CONTENT_TYPE, "application/json")],
            body.map(|value| value.to_string()).as_deref(),
        )
        .await;
        parse_json_response(response).await
    }

    async fn parse_json_response(response: axum::response::Response) -> Response {
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn workflow_revisions_record_history_and_restore() {
        let state = workflow_leader_state();
        let created = post_json(
            state.clone(),
            "/api/workflow-groups",
            json!({
                "name": "Pipeline",
                "members": [
                    {"node_id":"self", "session":"api", "task":"dev"}
                ],
                "graph": {"positions": [{"x":1.0,"y":2.0}], "edges": []}
            }),
        )
        .await;
        assert!(created.ok, "{}", created.message);
        let group_id = created.data.unwrap()["id"].as_str().unwrap().to_string();

        let updated = request_json(
            state.clone(),
            "PUT",
            &format!("/api/workflow-groups/{group_id}"),
            Some(json!({
                "name": "Pipeline v2",
                "members": [
                    {"node_id":"self", "session":"api", "task":"dev"}
                ]
            })),
        )
        .await;
        assert!(updated.ok, "{}", updated.message);

        let revisions = get_json(
            state.clone(),
            &format!("/api/workflow-groups/{group_id}/revisions"),
        )
        .await;
        assert!(revisions.ok, "{}", revisions.message);
        let data = revisions.data.unwrap();
        assert_eq!(data["group_id"], group_id);
        let items = data["revisions"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["revision"], 2);
        assert_eq!(items[0]["name"], "Pipeline v2");
        assert_eq!(items[1]["revision"], 1);
        assert_eq!(items[1]["graph"]["positions"][0]["x"], 1.0);

        let restored = post_json(
            state.clone(),
            &format!("/api/workflow-groups/{group_id}/revisions/1/restore"),
            json!({}),
        )
        .await;
        assert!(restored.ok, "{}", restored.message);
        let data = restored.data.unwrap();
        assert_eq!(data["name"], "Pipeline");

        let revisions = get_json(
            state.clone(),
            &format!("/api/workflow-groups/{group_id}/revisions"),
        )
        .await;
        let revisions_data = revisions.data.unwrap();
        let items = revisions_data["revisions"].as_array().unwrap();
        assert_eq!(items[0]["revision"], 3);
        assert_eq!(
            items[0]["note"].as_str().unwrap(),
            "restored from revision 1"
        );

        let missing = post_json(
            state.clone(),
            "/api/workflow-groups/pipeline/revisions/99/restore",
            json!({}),
        )
        .await;
        assert!(!missing.ok);
    }

    #[tokio::test]
    async fn workflow_run_follows_graph_order_and_stop_on_failure() {
        let state = workflow_leader_state();
        let created = post_json(
            state.clone(),
            "/api/workflow-groups",
            json!({
                "name": "Ordered",
                "members": [
                    {"node_id":"self", "session":"api", "task":"dev"},
                    {"node_id":"worker-7", "session":"worker-api", "task":"deploy"},
                    {"node_id":"self", "session":"web", "task":"dev"}
                ],
                "graph": {"edges": [{"from":0,"to":1},{"from":1,"to":2}]}
            }),
        )
        .await;
        assert!(created.ok, "{}", created.message);
        let group_id = created.data.unwrap()["id"].as_str().unwrap().to_string();

        let run = post_json(
            state.clone(),
            &format!("/api/workflow-groups/{group_id}/run"),
            json!({}),
        )
        .await;
        assert!(run.ok, "{}", run.message);
        let summary = run.data.unwrap();
        let results = summary["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["task"], "dev");
        assert_eq!(results[0]["session"], "api");
        assert_eq!(results[0]["status"], "success");
        assert_eq!(results[1]["task"], "deploy");
        assert_eq!(results[1]["status"], "skipped");

        let run_all = post_json(
            state.clone(),
            &format!("/api/workflow-groups/{group_id}/run"),
            json!({"stop_on_failure": false}),
        )
        .await;
        let summary = run_all.data.unwrap();
        let results = summary["results"].as_array().unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[2]["task"], "dev");
        assert_eq!(results[2]["session"], "web");
    }

    #[tokio::test]
    async fn quotas_api_crud_and_validation() {
        let mut state = DaemonState::new();
        insert_workflow_session(&state, "api", None, "/tmp/api", &["dev"]);
        let created = post_json(
            state.clone(),
            "/api/quotas",
            json!({"session": "api", "max_running_tasks": 2}),
        )
        .await;
        assert!(created.ok, "{}", created.message);
        assert_eq!(created.data.unwrap()["session"], "api");

        let node_quota = post_json(
            state.clone(),
            "/api/quotas",
            json!({"max_running_tasks": 8}),
        )
        .await;
        assert!(node_quota.ok, "{}", node_quota.message);

        let duplicate = post_json(
            state.clone(),
            "/api/quotas",
            json!({"session": "api", "max_running_tasks": 3}),
        )
        .await;
        assert!(!duplicate.ok);

        let invalid = post_json(
            state.clone(),
            "/api/quotas",
            json!({"max_running_tasks": 0}),
        )
        .await;
        assert!(!invalid.ok);

        let list = get_json(state.clone(), "/api/quotas").await;
        let data = list.data.unwrap();
        assert_eq!(data["quotas"].as_array().unwrap().len(), 2);
        assert!(
            data["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s == "api")
        );

        let quota_id = data["quotas"][0]["id"].as_str().unwrap().to_string();
        let updated = request_json(
            state.clone(),
            "PUT",
            &format!("/api/quotas/{quota_id}"),
            Some(json!({"session": "web", "max_running_tasks": 5})),
        )
        .await;
        assert!(updated.ok, "{}", updated.message);

        let deleted = request_json(
            state.clone(),
            "DELETE",
            &format!("/api/quotas/{quota_id}"),
            None,
        )
        .await;
        assert!(deleted.ok, "{}", deleted.message);
        assert!(state.check_quotas("api").is_ok());
    }

    #[tokio::test]
    async fn notification_rules_and_notifications_round_trip() {
        let state = DaemonState::new();
        let rule = post_json(
            state.clone(),
            "/api/notification-rules",
            json!({
                "name": "failures",
                "event_types": ["task_failed"],
                "scope_session": "api",
                "webhook_url": "https://example.com/hook"
            }),
        )
        .await;
        assert!(rule.ok, "{}", rule.message);
        let rule_id = rule.data.unwrap()["id"].as_str().unwrap().to_string();

        let invalid = post_json(
            state.clone(),
            "/api/notification-rules",
            json!({"name": "bad", "event_types": ["explosion"]}),
        )
        .await;
        assert!(!invalid.ok);

        let empty = get_json(state.clone(), "/api/notifications").await;
        assert_eq!(empty.data.unwrap()["unread_count"], 0);

        state
            .store
            .insert_notification(
                "self-node",
                Some(&rule_id),
                Some("failures"),
                "task_failed",
                "critical",
                Some("api"),
                Some("dev"),
                "task failed: dev",
                "dev exited with code 1",
                &json!({"exit_code": 1}),
            )
            .unwrap();

        let listed = get_json(state.clone(), "/api/notifications").await;
        let data = listed.data.unwrap();
        assert_eq!(data["notifications"].as_array().unwrap().len(), 1);
        assert_eq!(data["unread_count"], 1);

        let read = post_json(
            state.clone(),
            "/api/notifications/read",
            json!({"all": true}),
        )
        .await;
        assert!(read.ok, "{}", read.message);
        let listed = get_json(state.clone(), "/api/notifications").await;
        assert_eq!(listed.data.unwrap()["unread_count"], 0);

        let updated = request_json(
            state.clone(),
            "PUT",
            &format!("/api/notification-rules/{rule_id}"),
            Some(json!({
                "name": "failures",
                "event_types": ["task_failed", "task_stopped"],
                "enabled": false
            })),
        )
        .await;
        assert!(updated.ok, "{}", updated.message);
        assert!(!updated.data.unwrap()["enabled"].as_bool().unwrap());

        let deleted = request_json(
            state.clone(),
            "DELETE",
            &format!("/api/notification-rules/{rule_id}"),
            None,
        )
        .await;
        assert!(deleted.ok, "{}", deleted.message);
    }

    #[tokio::test]
    async fn api_tokens_authenticate_external_clients() {
        let state = DaemonState::new();
        let created = post_json(state.clone(), "/api/tokens", json!({"name": "ci"})).await;
        assert!(created.ok, "{}", created.message);
        let data = created.data.unwrap();
        let secret = data["secret"].as_str().unwrap().to_string();
        let token_id = data["id"].as_str().unwrap().to_string();
        assert!(secret.starts_with("tdk_"));

        let listed = get_json(state.clone(), "/api/tokens").await;
        let listed_data = listed.data.unwrap();
        let tokens = listed_data["tokens"].as_array().unwrap();
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].get("secret").is_none());

        state.store.set_access_key("test-access-key").unwrap();
        state.store.configure_auth(true).unwrap();

        let unauthorized = http_route(state.clone(), "GET", "/api/quotas", &[], None).await;
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let authorized = http_route(
            state.clone(),
            "GET",
            "/api/quotas",
            &[(header::AUTHORIZATION, &format!("Bearer {secret}"))],
            None,
        )
        .await;
        assert_eq!(authorized.status(), StatusCode::OK);

        let revoked = http_route(
            state.clone(),
            "DELETE",
            &format!("/api/tokens/{token_id}"),
            &[(header::AUTHORIZATION, &format!("Bearer {secret}"))],
            None,
        )
        .await;
        assert_eq!(revoked.status(), StatusCode::OK);
        let rejected = http_route(
            state,
            "GET",
            "/api/quotas",
            &[(header::AUTHORIZATION, &format!("Bearer {secret}"))],
            None,
        )
        .await;
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn board_templates_create_apply_export_import() {
        let state = workflow_leader_state();
        let board = post_json(
            state.clone(),
            "/api/boards",
            json!({
                "name": "Ops",
                "cards": [{"node_id":"self", "session":"api", "task":"dev", "mode":"logs", "pinned": true}]
            }),
        )
        .await;
        assert!(board.ok, "{}", board.message);
        let board_id = board.data.unwrap()["id"].as_str().unwrap().to_string();

        let template = post_json(
            state.clone(),
            "/api/board-templates",
            json!({"name": "Ops template", "source_board_id": board_id}),
        )
        .await;
        assert!(template.ok, "{}", template.message);
        let data = template.data.unwrap();
        let template_id = data["id"].as_str().unwrap().to_string();
        assert_eq!(data["cards"].as_array().unwrap().len(), 1);

        let applied = post_json(
            state.clone(),
            &format!("/api/board-templates/{template_id}/apply"),
            json!({"name": "Ops clone"}),
        )
        .await;
        assert!(applied.ok, "{}", applied.message);
        assert_eq!(applied.data.unwrap()["name"], "Ops clone");

        let export = get_json(
            state.clone(),
            &format!("/api/board-templates/{template_id}/export"),
        )
        .await;
        let exported = export.data.unwrap();
        assert_eq!(exported["kind"], "taskdeck_board_template");

        let deleted = request_json(
            state.clone(),
            "DELETE",
            &format!("/api/board-templates/{template_id}"),
            None,
        )
        .await;
        assert!(deleted.ok, "{}", deleted.message);

        let imported = post_json(state.clone(), "/api/board-templates/import", exported).await;
        assert!(imported.ok, "{}", imported.message);

        let listed = get_json(state.clone(), "/api/board-templates").await;
        let listed_data = listed.data.unwrap();
        let templates = listed_data["templates"].as_array().unwrap();
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0]["name"], "Ops template");
    }

    #[tokio::test]
    async fn dependencies_api_validates_scope_and_cycles() {
        let state = workflow_leader_state();
        let created = post_json(
            state.clone(),
            "/api/dependencies",
            json!({
                "node_id": "self", "session": "api", "task": "dev",
                "depends_node_id": "worker-7", "depends_session": "worker-api", "depends_task": "deploy"
            }),
        )
        .await;
        assert!(created.ok, "{}", created.message);
        let dependency = created.data.unwrap();
        assert_eq!(dependency["required_state"], "running");
        assert_eq!(dependency["target_exists"], true);
        let dependency_id = dependency["id"].as_str().unwrap().to_string();

        let duplicate = post_json(
            state.clone(),
            "/api/dependencies",
            json!({
                "node_id": "self", "session": "api", "task": "dev",
                "depends_node_id": "worker-7", "depends_session": "worker-api", "depends_task": "deploy"
            }),
        )
        .await;
        assert!(!duplicate.ok);

        let cycle = post_json(
            state.clone(),
            "/api/dependencies",
            json!({
                "node_id": "worker-7", "session": "worker-api", "task": "deploy",
                "depends_node_id": "self", "depends_session": "api", "depends_task": "dev"
            }),
        )
        .await;
        assert!(!cycle.ok);
        assert!(cycle.message.contains("cycle"));

        let unknown = post_json(
            state.clone(),
            "/api/dependencies",
            json!({
                "node_id": "self", "session": "api", "task": "dev",
                "depends_node_id": "ghost", "depends_session": "x", "depends_task": "y"
            }),
        )
        .await;
        assert!(!unknown.ok);

        let list = get_json(state.clone(), "/api/dependencies").await;
        let data = list.data.unwrap();
        assert_eq!(data["dependencies"].as_array().unwrap().len(), 1);
        assert!(!data["targets"].as_array().unwrap().is_empty());

        let deleted = request_json(
            state.clone(),
            "DELETE",
            &format!("/api/dependencies/{dependency_id}"),
            None,
        )
        .await;
        assert!(deleted.ok, "{}", deleted.message);
    }

    #[tokio::test]
    async fn node_metrics_reports_nodes_and_status_counts() {
        let state = workflow_leader_state();
        let listed = get_json(state.clone(), "/api/node-metrics").await;
        assert!(listed.ok, "{}", listed.message);
        let data = listed.data.unwrap();
        let nodes = data["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 2);
        let self_entry = nodes.iter().find(|node| node["node_id"] == "self").unwrap();
        assert!(self_entry["is_self"].as_bool().unwrap());
        assert_eq!(self_entry["task_status_counts"]["idle"], 2);
        assert!(data["task_status_counts"]["idle"].as_u64().unwrap() >= 3);
    }

    #[tokio::test]
    async fn scaling_policies_api_crud_and_validation() {
        let state = workflow_leader_state();
        let created = post_json(
            state.clone(),
            "/api/scaling-policies",
            json!({
                "name": "api autoscale",
                "watch_node_id": "self",
                "watch_session": "api",
                "watch_task": "dev",
                "metric": "cpu_percent",
                "scale_out_threshold": 80.0,
                "scale_in_threshold": 20.0,
                "scale_out_node_id": "self",
                "scale_out_session": "api",
                "scale_out_task": "dev-replica",
                "cooldown_seconds": 60
            }),
        )
        .await;
        assert!(created.ok, "{}", created.message);
        let policy_id = created.data.unwrap()["id"].as_str().unwrap().to_string();

        let invalid = post_json(
            state.clone(),
            "/api/scaling-policies",
            json!({
                "name": "bad",
                "watch_node_id": "self",
                "watch_session": "api",
                "watch_task": "dev",
                "metric": "cpu_percent",
                "scale_out_threshold": 20.0,
                "scale_in_threshold": 80.0,
                "scale_out_node_id": "self",
                "scale_out_session": "api",
                "scale_out_task": "dev-replica"
            }),
        )
        .await;
        assert!(!invalid.ok);

        let list = get_json(state.clone(), "/api/scaling-policies").await;
        let data = list.data.unwrap();
        assert_eq!(data["policies"].as_array().unwrap().len(), 1);
        assert!(!data["targets"].as_array().unwrap().is_empty());

        let updated = request_json(
            state.clone(),
            "PUT",
            &format!("/api/scaling-policies/{policy_id}"),
            Some(json!({
                "name": "api autoscale v2",
                "watch_node_id": "self",
                "watch_session": "api",
                "watch_task": "dev",
                "metric": "memory_bytes",
                "scale_out_threshold": 1000000000.0,
                "scale_in_threshold": 100000000.0,
                "scale_out_node_id": "self",
                "scale_out_session": "api",
                "scale_out_task": "dev-replica"
            })),
        )
        .await;
        assert!(updated.ok, "{}", updated.message);

        let deleted = request_json(
            state.clone(),
            "DELETE",
            &format!("/api/scaling-policies/{policy_id}"),
            None,
        )
        .await;
        assert!(deleted.ok, "{}", deleted.message);
    }
}
