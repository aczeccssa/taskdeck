//! Task log/history/metrics route tests.

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

pub(super) fn metrics_test_state() -> DaemonState {
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

pub(super) async fn task_logs_route_returns_incremental_payload_and_validates_queries() {
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

pub(super) async fn task_history_route_replaces_log_generation() {
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
        serde_json::from_slice(&to_bytes(cleared.into_body(), usize::MAX).await.unwrap()).unwrap();

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
