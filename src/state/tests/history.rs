//! history domain state tests.

use std::path::PathBuf;

use super::super::*;
use crate::protocol::*;

#[test]
fn task_run_generation_reuse_after_restart_is_not_deduplicated() {
    let store = StateStore::open_in_memory().unwrap();
    let node_id = store.node_settings().unwrap().node_id;
    let snapshot = crate::protocol::TaskSnapshot {
        label: "cleanup".into(),
        status: TaskStatus::Running,
        pid: Some(9),
        command: "echo done".into(),
        cwd: PathBuf::from("/tmp"),
        auto_start: false,
        last_exit: None,
        exit_code: None,
        logs: vec![],
        run_generation: 1,
        started_at_ms: 100,
        schedule: Some("* * * * *".into()),
        service: Default::default(),
    };
    store
        .record_task_run_start(&node_id, &snapshot, "cron", "demo")
        .unwrap();

    // Runtime generations restart at one when the daemon is restarted. The
    // persisted start timestamp is the stable identity for the new attempt.
    let restarted_snapshot = crate::protocol::TaskSnapshot {
        started_at_ms: 200,
        ..snapshot
    };
    store
        .record_task_run_start(&node_id, &restarted_snapshot, "cron", "demo")
        .unwrap();

    let runs = store
        .list_task_runs(&TaskRunFilter {
            session: Some("demo".into()),
            task: Some("cleanup".into()),
            status: None,
            trigger: None,
            page: 1,
            page_size: 20,
        })
        .unwrap();
    assert_eq!(runs.total, 2);
    assert_eq!(runs.items[0].started_at_ms, 200);
    assert_eq!(runs.items[1].started_at_ms, 100);
}

#[test]
fn failed_task_run_attempt_is_persisted_as_terminal_history() {
    let store = StateStore::open_in_memory().unwrap();
    let node_id = store.node_settings().unwrap().node_id;
    let snapshot = crate::protocol::TaskSnapshot {
        label: "cleanup".into(),
        status: TaskStatus::Idle,
        pid: None,
        command: "missing-command".into(),
        cwd: PathBuf::from("/tmp"),
        auto_start: false,
        last_exit: None,
        exit_code: None,
        logs: vec![],
        run_generation: 1,
        started_at_ms: 0,
        schedule: Some("* * * * *".into()),
        service: Default::default(),
    };
    let record = store
        .record_task_run_failure(
            &node_id,
            &snapshot,
            "cron",
            "demo",
            "failed to spawn scheduled task",
        )
        .unwrap()
        .unwrap();

    assert_eq!(record.status, "failed");
    assert!(record.finished_at_ms.is_some());
    assert_eq!(record.error_message.as_deref(), Some("failed to spawn scheduled task"));
    let runs = store
        .list_task_runs(&TaskRunFilter {
            session: Some("demo".into()),
            task: Some("cleanup".into()),
            status: Some("failed".into()),
            trigger: Some("cron".into()),
            page: 1,
            page_size: 20,
        })
        .unwrap();
    assert_eq!(runs.total, 1);
    assert!(runs.items[0].duration_ms.is_some());
}

#[test]
fn skipped_task_run_attempt_is_persisted_as_terminal_history() {
    let store = StateStore::open_in_memory().unwrap();
    let node_id = store.node_settings().unwrap().node_id;
    let snapshot = crate::protocol::TaskSnapshot {
        label: "cleanup".into(),
        status: TaskStatus::Running,
        pid: Some(9),
        command: "echo done".into(),
        cwd: PathBuf::from("/tmp"),
        auto_start: false,
        last_exit: None,
        exit_code: None,
        logs: vec![],
        run_generation: 2,
        started_at_ms: 100,
        schedule: Some("* * * * *".into()),
        service: Default::default(),
    };
    let record = store
        .record_task_run_skipped(
            &node_id,
            &snapshot,
            "cron",
            "demo",
            "task was already running",
        )
        .unwrap()
        .unwrap();

    assert_eq!(record.status, "skipped");
    assert_eq!(record.finished_at_ms, Some(record.started_at_ms));
    assert_eq!(record.duration_ms, Some(0));
    assert_eq!(record.error_message.as_deref(), Some("task was already running"));
}

