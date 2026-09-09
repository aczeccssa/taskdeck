//! quota domain state tests.


use super::super::*;
use crate::protocol::*;

    #[test]
    fn quotas_are_persisted_and_validated() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::open(dir.path()).unwrap();
        let node_quota = store
            .create_quota(
                "node-1",
                WorkspaceQuotaInput {
                    session: None,
                    max_running_tasks: 4,
                },
            )
            .unwrap();
        assert_eq!(node_quota.session, None);
        let session_quota = store
            .create_quota(
                "node-1",
                WorkspaceQuotaInput {
                    session: Some(" api ".to_string()),
                    max_running_tasks: 2,
                },
            )
            .unwrap();
        assert_eq!(session_quota.session.as_deref(), Some("api"));

        assert!(
            store
                .create_quota(
                    "node-1",
                    WorkspaceQuotaInput {
                        session: Some("api".to_string()),
                        max_running_tasks: 3,
                    }
                )
                .is_err()
        );
        assert!(
            store
                .create_quota(
                    "node-1",
                    WorkspaceQuotaInput {
                        session: None,
                        max_running_tasks: 0,
                    }
                )
                .is_err()
        );

        let updated = store
            .update_quota(
                &session_quota.id,
                WorkspaceQuotaInput {
                    session: Some("web".to_string()),
                    max_running_tasks: 6,
                },
            )
            .unwrap();
        assert_eq!(updated.session.as_deref(), Some("web"));
        assert_eq!(updated.max_running_tasks, 6);
        assert!(store.delete_quota(&session_quota.id).unwrap());
        assert_eq!(store.quotas().unwrap().len(), 1);
    }

