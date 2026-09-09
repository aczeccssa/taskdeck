//! Shared test helpers.

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
use crate::config;
use crate::protocol::*;
use crate::runtime::{SessionRuntime, Sessions};
use crate::state::{NodeRole, NodeSettings, StateStore};

pub(super) fn task_input(label: &str, command: &str) -> EditableTaskInput {
    EditableTaskInput {
        label: label.to_string(),

        command: command.to_string(),

        args: Vec::new(),

        cwd: ".".to_string(),

        env: Default::default(),

        shell: true,

        auto_start: false,

        stop_timeout_ms: 3_000,

        clear_logs_on_restart: false,

        schedule: None,
    }
}

pub(super) fn observed_process(
    pid: u32,

    ppid: Option<u32>,

    cpu_percent: f32,

    memory_bytes: u64,
) -> ObservedProcess {
    ObservedProcess {
        pid,

        ppid,

        name: format!("proc-{pid}"),

        cpu_percent,

        memory_bytes,

        status: "run".to_string(),

        run_time_seconds: pid as u64,
    }
}

pub(super) fn wait_for(deadline: Duration, mut condition: impl FnMut() -> bool) {
    let start = Instant::now();

    while start.elapsed() < deadline {
        if condition() {
            return;
        }

        thread::sleep(Duration::from_millis(10));
    }

    assert!(condition(), "condition was not met within {deadline:?}");
}
