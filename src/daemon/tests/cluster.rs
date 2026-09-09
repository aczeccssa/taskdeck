//! Remote worker audit tests.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use super::super::audit::*;
use super::super::client::*;
use super::super::dispatch::*;
use super::super::gates::*;
use super::super::handle::*;
use super::super::inventory::*;
use super::super::metrics::*;
use super::super::notifications::*;
use super::super::process_tree::*;
use super::super::sampler::*;
use super::super::scaling::*;
use super::super::scheduler::*;
use super::super::state::*;
use super::super::util::*;
use super::super::*;
use super::helpers::*;
use crate::config;
use crate::protocol::*;
use crate::runtime::{SessionRuntime, Sessions};
use crate::state::{NodeRole, NodeSettings, StateStore};

#[tokio::test]

async fn dispatch_node_compatibility_wrapper_still_audits_failures() {
    let state = DaemonState::new();

    let response = state
        .dispatch_node(
            "remote-on-worker",
            crate::cluster::RemoteRequest::ListSessions,
        )
        .await;

    assert!(!response.ok);

    let page = state
        .store
        .list_audit(&crate::protocol::AuditFilter {
            q: Some("worker nodes can only control".to_string()),

            source: Some("internal".to_string()),

            status: Some("error".to_string()),

            node: None,

            session: None,

            task: None,

            operation: Some("list_sessions".to_string()),

            page: 1,

            page_size: 20,
        })
        .unwrap();

    assert_eq!(page.total, 1);

    let detail = state
        .store
        .audit_detail(&page.items[0].audit_id)
        .unwrap()
        .unwrap();

    assert_eq!(detail.details["node"], "remote-on-worker");
}

#[tokio::test]

async fn leader_remote_worker_failures_are_audited_with_requested_executor() {
    let state = DaemonState::new();

    let settings = state
        .store
        .configure(crate::state::NodeSettingsUpdate {
            role: Some(crate::state::NodeRole::Leader),

            ..crate::state::NodeSettingsUpdate::default()
        })
        .unwrap();

    *state.settings.lock().expect("node settings lock") = settings.clone();

    let response = state
        .dispatch_node_with_audit(
            "missing-worker",
            crate::cluster::RemoteRequest::ListSessions,
            AuditContext::new(AuditSource::Web, AuditTransport::Http),
        )
        .await;

    assert!(!response.ok);

    let page = state
        .store
        .list_audit(&crate::protocol::AuditFilter {
            q: Some("worker 'missing-worker' not found".to_string()),

            source: Some("web".to_string()),

            status: Some("error".to_string()),

            node: Some("missing-worker".to_string()),

            session: None,

            task: None,

            operation: Some("list_sessions".to_string()),

            page: 1,

            page_size: 20,
        })
        .unwrap();

    assert_eq!(page.total, 1);

    let detail = state
        .store
        .audit_detail(&page.items[0].audit_id)
        .unwrap()
        .unwrap();

    assert_eq!(
        detail.origin_node_id.as_deref(),
        Some(settings.node_id.as_str())
    );

    assert_eq!(detail.executor_node_id.as_deref(), Some("missing-worker"));

    assert_eq!(detail.source, AuditSource::Web);
}
