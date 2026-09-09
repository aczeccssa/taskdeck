//! Metric sampling tests.

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

pub(super) fn aggregates_only_the_root_process_and_its_descendants() {
    let tree = aggregate_process_tree(
        10,
        &[
            observed_process(10, Some(1), 25.0, 100),
            observed_process(11, Some(10), 10.0, 40),
            observed_process(12, Some(11), 5.0, 20),
            observed_process(99, Some(1), 80.0, 999),
        ],
    )
    .unwrap();

    assert_eq!(
        tree.aggregate,
        TaskMetricsAggregate {
            cpu_percent: 40.0,

            memory_bytes: 160,

            process_count: 3,
        }
    );

    assert_eq!(
        tree.processes
            .iter()
            .map(|process| process.pid)
            .collect::<Vec<_>>(),
        vec![10, 11, 12]
    );
}

#[test]

pub(super) fn task_metrics_store_truncates_history_and_preserves_it_after_stop() {
    let mut store = TaskMetricsStore::default();

    for timestamp_ms in 1..=(MAX_TASK_METRIC_SAMPLES as u64 + 5) {
        store.record(
            "demo",
            "api",
            timestamp_ms,
            Some(AggregatedProcessTree {
                aggregate: TaskMetricsAggregate {
                    cpu_percent: timestamp_ms as f32,

                    memory_bytes: timestamp_ms * 10,

                    process_count: 2,
                },

                processes: vec![TaskProcessSnapshot {
                    pid: 42,

                    ppid: Some(1),

                    name: "api".to_string(),

                    cpu_percent: timestamp_ms as f32,

                    memory_bytes: timestamp_ms * 10,

                    status: "run".to_string(),

                    run_time_seconds: timestamp_ms,
                }],
            }),
        );
    }

    let running = store.snapshot("demo", "api", 600);

    assert!(running.running);

    assert_eq!(running.samples.len(), MAX_TASK_METRIC_SAMPLES);

    assert_eq!(running.samples.first().unwrap().timestamp_ms, 6);

    assert_eq!(running.samples.last().unwrap().timestamp_ms, 605);

    store.record("demo", "api", 606, None);

    let stopped = store.snapshot("demo", "api", 600);

    assert!(!stopped.running);

    assert_eq!(stopped.current, TaskMetricsAggregate::zero());

    assert!(stopped.processes.is_empty());

    assert_eq!(stopped.samples.len(), MAX_TASK_METRIC_SAMPLES);

    assert_eq!(stopped.samples.first().unwrap().timestamp_ms, 6);

    assert_eq!(stopped.samples.last().unwrap().timestamp_ms, 605);
}

#[test]

pub(super) fn task_metrics_restart_markers_are_windowed_and_clear_with_history() {
    let mut store = TaskMetricsStore::default();

    for timestamp_ms in [100, 200, 300] {
        store.record(
            "demo",
            "api",
            timestamp_ms,
            Some(AggregatedProcessTree {
                aggregate: TaskMetricsAggregate::zero(),

                processes: Vec::new(),
            }),
        );
    }

    store.mark_restart("demo", "api", 150);

    store.mark_restart("demo", "api", 250);

    assert_eq!(store.snapshot("demo", "api", 2).restart_markers_ms, [250]);

    store.clear_task("demo", "api");

    let cleared = store.snapshot("demo", "api", 600);

    assert!(cleared.samples.is_empty());

    assert!(cleared.restart_markers_ms.is_empty());

    assert!(cleared.processes.is_empty());
}

#[test]

pub(super) fn task_metrics_prune_deleted_tasks_but_keep_existing_stopped_history() {
    let state = DaemonState::new();

    state.sessions.lock().expect("sessions lock").insert(
        "demo".to_string(),
        SessionRuntime::new(crate::config::ProjectDefinition {
            session: "demo".to_string(),

            project: PathBuf::from("/tmp"),

            source: "taskdeck.yaml".to_string(),

            tasks: std::collections::BTreeMap::from([(
                "worker".to_string(),
                crate::config::TaskSpec {
                    label: "worker".to_string(),

                    program: "sleep".to_string(),

                    args: vec!["60".to_string()],

                    cwd: PathBuf::from("/tmp"),

                    env: Default::default(),

                    shell: false,

                    auto_start: false,

                    stop_timeout_ms: 500,

                    clear_logs_on_restart: false,

                    schedule: None,
                },
            )]),

            task_order: vec!["worker".to_string()],
        }),
    );

    let worker_observation = Some(AggregatedProcessTree {
        aggregate: TaskMetricsAggregate {
            cpu_percent: 5.0,

            memory_bytes: 50,

            process_count: 1,
        },

        processes: vec![TaskProcessSnapshot {
            pid: 7,

            ppid: Some(1),

            name: "worker".to_string(),

            cpu_percent: 5.0,

            memory_bytes: 50,

            status: "run".to_string(),

            run_time_seconds: 1,
        }],
    });

    let mut metrics = state.task_metrics.lock().expect("task metrics lock");

    metrics.record("demo", "api", 1, worker_observation.clone());

    metrics.record("demo", "worker", 2, worker_observation);

    drop(metrics);

    sample_task_metrics_with(&state, 3, || panic!("refresh should not run"));

    let metrics = state.task_metrics.lock().expect("task metrics lock");

    assert_eq!(metrics.entries.len(), 1);

    assert!(metrics.entries.contains_key(&TaskMetricsKey {
        session: "demo".to_string(),

        task: "worker".to_string(),
    }));

    let worker = metrics.snapshot("demo", "worker", 600);

    assert!(!worker.running);

    assert_eq!(worker.samples.len(), 1);

    assert_eq!(metrics.snapshot("demo", "api", 600).samples.len(), 0);
}

