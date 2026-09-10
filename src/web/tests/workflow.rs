//! Workflow group/run endpoint tests.

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

pub(super) async fn workflow_groups_api_crud_resolves_targets_and_ungrouped() {
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

pub(super) async fn workflow_groups_api_enforces_leader_scope_and_pure_master_self_rule() {
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

pub(super) async fn workflow_group_action_is_best_effort_and_audited() {
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

pub(super) async fn workflow_revisions_record_history_and_restore() {
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

pub(super) async fn workflow_run_follows_graph_order_and_stop_on_failure() {
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
