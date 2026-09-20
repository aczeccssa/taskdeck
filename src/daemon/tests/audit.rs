//! Audit dispatch tests.

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

pub(super) fn call_record() -> McpCallRecord {
    McpCallRecord {
        id: 0,

        tool: "taskdeck_control".to_string(),

        operation: Some("sessions".to_string()),

        started_at_ms: 1,

        duration_ms: 2,

        success: true,

        target_node: None,

        request: json!({"method": "tools/call"}),

        response: json!({"result": {"isError": false}}),
    }
}

#[test]

pub(super) fn dispatch_with_audit_records_success_and_error_sources() {
    let state = DaemonState::new();

    let cli_context =
        AuditContext::new(AuditSource::Cli, AuditTransport::Ipc).with_origin_node("cli-origin");

    let response = dispatch_with_audit(&state, Request::Ping, Some(cli_context));

    assert!(response.ok);

    let page = state
        .store
        .list_audit(&crate::protocol::AuditFilter {
            q: None,

            source: Some("cli".to_string()),

            status: Some("success".to_string()),

            node: Some("cli-origin".to_string()),

            session: None,

            task: None,

            operation: Some("ping".to_string()),

            page: 1,

            page_size: 20,
        })
        .unwrap();

    assert_eq!(page.total, 1);

    let success = state
        .store
        .audit_detail(&page.items[0].audit_id)
        .unwrap()
        .unwrap();

    assert_eq!(success.source, AuditSource::Cli);

    assert_eq!(success.transport, AuditTransport::Ipc);

    assert_eq!(success.origin_node_id.as_deref(), Some("cli-origin"));

    assert!(success.executor_node_id.is_some());

    let response = dispatch_with_audit(
        &state,
        Request::Snapshot {
            session: "missing".to_string(),

            tail: None,
        },
        Some(AuditContext::new(AuditSource::Tui, AuditTransport::Ipc)),
    );

    assert!(!response.ok);

    let errors = state
        .store
        .list_audit(&crate::protocol::AuditFilter {
            q: Some("session 'missing'".to_string()),

            source: Some("tui".to_string()),

            status: Some("error".to_string()),

            node: None,

            session: Some("missing".to_string()),

            task: None,

            operation: Some("snapshot".to_string()),

            page: 1,

            page_size: 20,
        })
        .unwrap();

    assert_eq!(errors.total, 1);

    assert!(
        errors.items[0]
            .error
            .as_deref()
            .unwrap()
            .contains("missing")
    );
}

#[test]
pub(super) fn successful_web_polling_requests_are_not_audited() {
    let state = DaemonState::new();
    let context = AuditContext::new(AuditSource::Web, AuditTransport::Http);
    let response = Response::empty("poll");

    for request in [
        Request::Snapshot {
            session: "dashboard".to_string(),
            tail: Some(50),
        },
        Request::TaskLogs {
            session: "dashboard".to_string(),
            task: "api".to_string(),
            after: None,
            limit: 50,
        },
        Request::TaskMetrics {
            session: "dashboard".to_string(),
            task: "api".to_string(),
            window_seconds: 600,
        },
    ] {
        record_request_audit(
            &state,
            &request,
            context.clone(),
            None,
            1,
            1,
            AuditStatus::Success,
            None,
            &response,
            json!({}),
        );
    }

    let filter = crate::protocol::AuditFilter {
        q: None,
        source: None,
        status: None,
        node: None,
        session: None,
        task: None,
        operation: None,
        page: 1,
        page_size: 20,
    };
    assert_eq!(state.store.list_audit(&filter).unwrap().total, 0);

    record_request_audit(
        &state,
        &Request::Snapshot {
            session: "dashboard".to_string(),
            tail: Some(50),
        },
        context,
        None,
        1,
        1,
        AuditStatus::Error,
        None,
        &Response::error("poll failed"),
        json!({}),
    );
    assert_eq!(state.store.list_audit(&filter).unwrap().total, 1);
}
