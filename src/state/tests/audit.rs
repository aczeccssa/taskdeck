//! audit domain state tests.


use super::super::*;
use crate::protocol::*;


    fn sample_audit(audit_id: &str, replicated: bool) -> AuditRecord {
        AuditRecord {
            audit_id: audit_id.to_string(),
            correlation_id: format!("corr-{audit_id}"),
            timestamp_ms: 100,
            duration_ms: 5,
            source: AuditSource::Cli,
            transport: AuditTransport::Ipc,
            origin_node_id: Some("worker-1".into()),
            executor_node_id: Some("worker-1".into()),
            request_kind: "action".into(),
            operation: "start".into(),
            session: Some("demo".into()),
            task: Some("api".into()),
            status: AuditStatus::Success,
            success: true,
            error: None,
            request: serde_json::json!({"type":"action","token":"secret"}),
            response: serde_json::json!({"ok":true}),
            details: serde_json::json!({}),
            replicated_at_ms: replicated.then_some(200),
        }
    }

    #[test]
    fn audit_records_are_idempotent_and_survive_reopening() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = StateStore::open(dir.path()).unwrap();
            store.record_audit(sample_audit("audit-1", false)).unwrap();
            store.record_audit(sample_audit("audit-1", false)).unwrap();
            let page = store
                .list_audit(&AuditFilter {
                    q: Some("action".into()),
                    source: None,
                    status: None,
                    node: Some("worker-1".into()),
                    session: Some("demo".into()),
                    task: None,
                    operation: Some("start".into()),
                    page: 1,
                    page_size: 20,
                })
                .unwrap();
            assert_eq!(page.total, 1);
            let detail = store.audit_detail("audit-1").unwrap().unwrap();
            assert_eq!(detail.request["token"], "[REDACTED]");
            assert!(detail.replicated_at_ms.is_none());
        }
        let reopened = StateStore::open(dir.path()).unwrap();
        assert_eq!(
            reopened.audit_detail("audit-1").unwrap().unwrap().operation,
            "start"
        );
    }

    #[test]
    fn audit_retention_keeps_unreplicated_records() {
        let store = StateStore::open_in_memory().unwrap();
        for index in 0..(AUDIT_RETENTION_LIMIT + 5) {
            let mut record = sample_audit(&format!("kept-{index}"), true);
            record.timestamp_ms = index as u64;
            store.record_audit(record).unwrap();
        }
        let mut unreplicated = sample_audit("pending", false);
        unreplicated.timestamp_ms = 0;
        store.record_audit(unreplicated).unwrap();
        let unreplicated = store.unreplicated_audit_records(20).unwrap();
        assert_eq!(unreplicated.len(), 1);
        assert_eq!(unreplicated[0].audit_id, "pending");
        let page = store
            .list_audit(&AuditFilter {
                q: None,
                source: None,
                status: None,
                node: None,
                session: None,
                task: None,
                operation: None,
                page: 1,
                page_size: 100,
            })
            .unwrap();
        assert_eq!(page.total, AUDIT_RETENTION_LIMIT + 1);
        store
            .mark_audit_replicated(&["pending".to_string()], 9_999)
            .unwrap();
        let page = store
            .list_audit(&AuditFilter {
                q: None,
                source: None,
                status: None,
                node: None,
                session: None,
                task: None,
                operation: None,
                page: 1,
                page_size: 100,
            })
            .unwrap();
        assert_eq!(page.total, AUDIT_RETENTION_LIMIT);
        assert!(store.audit_detail("pending").unwrap().is_none());
    }
