//! Daemon lifecycle tests.

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

pub(super) fn worker_binds_to_loopback_by_default() {
    let state = DaemonState::new();

    assert_eq!(state.public_settings().bind_host, "127.0.0.1");
}

#[test]

pub(super) fn restores_registered_project_after_state_recreation() {
    let root = tempfile::tempdir().unwrap();

    let project = tempfile::tempdir().unwrap();

    fs::write(
        project.path().join("taskdeck.yaml"),
        "version: 1\nsession: restored\ntasks:\n  idle:\n    command: echo\n    args: [ready]\n",
    )
    .unwrap();

    let project = project.path().canonicalize().unwrap();

    let paths = GlobalPaths {
        root: root.path().to_path_buf(),

        socket: root.path().join("taskdeck.sock"),

        lock: root.path().join("daemon.lock"),

        log: root.path().join("daemon.log"),
    };

    StateStore::open(root.path())
        .unwrap()
        .upsert_registration("restored", &project)
        .unwrap();

    let state = DaemonState::load(&paths).unwrap();

    let response = dispatch(&state, Request::ListSessions);

    assert!(response.ok);

    assert_eq!(response.data.unwrap(), json!(["restored"]));
}

#[test]

pub(super) fn pure_master_rejects_local_registration() {
    let state = DaemonState::new();

    let settings = state
        .store
        .configure(crate::state::NodeSettingsUpdate {
            role: Some(crate::state::NodeRole::Leader),

            leader_mode: Some(crate::state::LeaderMode::PureMaster),

            ..crate::state::NodeSettingsUpdate::default()
        })
        .unwrap();

    *state.settings.lock().expect("node settings lock") = settings;

    let response = dispatch(
        &state,
        Request::Register {
            project: PathBuf::from("/tmp/missing"),

            session: None,
            allow_empty: false,
        },
    );

    assert!(!response.ok);

    assert!(response.message.contains("pure master"));
}

#[test]
pub(super) fn empty_projects_require_explicit_registration_opt_in() {
    let project = tempfile::tempdir().unwrap();
    fs::write(
        project.path().join("taskdeck.yaml"),
        "version: 1\ntasks: {}\n",
    )
    .unwrap();
    let state = DaemonState::new();

    let rejected = dispatch(
        &state,
        Request::Register {
            project: project.path().to_path_buf(),
            session: Some("empty".to_string()),
            allow_empty: false,
        },
    );
    assert!(!rejected.ok);
    assert!(rejected.message.contains("no tasks found"));

    let registered = dispatch(
        &state,
        Request::Register {
            project: project.path().to_path_buf(),
            session: Some("empty".to_string()),
            allow_empty: true,
        },
    );
    assert!(registered.ok);
    assert!(state.sessions.lock().unwrap().contains_key("empty"));
}
