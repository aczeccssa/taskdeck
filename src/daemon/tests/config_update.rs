//! Session config update tests.

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

#[test]

pub(super) fn update_reloads_a_registered_project_configuration() {
    let dir = tempfile::tempdir().unwrap();

    let config_path = dir.path().join(config::PROJECT_CONFIG);

    fs::write(
        &config_path,
        "version: 1\nsession: demo\ntasks:\n  api:\n    command: echo old\n",
    )
    .unwrap();

    let state = DaemonState::new();

    let registered = handle(
        &state,
        Request::Register {
            project: dir.path().to_path_buf(),

            session: Some("custom".to_string()),
        },
    )
    .unwrap();

    assert!(registered.ok);

    fs::write(

            &config_path,

            "version: 1\nsession: demo\ntasks:\n  api:\n    command: echo new\n  worker:\n    command: echo worker\n",

        )

        .unwrap();

    let updated = handle(
        &state,
        Request::Update {
            project: dir.path().to_path_buf(),

            session: None,
        },
    )
    .unwrap();

    let snapshot: crate::protocol::SessionSnapshot =
        serde_json::from_value(updated.data.unwrap()).unwrap();

    assert_eq!(updated.message, "updated session 'custom'");

    assert_eq!(snapshot.name, "custom");

    assert_eq!(snapshot.tasks["api"].command, "echo new");

    assert!(snapshot.tasks.contains_key("worker"));
}

#[test]

pub(super) fn put_session_config_updates_all_registered_sessions_for_the_project() {
    let dir = tempfile::tempdir().unwrap();

    let config_path = dir.path().join(config::PROJECT_CONFIG);

    fs::write(

            &config_path,

            "version: 1\ntasks:\n  api:\n    command: echo old\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",

        )

        .unwrap();

    let state = DaemonState::new();

    assert!(
        handle(
            &state,
            Request::Register {
                project: dir.path().to_path_buf(),

                session: Some("one".to_string()),
            },
        )
        .unwrap()
        .ok
    );

    assert!(
        handle(
            &state,
            Request::Register {
                project: dir.path().to_path_buf(),

                session: Some("two".to_string()),
            },
        )
        .unwrap()
        .ok
    );

    let revision = serde_json::from_value::<SessionConfigSnapshot>(
        handle(
            &state,
            Request::GetSessionConfig {
                session: "one".to_string(),
            },
        )
        .unwrap()
        .data
        .unwrap(),
    )
    .unwrap()
    .revision;

    let updated = handle(
        &state,
        Request::PutSessionConfig {
            session: "one".to_string(),

            revision,

            workspace_env: None,

            tasks: vec![
                task_input("api", "echo new"),
                task_input("worker", "echo worker"),
            ],
        },
    )
    .unwrap();

    let config: SessionConfigSnapshot = serde_json::from_value(updated.data.unwrap()).unwrap();

    assert!(updated.ok);

    assert_eq!(config.tasks.len(), 2);

    let one: crate::protocol::SessionSnapshot = serde_json::from_value(
        handle(
            &state,
            Request::Snapshot {
                session: "one".to_string(),

                tail: Some(20),
            },
        )
        .unwrap()
        .data
        .unwrap(),
    )
    .unwrap();

    let two: crate::protocol::SessionSnapshot = serde_json::from_value(
        handle(
            &state,
            Request::Snapshot {
                session: "two".to_string(),

                tail: Some(20),
            },
        )
        .unwrap()
        .data
        .unwrap(),
    )
    .unwrap();

    assert_eq!(one.tasks["api"].command, "echo new");

    assert_eq!(two.tasks["api"].command, "echo new");

    assert!(one.tasks.contains_key("worker"));

    assert!(two.tasks.contains_key("worker"));
}

#[test]

