//! Runtime tests.

use super::*;
use super::log_buffer::*;
use super::process::*;
use super::task::*;
use crate::config::{ProjectDefinition, TaskSpec};
use crate::protocol::*;
use std::collections::BTreeMap;
use std::thread;
use std::time::Duration;
    use std::path::PathBuf;

    fn long_running_task() -> TaskRuntime {
        TaskRuntime::new(TaskSpec {
            label: "clock".to_string(),
            program: "while true; do echo tick; sleep 0.05; done".to_string(),
            args: Vec::new(),
            cwd: PathBuf::from("/tmp"),
            env: BTreeMap::new(),
            shell: true,
            auto_start: false,
            stop_timeout_ms: 500,
            clear_logs_on_restart: false,
            schedule: None,
        })
    }

    fn task_spec(label: &str, program: &str, auto_start: bool) -> TaskSpec {
        TaskSpec {
            label: label.to_string(),
            program: program.to_string(),
            args: Vec::new(),
            cwd: PathBuf::from("/tmp"),
            env: BTreeMap::new(),
            shell: true,
            auto_start,
            stop_timeout_ms: 500,
            clear_logs_on_restart: false,
            schedule: None,
        }
    }

    #[test]
    fn controls_a_process_group_through_its_lifecycle() {
        let mut task = long_running_task();
        task.start().unwrap();
        assert_eq!(task.status, TaskStatus::Running);
        assert!(task.pid.is_some());

        task.pause().unwrap();
        assert_eq!(task.status, TaskStatus::Paused);
        task.resume().unwrap();
        assert_eq!(task.status, TaskStatus::Running);
        task.restart().unwrap();
        assert_eq!(task.status, TaskStatus::Running);
        task.set_service_observation(
            vec![ServiceEndpoint {
                bind_host: "127.0.0.1".to_string(),
                port: 41023,
                protocol: "tcp".to_string(),
                pid: task.pid,
                source: "socket".to_string(),
                state: "listening".to_string(),
            }],
            ServiceInspectionState::Listening,
        );
        task.stop().unwrap();
        assert_eq!(task.status, TaskStatus::Idle);
        assert!(task.pid.is_none());
        assert_eq!(task.service.inspection, ServiceInspectionState::NotRunning);
        assert!(task.service.endpoints.is_empty());
    }

    #[test]
    fn incremental_logs_reset_when_cursor_falls_behind_or_limit_is_exceeded() {
        let mut logs = LogBuffer::default();
        for index in 0..6 {
            logs.push("stdout", format!("line {index}"));
        }

        let initial = logs.snapshot(None, 3);
        assert!(!initial.reset);
        assert!(initial.generation > 0);
        assert_eq!(
            initial
                .lines
                .iter()
                .map(|line| line.seq)
                .collect::<Vec<_>>(),
            [4, 5, 6]
        );

        let incremental = logs.snapshot(Some(4), 3);
        assert!(!incremental.reset);
        assert_eq!(incremental.generation, initial.generation);
        assert_eq!(
            incremental
                .lines
                .iter()
                .map(|line| line.seq)
                .collect::<Vec<_>>(),
            [5, 6]
        );

        let overflow = logs.snapshot(Some(1), 3);
        assert!(overflow.reset);
        assert_eq!(
            overflow
                .lines
                .iter()
                .map(|line| line.seq)
                .collect::<Vec<_>>(),
            [4, 5, 6]
        );

        let cursor_from_replaced_task = logs.snapshot(Some(60), 3);
        assert!(cursor_from_replaced_task.reset);
        assert_eq!(
            cursor_from_replaced_task
                .lines
                .iter()
                .map(|line| line.seq)
                .collect::<Vec<_>>(),
            [4, 5, 6]
        );

        let replacement = LogBuffer::default().snapshot(None, 3);
        assert_ne!(replacement.generation, initial.generation);
    }

    #[test]
    fn decodes_non_utf8_log_bytes_without_interrupting_the_log_reader() {
        assert_eq!(decode_log_line(b"ready\r\n"), "ready");
        assert!(!decode_log_line(&[0x81, 0x82, b'\n']).is_empty());
    }

    #[test]
    fn preserves_ansi_sequences_for_the_log_renderer() {
        assert_eq!(
            decode_log_line(b"\x1b[32mVITE\x1b[0m ready\n"),
            "\x1b[32mVITE\x1b[0m ready"
        );
    }

    #[test]
    fn updates_tasks_while_preserving_unchanged_runtime_state() {
        let mut runtime = SessionRuntime::new(ProjectDefinition {
            session: "demo".to_string(),
            project: PathBuf::from("/tmp"),
            source: "taskdeck.yaml".to_string(),
            tasks: BTreeMap::from([
                ("keep".to_string(), task_spec("keep", "echo old", false)),
                (
                    "remove".to_string(),
                    task_spec("remove", "echo remove", false),
                ),
            ]),
            task_order: vec!["keep".to_string(), "remove".to_string()],
        });
        runtime
            .tasks
            .get_mut("keep")
            .unwrap()
            .push_system("retained log");

        runtime
            .update(ProjectDefinition {
                session: "demo".to_string(),
                project: PathBuf::from("/tmp"),
                source: ".vscode/tasks.json + taskdeck.yaml".to_string(),
                tasks: BTreeMap::from([
                    ("keep".to_string(), task_spec("keep", "echo new", false)),
                    ("add".to_string(), task_spec("add", "echo add", false)),
                ]),
                task_order: vec!["keep".to_string(), "add".to_string()],
            })
            .unwrap();

        let snapshot = runtime.snapshot(20).unwrap();
        assert_eq!(snapshot.source, ".vscode/tasks.json + taskdeck.yaml");
        assert_eq!(
            snapshot.tasks.keys().cloned().collect::<Vec<_>>(),
            ["add", "keep"]
        );
        assert_eq!(snapshot.tasks["keep"].command, "echo new");
        assert!(
            snapshot.tasks["keep"]
                .logs
                .iter()
                .any(|line| line.text == "retained log")
        );
        assert!(
            snapshot.tasks["keep"]
                .logs
                .iter()
                .any(|line| line.text.contains("configuration updated"))
        );
    }

    #[test]
    fn update_only_auto_starts_new_tasks_and_reports_start_failures() {
        let program = "while true; do sleep 1; done";
        let mut runtime = SessionRuntime::new(ProjectDefinition {
            session: "demo".to_string(),
            project: PathBuf::from("/tmp"),
            source: "taskdeck.yaml".to_string(),
            tasks: BTreeMap::from([("existing".to_string(), task_spec("existing", program, true))]),
            task_order: vec!["existing".to_string()],
        });
        runtime.auto_start();
        runtime
            .apply(Some("existing"), Action::Stop)
            .expect("stop existing auto-start task");

        runtime
            .update(ProjectDefinition {
                session: "demo".to_string(),
                project: PathBuf::from("/tmp"),
                source: "taskdeck.yaml".to_string(),
                tasks: BTreeMap::from([
                    ("existing".to_string(), task_spec("existing", program, true)),
                    ("new".to_string(), task_spec("new", program, true)),
                ]),
                task_order: vec!["existing".to_string(), "new".to_string()],
            })
            .expect("update with new auto-start task");

        let snapshot = runtime.snapshot(20).unwrap();
        assert_eq!(snapshot.tasks["existing"].status, TaskStatus::Idle);
        assert_eq!(snapshot.tasks["new"].status, TaskStatus::Running);
        runtime.stop_all();

        let mut invalid = task_spec("broken", program, true);
        invalid.cwd = PathBuf::from("/tmp/taskdeck-directory-that-does-not-exist");
        let error = runtime
            .update(ProjectDefinition {
                session: "demo".to_string(),
                project: PathBuf::from("/tmp"),
                source: "taskdeck.yaml".to_string(),
                tasks: BTreeMap::from([("broken".to_string(), invalid)]),
                task_order: vec!["broken".to_string()],
            })
            .unwrap_err();
        assert!(error.to_string().contains("failed to auto-start new task"));
        assert!(!runtime.has_task("broken"));
    }

    #[test]
    fn clearing_history_replaces_log_generation_and_invalidates_metric_identity() {
        let mut task = TaskRuntime::new(task_spec("api", "echo ready", false));
        task.push_system("old output");
        let before_logs = task.logs(None, 20).unwrap();
        let before_history = task.history_generation;

        task.clear_history();

        let after_logs = task.logs(None, 20).unwrap();
        assert_ne!(after_logs.generation, before_logs.generation);
        assert!(after_logs.lines.is_empty());
        assert_ne!(task.history_generation, before_history);
    }

    #[test]
    fn restart_clear_setting_replaces_history_between_stop_and_start() {
        let mut task = long_running_task();
        task.spec.clear_logs_on_restart = true;
        task.start().unwrap();
        task.push_system("old output");
        let before_logs = task.logs(None, 20).unwrap();
        let before_history = task.history_generation;

        assert!(task.restart().unwrap());

        let after_logs = task.logs(None, 20).unwrap();
        assert_ne!(after_logs.generation, before_logs.generation);
        assert_ne!(task.history_generation, before_history);
        assert!(
            after_logs
                .lines
                .iter()
                .all(|line| line.text != "old output")
        );
        assert_eq!(task.status, TaskStatus::Running);
        task.stop().unwrap();
    }

    #[test]
    fn snapshot_exposes_configured_order() {
        let mut runtime = SessionRuntime::new(ProjectDefinition {
            session: "demo".to_string(),
            project: PathBuf::from("/tmp"),
            source: "taskdeck.yaml".to_string(),
            tasks: BTreeMap::from([
                ("api".to_string(), task_spec("api", "echo api", false)),
                ("web".to_string(), task_spec("web", "echo web", false)),
            ]),
            task_order: vec!["web".to_string(), "api".to_string()],
        });
        assert_eq!(runtime.snapshot(0).unwrap().task_order, ["web", "api"]);
    }

    #[test]
    fn scheduled_start_runs_once_then_skips_running_task() {
        let mut tasks = BTreeMap::new();
        tasks.insert(
            "one-shot".to_string(),
            TaskSpec {
                label: "one-shot".into(),
                program: "sleep".into(),
                args: vec!["0.05".into()],
                cwd: PathBuf::from("/tmp"),
                env: BTreeMap::new(),
                shell: true,
                auto_start: false,
                stop_timeout_ms: 1000,
                clear_logs_on_restart: false,
                schedule: Some("* * * * *".into()),
            },
        );
        let mut runtime = SessionRuntime::new(ProjectDefinition {
            session: "demo".into(),
            project: PathBuf::from("/tmp"),
            source: "test".into(),
            tasks,
            task_order: vec!["one-shot".into()],
        });
        assert!(runtime.scheduled_start("one-shot").unwrap());
        for _ in 0..50 {
            if let Ok(snapshot) = runtime.snapshot(0) {
                if snapshot.tasks["one-shot"].status == TaskStatus::Running {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!runtime.scheduled_start("one-shot").unwrap());
    }
