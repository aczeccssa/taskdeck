//! Cluster tests.

use super::*;
use super::leader::*;
use super::messages::*;
use super::worker::*;
use crate::protocol::*;
use crate::state::StateStore;
use crate::daemon::DaemonState;
use std::sync::Arc;

    #[test]
    fn duplicate_command_returns_cached_result() {
        let mut cache = CommandResultCache::new(2);
        cache.insert("cmd-1".to_string(), Response::empty("done"));
        assert_eq!(cache.get("cmd-1").unwrap().message, "done");
    }

    #[test]
    fn rejects_wrong_enrollment_token_and_protocol() {
        let store = Arc::new(StateStore::open_in_memory().unwrap());
        let cluster = LeaderCluster::new(store, Some("secret".to_string())).unwrap();
        let wrong_token = AgentMessage::Hello {
            protocol: AGENT_PROTOCOL_VERSION,
            node_id: "worker-1".to_string(),
            name: "worker".to_string(),
            version: "test".to_string(),
            token: Some("wrong".to_string()),
        };
        assert!(cluster.validate_hello(&wrong_token).is_err());

        let wrong_protocol = AgentMessage::Hello {
            protocol: 99,
            node_id: "worker-1".to_string(),
            name: "worker".to_string(),
            version: "test".to_string(),
            token: Some("secret".to_string()),
        };
        assert!(cluster.validate_hello(&wrong_protocol).is_err());
    }

    #[test]
    fn ingest_worker_audits_is_idempotent_and_normalizes_executor_identity() {
        let store = Arc::new(StateStore::open_in_memory().unwrap());
        let cluster = LeaderCluster::new(store.clone(), None).unwrap();
        let record = AuditRecord {
            audit_id: "audit-1".to_string(),
            correlation_id: "corr-1".to_string(),
            timestamp_ms: 42,
            duration_ms: 3,
            source: crate::protocol::AuditSource::Cli,
            transport: crate::protocol::AuditTransport::Ipc,
            origin_node_id: None,
            executor_node_id: Some("spoofed-worker".to_string()),
            request_kind: "action".to_string(),
            operation: "start".to_string(),
            session: Some("demo".to_string()),
            task: Some("api".to_string()),
            status: AuditStatus::Success,
            success: true,
            error: None,
            request: serde_json::json!({"type":"action","secret":"hide-me"}),
            response: serde_json::json!({"ok":true}),
            details: serde_json::json!({}),
            replicated_at_ms: None,
        };

        let accepted = cluster.ingest_worker_audits("worker-1", vec![record.clone(), record]);
        assert_eq!(accepted, vec!["audit-1".to_string()]);

        let page = store
            .list_audit(&crate::protocol::AuditFilter {
                q: None,
                source: Some("cli".to_string()),
                status: None,
                node: Some("worker-1".to_string()),
                session: Some("demo".to_string()),
                task: Some("api".to_string()),
                operation: Some("start".to_string()),
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(page.total, 1);
        let detail = store.audit_detail("audit-1").unwrap().unwrap();
        assert_eq!(detail.origin_node_id.as_deref(), Some("worker-1"));
        assert_eq!(detail.executor_node_id.as_deref(), Some("worker-1"));
        assert_eq!(
            detail.details["reported_executor_node_id"],
            "spoofed-worker"
        );
        assert_eq!(detail.request["secret"], "[REDACTED]");
        assert!(detail.replicated_at_ms.is_some());
    }

    #[test]
    fn builds_agent_urls_for_http_and_websocket_leaders() {
        assert_eq!(
            agent_url("http://leader:9837").unwrap(),
            "ws://leader:9837/api/agent/connect"
        );
        assert_eq!(
            agent_url("wss://leader.example/").unwrap(),
            "wss://leader.example/api/agent/connect"
        );
    }
