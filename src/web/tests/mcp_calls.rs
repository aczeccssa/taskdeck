//! MCP call history endpoint tests.

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

pub(super) fn mcp_call_test_state() -> DaemonState {
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

pub(super) fn test_mcp_call(
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

pub(super) async fn list_mcp_calls_response(
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

pub(super) async fn mcp_calls_route_searches_across_targets_and_serialized_input() {
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

pub(super) async fn mcp_calls_route_uses_unicode_casefold_search() {
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

pub(super) async fn mcp_calls_route_does_not_search_response_payload_and_detail_keeps_full_record()
{
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

pub(super) async fn mcp_calls_route_applies_exact_and_combined_filters() {
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

pub(super) async fn mcp_calls_route_rejects_invalid_status_and_page_inputs() {
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

pub(super) async fn mcp_calls_route_paginates_and_snaps_page_sizes() {
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

pub(super) async fn mcp_calls_route_defaults_and_keeps_newest_first_order() {
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
