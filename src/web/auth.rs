//! Auth middleware, login/logout and cookie helpers.

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
use super::index;
#[derive(Deserialize)]
pub(crate) struct LoginBody {

    access_key: String,

}



pub const AUTH_COOKIE: &str = "taskdeck_session";



pub(crate) fn bearer_token(headers: &HeaderMap) -> Option<String> {

    headers

        .get(header::AUTHORIZATION)?

        .to_str()

        .ok()?

        .strip_prefix("Bearer ")

        .map(str::trim)

        .filter(|v| !v.is_empty())

        .map(str::to_string)

}



pub(crate) fn session_cookie(headers: &HeaderMap) -> Option<String> {

    let value = headers.get(header::COOKIE)?.to_str().ok()?;

    value.split(';').find_map(|part| {

        let part = part.trim();

        part.strip_prefix(AUTH_COOKIE)

            .and_then(|rest| rest.strip_prefix('='))

            .filter(|token| !token.is_empty())

            .map(|token| token.to_string())

    })

}



pub(crate) async fn auth_middleware(

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

    let exempt = path == "/"

        || path == "/login"

        || path == "/health"

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

        return index().await;

    }

    (

        StatusCode::UNAUTHORIZED,

        Json(json!({"kind":"unauthorized","status":401})),

    )

        .into_response()

}



pub(crate) async fn login_submit(

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

            return index().await;

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

        Err(_) => index().await,

    }

}



pub(crate) async fn logout(State(state): State<DaemonState>, headers: HeaderMap) -> AxumResponse {

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



pub(crate) async fn auth_status(State(state): State<DaemonState>, headers: HeaderMap) -> Json<Response> {

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