pub(super) fn put_session_config_reports_new_auto_start_failures() {
    let dir = tempfile::tempdir().unwrap();

    let config_path = dir.path().join(config::PROJECT_CONFIG);

    fs::write(

            &config_path,

            "version: 1\ntasks:\n  api:\n    command: echo ready\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",

        )

        .unwrap();

    let state = DaemonState::new();

    assert!(
        handle(
            &state,
            Request::Register {
                project: dir.path().to_path_buf(),

                session: Some("demo".to_string()),
            },
        )
        .unwrap()
        .ok
    );

    let config: SessionConfigSnapshot = serde_json::from_value(
        handle(
            &state,
            Request::GetSessionConfig {
                session: "demo".to_string(),
            },
        )
        .unwrap()
        .data
        .unwrap(),
    )
    .unwrap();

    let mut broken = task_input("broken", "/taskdeck/no-such-executable");

    broken.shell = false;

    broken.auto_start = true;

    let response = handle(
        &state,
        Request::PutSessionConfig {
            session: "demo".to_string(),

            revision: config.revision,

            workspace_env: None,

            tasks: vec![task_input("api", "echo ready"), broken],
        },
    )
    .unwrap();

    assert!(!response.ok);

    let data = response.data.unwrap();

    assert_eq!(data["kind"], "reconciliation_error");

    assert_eq!(data["saved"], true);

    assert_eq!(data["errors"][0]["session"], "demo");

    assert!(
        data["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("failed to auto-start new task")
    );

    let runtime = state.sessions.lock().expect("sessions lock");

    assert!(!runtime["demo"].has_task("broken"));

    assert!(fs::read_to_string(config_path).unwrap().contains("broken:"));
}

#[test]

pub(super) fn put_session_config_reports_stale_revision_conflicts_in_the_response_envelope() {
    let dir = tempfile::tempdir().unwrap();

    let config_path = dir.path().join(config::PROJECT_CONFIG);

    fs::write(

            &config_path,

            "version: 1\ntasks:\n  api:\n    command: echo old\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",

        )

        .unwrap();

    let state = DaemonState::new();

    assert!(
        handle(
            &state,
            Request::Register {
                project: dir.path().to_path_buf(),

                session: Some("one".to_string()),
            },
        )
        .unwrap()
        .ok
    );

    let snapshot: SessionConfigSnapshot = serde_json::from_value(
        handle(
            &state,
            Request::GetSessionConfig {
                session: "one".to_string(),
            },
        )
        .unwrap()
        .data
        .unwrap(),
    )
    .unwrap();

    fs::write(

            &config_path,

            "version: 1\ntasks:\n  api:\n    command: echo newer\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",

        )

        .unwrap();

    let response = handle(
        &state,
        Request::PutSessionConfig {
            session: "one".to_string(),

            revision: snapshot.revision,

            workspace_env: None,

            tasks: vec![task_input("api", "echo changed")],
        },
    )
    .unwrap();

    assert!(!response.ok);

    assert_eq!(response.data.as_ref().unwrap()["kind"], "stale_revision");

    assert_eq!(response.data.as_ref().unwrap()["status"], 409);
}

#[test]

pub(super) fn concurrent_same_revision_puts_allow_exactly_one_winner() {
    let dir = tempfile::tempdir().unwrap();

    let config_path = dir.path().join(config::PROJECT_CONFIG);

    fs::write(

            &config_path,

            "version: 1\ntasks:\n  api:\n    command: echo old\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",

        )

        .unwrap();

    let state = DaemonState::new();

    state.set_put_config_post_check_delay(Duration::from_millis(100));

    assert!(
        handle(
            &state,
            Request::Register {
                project: dir.path().to_path_buf(),

                session: Some("one".to_string()),
            },
        )
        .unwrap()
        .ok
    );

    let snapshot: SessionConfigSnapshot = serde_json::from_value(
        handle(
            &state,
            Request::GetSessionConfig {
                session: "one".to_string(),
            },
        )
        .unwrap()
        .data
        .unwrap(),
    )
    .unwrap();

    let barrier = Arc::new(Barrier::new(3));

    let first_state = state.clone();

    let first_barrier = barrier.clone();

    let first_revision = snapshot.revision.clone();

    let first = thread::spawn(move || {
        first_barrier.wait();

        handle(
            &first_state,
            Request::PutSessionConfig {
                session: "one".to_string(),

                revision: first_revision,

                workspace_env: None,

                tasks: vec![task_input("api", "echo one")],
            },
        )
        .unwrap()
    });

    let second_state = state.clone();

    let second_barrier = barrier.clone();

    let second_revision = snapshot.revision;

    let second = thread::spawn(move || {
        second_barrier.wait();

        handle(
            &second_state,
            Request::PutSessionConfig {
                session: "one".to_string(),

                revision: second_revision,

                workspace_env: None,

                tasks: vec![task_input("api", "echo two")],
            },
        )
        .unwrap()
    });

    barrier.wait();

    let responses = [first.join().unwrap(), second.join().unwrap()];

    let success_count = responses.iter().filter(|response| response.ok).count();

    let conflict_count = responses
        .iter()
        .filter(|response| {
            response
                .data
                .as_ref()
                .is_some_and(|data| data["kind"] == "stale_revision")
        })
        .count();

    assert_eq!(success_count, 1);

    assert_eq!(conflict_count, 1);
}

#[test]

pub(super) fn put_session_config_detects_external_edit_before_finalize_without_updating_runtime() {
    let dir = tempfile::tempdir().unwrap();

    let config_path = dir.path().join(config::PROJECT_CONFIG);

    fs::write(

            &config_path,

            "version: 1\ntasks:\n  api:\n    command: echo old\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",

        )

        .unwrap();

    let state = DaemonState::new();

    state.set_put_config_before_finalize_content(

            "version: 1\ntasks:\n  api:\n    command: echo external\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",

        );

    assert!(
        handle(
            &state,
            Request::Register {
                project: dir.path().to_path_buf(),

                session: Some("one".to_string()),
            },
        )
        .unwrap()
        .ok
    );

    let snapshot: SessionConfigSnapshot = serde_json::from_value(
        handle(
            &state,
            Request::GetSessionConfig {
                session: "one".to_string(),
            },
        )
        .unwrap()
        .data
        .unwrap(),
    )
    .unwrap();

    let response = handle(
        &state,
        Request::PutSessionConfig {
            session: "one".to_string(),

            revision: snapshot.revision,

            workspace_env: None,

            tasks: vec![task_input("api", "echo submitted")],
        },
    )
    .unwrap();

    assert!(!response.ok);

    assert_eq!(response.data.as_ref().unwrap()["kind"], "stale_revision");

    assert!(
        fs::read_to_string(&config_path)
            .unwrap()
            .contains("echo external")
    );

    let runtime: crate::protocol::SessionSnapshot = serde_json::from_value(
        handle(
            &state,
            Request::Snapshot {
                session: "one".to_string(),

                tail: Some(20),
            },
        )
        .unwrap()
        .data
        .unwrap(),
    )
    .unwrap();

    assert_eq!(runtime.tasks["api"].command, "echo old");
}

#[test]

pub(super) fn register_and_update_wait_for_the_config_mutation_lock() {
    let dir = tempfile::tempdir().unwrap();

    let config_path = dir.path().join(config::PROJECT_CONFIG);

    fs::write(

            &config_path,

            "version: 1\ntasks:\n  api:\n    command: echo old\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",

        )

        .unwrap();

    let state = DaemonState::new();

    let guard = state
        .config_mutations
        .lock()
        .expect("config mutations lock");

    let (register_tx, register_rx) = mpsc::channel();

    let register_state = state.clone();

    let register_project = dir.path().to_path_buf();

    let register = thread::spawn(move || {
        let response = handle(
            &register_state,
            Request::Register {
                project: register_project,

                session: Some("one".to_string()),
            },
        )
        .unwrap();

        register_tx.send(response.ok).unwrap();
    });

    assert!(register_rx.recv_timeout(Duration::from_millis(50)).is_err());

    drop(guard);

    assert!(register_rx.recv_timeout(Duration::from_secs(1)).unwrap());

    register.join().unwrap();

    let guard = state
        .config_mutations
        .lock()
        .expect("config mutations lock");

    let (update_tx, update_rx) = mpsc::channel();

    let update_state = state.clone();

    let update_project = dir.path().to_path_buf();

    let update = thread::spawn(move || {
        let response = handle(
            &update_state,
            Request::Update {
                project: update_project,

                session: Some("one".to_string()),
            },
        )
        .unwrap();

        update_tx.send(response.ok).unwrap();
    });

    assert!(update_rx.recv_timeout(Duration::from_millis(50)).is_err());

    drop(guard);

    assert!(update_rx.recv_timeout(Duration::from_secs(1)).unwrap());

    update.join().unwrap();
}

#[test]

pub(super) fn put_session_config_commits_then_reconciles_remaining_sessions_and_retry_converges() {
    let dir = tempfile::tempdir().unwrap();

    let config_path = dir.path().join(config::PROJECT_CONFIG);

    fs::write(

            &config_path,

            "version: 1\ntasks:\n  api:\n    command: echo old\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",

        )

        .unwrap();

    let state = DaemonState::new();

    for session in ["a", "b", "c"] {
        assert!(
            handle(
                &state,
                Request::Register {
                    project: dir.path().to_path_buf(),

                    session: Some(session.to_string()),
                },
            )
            .unwrap()
            .ok
        );
    }

    state.set_put_config_runtime_failure_after(1);

    let snapshot: SessionConfigSnapshot = serde_json::from_value(
        handle(
            &state,
            Request::GetSessionConfig {
                session: "a".to_string(),
            },
        )
        .unwrap()
        .data
        .unwrap(),
    )
    .unwrap();

    let response = handle(
        &state,
        Request::PutSessionConfig {
            session: "a".to_string(),

            revision: snapshot.revision,

            workspace_env: None,

            tasks: vec![task_input("api", "echo new")],
        },
    )
    .unwrap();

    assert!(!response.ok);

    assert!(response.message.contains("saved to disk"));

    let data = response.data.as_ref().unwrap();

    assert_eq!(data["kind"], "reconciliation_error");

    assert_eq!(data["status"], 500);

    assert_eq!(data["saved"], true);

    assert_eq!(data["errors"][0]["session"], "b");

    let current_revision = data["current_revision"].as_str().unwrap().to_string();

    assert!(
        fs::read_to_string(&config_path)
            .unwrap()
            .contains("echo new")
    );

    let runtime_command = |session: &str| {
        let snapshot: crate::protocol::SessionSnapshot = serde_json::from_value(
            handle(
                &state,
                Request::Snapshot {
                    session: session.to_string(),

                    tail: Some(20),
                },
            )
            .unwrap()
            .data
            .unwrap(),
        )
        .unwrap();

        snapshot.tasks["api"].command.clone()
    };

    assert_eq!(runtime_command("a"), "echo new");

    assert_eq!(runtime_command("b"), "echo old");

    assert_eq!(runtime_command("c"), "echo new");

    state.clear_put_config_runtime_failure();

    let response = handle(
        &state,
        Request::PutSessionConfig {
            session: "a".to_string(),

            revision: current_revision,

            workspace_env: None,

            tasks: vec![task_input("api", "echo new")],
        },
    )
    .unwrap();

    assert!(response.ok);

    assert_eq!(runtime_command("a"), "echo new");

    assert_eq!(runtime_command("b"), "echo new");

    assert_eq!(runtime_command("c"), "echo new");
}
