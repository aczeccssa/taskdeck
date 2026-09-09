//! MCP schema and audit trail tests.

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

use super::sessions_metrics::metrics_test_state;
#[test]

pub(super) fn mcp_exposes_one_control_tool() {
    let state = DaemonState::new();

    let tool = mcp_tool_definition(&state);

    assert_eq!(tool["name"], "taskdeck_control");

    assert_eq!(tool["inputSchema"]["required"][0], "action");

    assert!(tool["inputSchema"]["properties"].get("node").is_none());
}

#[test]

pub(super) fn leader_mcp_schema_exposes_cluster_targeting() {
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

#[tokio::test]

pub(super) async fn worker_mcp_audit_records_self_target() {
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

                        "id": 1,

                        "method": "tools/call",

                        "params": {"name": "taskdeck_control", "arguments": {"action": "sessions"}}

                    })
                    .to_string(),
                ))
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

pub(super) async fn leader_mcp_audit_preserves_self_and_worker_targets() {
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

pub(super) async fn task_metrics_route_clamps_valid_windows_and_envelopes_invalid_queries() {
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

pub(super) fn audit_test_state() -> DaemonState {
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

pub(super) fn test_audit_record(
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

pub(super) async fn list_audit_response(
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

pub(super) async fn audit_route_filters_searches_paginates_and_returns_redacted_details() {
    let (_, page) = list_audit_response("/api/audit", audit_test_state()).await;

    let page = page.unwrap();

    assert_eq!(page.page, 1);

    assert_eq!(page.page_size, 20);

    assert_eq!(page.total, 3);

    assert_eq!(page.items[0].audit_id, "audit-scheduler");

    assert_eq!(page.items[1].audit_id, "audit-web");

    assert_eq!(page.items[2].audit_id, "audit-cli");

    let (_, search_page) = list_audit_response("/api/audit?q=STRASSE", audit_test_state()).await;

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

pub(super) async fn mcp_missing_action_is_recorded_in_unified_audit() {
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