#[test]

pub(super) fn sample_task_metrics_skips_refresh_when_no_running_targets_exist() {
    let state = DaemonState::new();

    state.sessions.lock().expect("sessions lock").insert(
        "demo".to_string(),
        SessionRuntime::new(crate::config::ProjectDefinition {
            session: "demo".to_string(),

            project: PathBuf::from("/tmp"),

            source: "taskdeck.yaml".to_string(),

            tasks: std::collections::BTreeMap::from([(
                "idle".to_string(),
                crate::config::TaskSpec {
                    label: "idle".to_string(),

                    program: "sleep".to_string(),

                    args: vec!["60".to_string()],

                    cwd: PathBuf::from("/tmp"),

                    env: Default::default(),

                    shell: false,

                    auto_start: false,

                    stop_timeout_ms: 500,

                    clear_logs_on_restart: false,

                    schedule: None,
                },
            )]),

            task_order: vec!["idle".to_string()],
        }),
    );

    let mut refreshes = 0usize;

    sample_task_metrics_with(&state, 1, || {
        refreshes += 1;

        Vec::new()
    });

    assert_eq!(refreshes, 0);
}

#[test]

pub(super) fn stale_generation_targets_are_discarded_before_recording() {
    let state = DaemonState::new();

    let mut runtime = SessionRuntime::new(crate::config::ProjectDefinition {
        session: "demo".to_string(),

        project: PathBuf::from("/tmp"),

        source: "taskdeck.yaml".to_string(),

        tasks: std::collections::BTreeMap::from([(
            "clock".to_string(),
            crate::config::TaskSpec {
                label: "clock".to_string(),

                program: "while true; do sleep 1; done".to_string(),

                args: Vec::new(),

                cwd: PathBuf::from("/tmp"),

                env: Default::default(),

                shell: true,

                auto_start: false,

                stop_timeout_ms: 500,

                clear_logs_on_restart: false,

                schedule: None,
            },
        )]),

        task_order: vec!["clock".to_string()],
    });

    runtime
        .apply(Some("clock"), crate::protocol::Action::Start)
        .unwrap();

    state
        .sessions
        .lock()
        .expect("sessions lock")
        .insert("demo".to_string(), runtime);

    let stale_targets = collect_metric_targets(&state);

    {
        let mut sessions = state.sessions.lock().expect("sessions lock");

        let runtime = sessions.get_mut("demo").unwrap();

        runtime
            .apply(Some("clock"), crate::protocol::Action::Restart)
            .unwrap();
    }

    record_task_metrics_for_targets(
        &state,
        &stale_targets,
        10,
        &[observed_process(
            stale_targets[0].root_pid.unwrap(),
            Some(1),
            33.0,
            99,
        )],
    );

    let snapshot = state
        .task_metrics
        .lock()
        .expect("task metrics lock")
        .snapshot("demo", "clock", 600);

    assert!(!snapshot.running);

    assert_eq!(snapshot.current, TaskMetricsAggregate::zero());

    assert!(snapshot.samples.is_empty());

    state
        .sessions
        .lock()
        .expect("sessions lock")
        .get_mut("demo")
        .unwrap()
        .stop_all();
}

#[test]

pub(super) fn collect_metric_targets_excludes_naturally_exited_tasks_without_snapshot_or_action() {
    let state = DaemonState::new();

    let mut runtime = SessionRuntime::new(crate::config::ProjectDefinition {
        session: "demo".to_string(),

        project: PathBuf::from("/tmp"),

        source: "taskdeck.yaml".to_string(),

        tasks: std::collections::BTreeMap::from([(
            "flash".to_string(),
            crate::config::TaskSpec {
                label: "flash".to_string(),

                program: "sleep 0.05".to_string(),

                args: Vec::new(),

                cwd: PathBuf::from("/tmp"),

                env: Default::default(),

                shell: true,

                auto_start: false,

                stop_timeout_ms: 500,

                clear_logs_on_restart: false,

                schedule: None,
            },
        )]),

        task_order: vec!["flash".to_string()],
    });

    runtime
        .apply(Some("flash"), crate::protocol::Action::Start)
        .unwrap();

    state
        .sessions
        .lock()
        .expect("sessions lock")
        .insert("demo".to_string(), runtime);

    wait_for(Duration::from_secs(2), || {
        collect_metric_targets(&state)
            .first()
            .is_some_and(|target| target.root_pid.is_none())
    });

    let targets = collect_metric_targets(&state);

    assert_eq!(targets.len(), 1);

    assert_eq!(targets[0].session, "demo");

    assert_eq!(targets[0].task, "flash");

    assert_eq!(targets[0].root_pid, None);
}
