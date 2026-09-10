//! notification domain state tests.

use super::super::*;
use crate::protocol::*;

#[test]
fn notifications_rules_and_records_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::open(dir.path()).unwrap();
    let rule = store
        .create_notification_rule(NotificationRuleInput {
            name: " failures ".to_string(),
            event_types: vec!["task_failed".to_string()],
            scope_session: Some("api".to_string()),
            scope_task: None,
            webhook_url: Some("https://example.com/hook".to_string()),
            enabled: true,
        })
        .unwrap();
    assert_eq!(rule.name, "failures");

    assert!(
        store
            .create_notification_rule(NotificationRuleInput {
                name: "bad".to_string(),
                event_types: vec!["explosion".to_string()],
                scope_session: None,
                scope_task: None,
                webhook_url: None,
                enabled: true,
            })
            .is_err()
    );
    assert!(
        store
            .create_notification_rule(NotificationRuleInput {
                name: "bad".to_string(),
                event_types: vec![],
                scope_session: None,
                scope_task: None,
                webhook_url: None,
                enabled: true,
            })
            .is_err()
    );
    assert!(
        store
            .create_notification_rule(NotificationRuleInput {
                name: "bad".to_string(),
                event_types: vec!["task_failed".to_string()],
                scope_session: None,
                scope_task: None,
                webhook_url: Some("ftp://example.com".to_string()),
                enabled: true,
            })
            .is_err()
    );

    let first = store
        .insert_notification(
            "node-1",
            Some(&rule.id),
            Some(&rule.name),
            "task_failed",
            "critical",
            Some("api"),
            Some("build"),
            "task failed",
            "build exited with code 1",
            &serde_json::json!({"exit_code": 1}),
        )
        .unwrap();
    store
        .insert_notification(
            "node-1",
            None,
            None,
            "scale_out",
            "info",
            None,
            None,
            "scaled out",
            "replica started",
            &serde_json::json!({}),
        )
        .unwrap();
    assert_eq!(store.unread_notification_count().unwrap(), 2);
    assert_eq!(store.mark_notifications_read(Some(first.id)).unwrap(), 1);
    assert_eq!(store.unread_notification_count().unwrap(), 1);
    assert_eq!(store.mark_notifications_read(None).unwrap(), 1);
    assert_eq!(store.unread_notification_count().unwrap(), 0);
    assert_eq!(store.notifications(10).unwrap().len(), 2);

    for _index in 0..(NOTIFICATION_RETENTION_LIMIT + 5) {
        store
            .insert_notification(
                "node-1",
                None,
                None,
                "task_started",
                "info",
                Some("api"),
                Some("dev"),
                "started",
                "dev",
                &serde_json::json!({}),
            )
            .unwrap();
    }
    let (total, _) = store
        .connection
        .lock()
        .expect("state store lock")
        .query_row("SELECT COUNT(*) FROM notifications", [], |row| {
            row.get::<_, i64>(0).map(|count| (count, ()))
        })
        .unwrap();
    assert_eq!(total as usize, NOTIFICATION_RETENTION_LIMIT);

    let updated = store
        .update_notification_rule(
            &rule.id,
            NotificationRuleInput {
                name: "failures".to_string(),
                event_types: vec!["task_failed".to_string(), "task_stopped".to_string()],
                scope_session: None,
                scope_task: None,
                webhook_url: None,
                enabled: false,
            },
        )
        .unwrap();
    assert!(!updated.enabled);
    assert_eq!(updated.event_types.len(), 2);
    assert!(store.delete_notification_rule(&rule.id).unwrap());
}
