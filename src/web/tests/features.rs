//! Quota/notification/token/template/dependency/scaling tests.

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

pub(super) async fn quotas_api_crud_and_validation() {
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

pub(super) async fn notification_rules_and_notifications_round_trip() {
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

pub(super) async fn api_tokens_authenticate_external_clients() {
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

pub(super) async fn board_templates_create_apply_export_import() {
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

pub(super) async fn dependencies_api_validates_scope_and_cycles() {
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

pub(super) async fn node_metrics_reports_nodes_and_status_counts() {
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

pub(super) async fn node_metrics_includes_self_on_pure_master() {
    let state = DaemonState::new();

    let settings = state
        .store
        .configure(crate::state::NodeSettingsUpdate {
            role: Some(crate::state::NodeRole::Leader),

            leader_mode: Some(crate::state::LeaderMode::PureMaster),

            ..Default::default()
        })
        .unwrap();

    *state.settings.lock().expect("node settings lock") = settings;

    // Seed one self sample the way the daemon sampler would.

    state.node_metrics.push(
        &state.public_settings().node_id,
        crate::protocol::NodeMetricsSample {
            timestamp_ms: current_millis(),

            cpu_percent: 12.5,

            memory_bytes: 1_000,

            memory_total_bytes: 4_000,

            running_tasks: 0,
        },
    );

    let listed = get_json(state, "/api/node-metrics").await;

    assert!(listed.ok, "{}", listed.message);

    let nodes = listed.data.unwrap()["nodes"].as_array().unwrap().clone();

    assert_eq!(nodes.len(), 1);

    assert!(nodes[0]["is_self"].as_bool().unwrap());

    assert_eq!(nodes[0]["current"]["cpu_percent"], 12.5);

    assert_eq!(nodes[0]["session_count"], 0);
}

#[tokio::test]

pub(super) async fn scaling_policies_api_crud_and_validation() {
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
