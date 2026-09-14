//! Workspace/node settings endpoint tests.

use std::collections::BTreeMap;
use std::path::PathBuf;

use axum::body::{Body, to_bytes};
use axum::http::Request as HttpRequest;
use serde_json::json;
use tower::util::ServiceExt;

use super::super::*;
use super::helpers::*;
use crate::config::{ProjectDefinition, TaskSpec};
use crate::runtime::SessionRuntime;

#[tokio::test]

pub(super) async fn workspaces_api_lists_and_updates_aliases_without_changing_session_id() {
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
pub(super) async fn workspaces_api_removes_an_unavailable_registration() {
    let state = DaemonState::new();
    state
        .store
        .upsert_registration("stale", &PathBuf::from("/tmp/missing-project"))
        .unwrap();
    state
        .unavailable_sessions
        .lock()
        .unwrap()
        .insert(
            "stale".to_string(),
            crate::daemon::UnavailableSession {
                session: "stale".to_string(),
                project: PathBuf::from("/tmp/missing-project"),
                error: "project directory does not exist".to_string(),
            },
        );

    let response = http_route(state.clone(), "DELETE", "/api/workspaces/stale", &[], None).await;
    let parsed = parse_json_response(response).await;
    assert!(parsed.ok, "{}", parsed.message);
    assert!(state.store.registrations().unwrap().is_empty());
    assert!(state.unavailable_sessions.lock().unwrap().is_empty());
}

#[tokio::test]

pub(super) async fn node_settings_api_keeps_token_hidden_and_reports_restart() {
    let state = DaemonState::new();

    let response = http_route(state.clone(), "GET", "/api/nodes/self/settings", &[], None).await;

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

pub(super) async fn workspaces_route_uses_cached_aliases_for_offline_worker() {
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

pub(super) async fn connected_nodes_manager_refuses_self_disconnect_and_forgets_remote_workers() {
    let state = workflow_leader_state();

    let response = http_route(state.clone(), "DELETE", "/api/nodes/self", &[], None).await;
    let parsed = parse_json_response(response).await;
    assert_eq!(parsed.ok, false);
    assert!(state.store.known_workers().unwrap().iter().any(|worker| worker.node_id == "worker-7"));

    let response = http_route(state.clone(), "DELETE", "/api/nodes/worker-7", &[], None).await;
    assert_eq!(response.status(), StatusCode::OK);

    let parsed = parse_json_response(response).await;
    assert_eq!(parsed.ok, true);
    assert_eq!(parsed.data.unwrap()["disconnected"], true);
    assert!(!state.store.known_workers().unwrap().iter().any(|worker| worker.node_id == "worker-7"));
}

#[tokio::test]

pub(super) async fn node_settings_and_service_actions_are_audited_with_token_redaction() {
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
