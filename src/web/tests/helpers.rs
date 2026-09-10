//! Shared HTTP test helpers.

use std::collections::BTreeMap;
use std::path::PathBuf;

use axum::body::{Body, to_bytes};
use axum::http::Request as HttpRequest;
use serde_json::json;
use tower::util::ServiceExt;

use super::super::*;
use crate::config::{ProjectDefinition, TaskSpec};
use crate::runtime::SessionRuntime;

pub(super) async fn http_route(
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

pub(super) fn workflow_task_spec(label: &str) -> TaskSpec {
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

pub(super) fn workflow_definition(
    session: &str,
    project: &str,
    tasks: &[&str],
) -> ProjectDefinition {
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

pub(super) fn insert_workflow_session(
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

pub(super) fn workflow_leader_state() -> DaemonState {
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

pub(super) async fn async_body_login(state: DaemonState, key: &str) -> axum::response::Response {
    let body = Body::from(format!("access_key={key}"));

    let request = HttpRequest::builder()
        .method("POST")
        .uri("/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .unwrap();

    app(state).oneshot(request).await.unwrap()
}

pub(super) async fn post_json(state: DaemonState, uri: &str, body: serde_json::Value) -> Response {
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

pub(super) async fn get_json(state: DaemonState, uri: &str) -> Response {
    let response = http_route(state, "GET", uri, &[], None).await;

    parse_json_response(response).await
}

pub(super) async fn request_json(
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

pub(super) async fn parse_json_response(response: axum::response::Response) -> Response {
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();

    serde_json::from_slice(&body).unwrap()
}
