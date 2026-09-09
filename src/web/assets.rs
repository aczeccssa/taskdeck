//! Embedded frontend asset serving.

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
pub(crate) struct EmbeddedAsset {

    pub(crate) path: &'static str,

    pub(crate) bytes: &'static [u8],

}



include!(concat!(env!("OUT_DIR"), "/embedded_assets.rs"));



pub(crate) async fn index() -> AxumResponse {

    embedded_asset_response("/index.html", false)

}



pub(crate) async fn static_asset(Path(asset_path): Path<String>) -> AxumResponse {

    if asset_path.contains("..") {

        return StatusCode::NOT_FOUND.into_response();

    }

    embedded_asset_response(&format!("/{asset_path}"), asset_path.starts_with("assets/"))

}



pub(crate) fn embedded_asset_response(path: &str, immutable: bool) -> AxumResponse {

    let Some(asset) = EMBEDDED_ASSETS.iter().find(|asset| asset.path == path) else {

        return StatusCode::NOT_FOUND.into_response();

    };

    let content_type = match path.rsplit('.').next().unwrap_or_default() {

        "html" => "text/html; charset=utf-8",

        "css" => "text/css; charset=utf-8",

        "js" | "mjs" => "application/javascript; charset=utf-8",

        "json" => "application/json; charset=utf-8",

        "svg" => "image/svg+xml",

        "png" => "image/png",

        "jpg" | "jpeg" => "image/jpeg",

        "webp" => "image/webp",

        "woff2" => "font/woff2",

        "woff" => "font/woff",

        _ => "application/octet-stream",

    };

    let cache_control = if immutable {

        "public, max-age=31536000, immutable"

    } else {

        "no-cache"

    };

    AxumResponse::builder()

        .header(header::CONTENT_TYPE, content_type)

        .header(header::CACHE_CONTROL, cache_control)

        .body(Body::from(asset.bytes.to_vec()))

        .expect("valid embedded frontend response")

}
