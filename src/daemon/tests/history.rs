//! History sampler tests.

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

use super::audit::call_record;
#[test]

pub(super) fn history_sampler_records_a_subsecond_run_and_exit_code() {
    let state = DaemonState::new();

    let project = tempfile::tempdir().unwrap();

    let mut runtime = SessionRuntime::new(crate::config::ProjectDefinition {
        session: "demo".to_string(),

        project: project.path().to_path_buf(),

        source: "taskdeck.yaml".to_string(),

        tasks: std::collections::BTreeMap::from([(
            "quick".to_string(),
            crate::config::TaskSpec {
                label: "quick".to_string(),

                program: "exit".to_string(),

                args: vec!["7".to_string()],

                cwd: project.path().to_path_buf(),

                env: Default::default(),

                shell: true,

                auto_start: false,

                stop_timeout_ms: 500,

                clear_logs_on_restart: false,

                schedule: None,
            },
        )]),

        task_order: vec!["quick".to_string()],
    });

    runtime
        .apply(Some("quick"), crate::protocol::Action::Start)
        .unwrap();

    state
        .sessions
        .lock()
        .expect("sessions lock")
        .insert("demo".to_string(), runtime);

    let mut tracked = std::collections::HashMap::new();

    let mut transitions = collect_run_transitions(&state, &mut tracked);

    for _ in 0..100 {
        if transitions.len() >= 2 {
            break;
        }

        thread::sleep(Duration::from_millis(10));

        transitions.extend(collect_run_transitions(&state, &mut tracked));
    }

    assert_eq!(transitions.len(), 2);

    let node_id = state.store.node_settings().unwrap().node_id;

    for transition in transitions {
        match transition {
            RunTransition::Started(session, snapshot, trigger) => {
                state
                    .store
                    .record_task_run_start(&node_id, &snapshot, &trigger, &session)
                    .unwrap();
            }

            RunTransition::Finished {
                session,

                task,

                generation,

                status,

                exit_code,

                error_message,
            } => {
                assert!(
                    state
                        .store
                        .finish_task_run(
                            &node_id,
                            &session,
                            &task,
                            generation,
                            &status,
                            exit_code,
                            error_message.as_deref(),
                        )
                        .unwrap()
                );
            }
        }
    }

    let runs = state
        .store
        .list_task_runs(&crate::protocol::TaskRunFilter {
            session: Some("demo".into()),

            task: Some("quick".into()),

            status: None,

            trigger: None,

            page: 1,

            page_size: 20,
        })
        .unwrap();

    assert_eq!(runs.total, 1);

    assert_eq!(runs.items[0].trigger, "manual");

    assert_eq!(runs.items[0].status, "failed");

    assert_eq!(runs.items[0].exit_code, Some(7));

    assert!(runs.items[0].duration_ms.is_some());
}

#[test]

pub(super) fn history_sampler_finishes_a_stopped_run_with_exit_code() {
    let state = DaemonState::new();

    let project = tempfile::tempdir().unwrap();

    let mut runtime = SessionRuntime::new(crate::config::ProjectDefinition {
        session: "demo".to_string(),

        project: project.path().to_path_buf(),

        source: "taskdeck.yaml".to_string(),

        tasks: std::collections::BTreeMap::from([(
            "long".to_string(),
            crate::config::TaskSpec {
                label: "long".to_string(),

                program: "trap 'exit 0' TERM; while :; do sleep 1; done".to_string(),

                args: Vec::new(),

                cwd: project.path().to_path_buf(),

                env: Default::default(),

                shell: true,

                auto_start: false,

                stop_timeout_ms: 3000,

                clear_logs_on_restart: false,

                schedule: None,
            },
        )]),

        task_order: vec!["long".to_string()],
    });

    runtime
        .apply(Some("long"), crate::protocol::Action::Start)
        .unwrap();

    state
        .sessions
        .lock()
        .expect("sessions lock")
        .insert("demo".to_string(), runtime);

    let mut tracked = std::collections::HashMap::new();

    let started = collect_run_transitions(&state, &mut tracked);

    assert_eq!(started.len(), 1);

    assert!(matches!(started[0], RunTransition::Started(_, _, _)));

    state
        .sessions
        .lock()
        .expect("sessions lock")
        .get_mut("demo")
        .unwrap()
        .apply(Some("long"), crate::protocol::Action::Stop)
        .unwrap();

    let finished = collect_run_transitions(&state, &mut tracked);

    assert_eq!(finished.len(), 1);

    let RunTransition::Finished {
        exit_code,

        session,

        task,

        generation,

        status,

        error_message,
    } = &finished[0]
    else {
        panic!("expected finished transition");
    };

    assert_eq!(*exit_code, None);

    assert_eq!(status, "stopped");

    assert!(
        error_message
            .as_deref()
            .is_some_and(|message| message.contains("signal"))
    );

    let node_id = state.store.node_settings().unwrap().node_id;

    if let RunTransition::Started(_, snapshot, trigger) = &started[0] {
        state
            .store
            .record_task_run_start(&node_id, snapshot, trigger, session)
            .unwrap();
    }

    state
        .store
        .finish_task_run(
            &node_id,
            session,
            task,
            *generation,
            status,
            *exit_code,
            error_message.as_deref(),
        )
        .unwrap();

    let runs = state
        .store
        .list_task_runs(&crate::protocol::TaskRunFilter {
            session: Some("demo".into()),

            task: Some("long".into()),

            status: None,

            trigger: None,

            page: 1,

            page_size: 20,
        })
        .unwrap();

    assert_eq!(runs.items[0].status, "stopped");

    assert_eq!(runs.items[0].exit_code, None);

    assert!(
        runs.items[0]
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("signal"))
    );
}

#[test]

pub(super) fn mcp_call_history_is_persisted_with_database_ids() {
    let state = DaemonState::new();

    for _ in 0..3 {
        let _ = state.store.record_mcp_call(call_record());
    }

    let page = state
        .store
        .list_mcp_calls(None, None, None, None, None, 1, 20)
        .unwrap();

    assert_eq!(page.total, 3);

    assert_eq!(page.items[0].id, 3);

    assert_eq!(page.items[0].tool, "taskdeck_control");

    assert!(state.store.mcp_call_detail(1).unwrap().is_some());
}