#[test]
fn unfinished_task_runs_can_be_closed_after_restart_or_shutdown() {
    let store = StateStore::open_in_memory().unwrap();
    let node_id = store.node_settings().unwrap().node_id;
    let snapshot = crate::protocol::TaskSnapshot {
        label: "cleanup".into(),
        status: TaskStatus::Running,
        pid: Some(9),
        command: "echo done".into(),
        cwd: PathBuf::from("/tmp"),
        auto_start: false,
        last_exit: None,
        exit_code: None,
        logs: vec![],
        run_generation: 3,
        started_at_ms: 100,
        schedule: None,
        service: Default::default(),
    };
    store
        .record_task_run_start(&node_id, &snapshot, "cron", "demo")
        .unwrap();
    assert_eq!(
        store
            .finish_running_task_runs(&node_id, "failed", "daemon restarted")
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .finish_running_task_runs(&node_id, "stopped", "daemon stopped")
            .unwrap(),
        0
    );

    let runs = store
        .list_task_runs(&TaskRunFilter {
            session: Some("demo".into()),
            task: Some("cleanup".into()),
            status: Some("failed".into()),
            trigger: Some("cron".into()),
            page: 1,
            page_size: 20,
        })
        .unwrap();
    assert_eq!(runs.total, 1);
    assert!(runs.items[0].finished_at_ms.is_some());
    assert_eq!(runs.items[0].error_message.as_deref(), Some("daemon restarted"));
}

#[test]
fn task_runs_and_mcp_calls_survive_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let node_id = StateStore::open(dir.path())
        .unwrap()
        .node_settings()
        .unwrap()
        .node_id;
    let snapshot = crate::protocol::TaskSnapshot {
        label: "cleanup".into(),
        status: TaskStatus::Exited,
        pid: Some(9),
        command: "echo done".into(),
        cwd: PathBuf::from("/tmp"),
        auto_start: false,
        last_exit: Some("exit status: 0".into()),
        exit_code: Some(0),
        logs: vec![],
        run_generation: 2,
        started_at_ms: 42,
        schedule: Some("* * * * *".into()),
        service: Default::default(),
    };
    {
        let store = StateStore::open(dir.path()).unwrap();
        store
            .record_task_run_start(&node_id, &snapshot, "cron", "demo")
            .unwrap();
        assert!(
            store
                .finish_task_run(&node_id, "demo", "cleanup", 2, "exited", Some(0), None)
                .unwrap()
        );
        store.record_mcp_call(McpCallRecord{id:0,tool:"taskdeck_control".into(),operation:Some("start".into()),started_at_ms:99,duration_ms:5,success:true,target_node:Some("self".into()),request:serde_json::json!({"params":{"arguments":{"session":"demo","needle":"unique-request"}}}),response:serde_json::json!({"ok":true})}).unwrap();
        store
            .record_event(
                "scheduler",
                "scheduler started",
                serde_json::json!({"caught_up":false}),
            )
            .unwrap();
    }
    let store = StateStore::open(dir.path()).unwrap();
    let runs = store
        .list_task_runs(&TaskRunFilter {
            session: Some("demo".into()),
            task: None,
            status: None,
            trigger: None,
            page: 1,
            page_size: 20,
        })
        .unwrap();
    assert_eq!(runs.items.len(), 1);
    assert_eq!(runs.items[0].status, "exited");
    assert!(runs.items[0].duration_ms.is_some());
    let calls = store
        .list_mcp_calls(Some("unique-request"), None, None, None, None, 1, 20)
        .unwrap();
    assert_eq!(calls.items.len(), 1);
    assert_eq!(calls.total, 1);
    assert_eq!(
        store
            .mcp_call_detail(calls.items[0].id)
            .unwrap()
            .unwrap()
            .tool,
        "taskdeck_control"
    );
    assert_eq!(
        store
            .list_events(&EventFilter {
                category: None,
                page: 1,
                page_size: 20
            })
            .unwrap()
            .items
            .len(),
        1
    );
}
