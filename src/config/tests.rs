//! Config tests.

use super::*;

#[test]
fn discovers_vscode_tasks_and_yaml_overrides() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".vscode")).unwrap();
    fs::write(
        dir.path().join(".vscode/tasks.json"),
        r#"{
                // JSON with comments is valid in VS Code.
                "tasks": [{
                    "label": "api", "type": "process", "command": "dotnet",
                    "args": ["run", "--project", "${workspaceFolder}/api.csproj"],
                    "options": { "cwd": "${workspaceFolder}" }
                }],
            }"#,
    )
    .unwrap();
    fs::write(
            dir.path().join(PROJECT_CONFIG),
            "version: 1\nsession: demo\ntasks:\n  api:\n    auto_start: true\n  web:\n    command: npm\n    args: [run, dev]\n",
        )
        .unwrap();

    let definition = discover(dir.path(), None).unwrap();
    assert_eq!(definition.session, "demo");
    assert_eq!(definition.tasks.len(), 2);
    assert!(definition.tasks["api"].auto_start);
    assert!(definition.tasks["api"].args[2].ends_with("api.csproj"));
}

#[test]
fn discover_reports_missing_task_sources() {
    let dir = tempfile::tempdir().unwrap();

    let error = discover(dir.path(), None).unwrap_err().to_string();

    assert!(error.contains("no tasks found"));
}

#[test]
fn reads_editable_session_config_with_origin_metadata() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".vscode")).unwrap();
    fs::write(
        dir.path().join(".vscode/tasks.json"),
        r#"{
                "tasks": [{
                    "label": "api",
                    "type": "process",
                    "command": "cargo",
                    "args": ["run"]
                }]
            }"#,
    )
    .unwrap();
    fs::write(
        dir.path().join(PROJECT_CONFIG),
        r#"version: 1
session: demo
theme: nord
tasks:
  api:
    auto_start: true
    note: keep
  web:
    command: npm
    args: [run, dev]
    cwd: .
    shell: true
    auto_start: false
    stop_timeout_ms: 3000
"#,
    )
    .unwrap();

    let snapshot = read_session_config(dir.path(), "custom").unwrap();
    let api = snapshot
        .tasks
        .iter()
        .find(|task| task.label == "api")
        .unwrap();
    let web = snapshot
        .tasks
        .iter()
        .find(|task| task.label == "web")
        .unwrap();

    assert_eq!(snapshot.session, "custom");
    assert_eq!(snapshot.project, dir.path().canonicalize().unwrap());
    assert!(api.origin.imported);
    assert!(api.origin.has_yaml_override);
    assert!(!web.origin.imported);
    assert!(!web.origin.has_yaml_override);
}

#[test]
fn write_config_preserves_unknown_fields_and_deletion_semantics() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".vscode")).unwrap();
    fs::write(
        dir.path().join(".vscode/tasks.json"),
        r#"{
                "tasks": [{
                    "label": "api",
                    "type": "process",
                    "command": "cargo",
                    "args": ["run"]
                }]
            }"#,
    )
    .unwrap();
    fs::write(
        dir.path().join(PROJECT_CONFIG),
        r#"version: 1
session: demo
theme: nord
tasks:
  api:
    auto_start: true
    note: keep
  web:
    command: npm
    args: [run, dev]
    cwd: .
    shell: true
    auto_start: false
    stop_timeout_ms: 3000
    category: frontend
"#,
    )
    .unwrap();

    let snapshot = read_session_config(dir.path(), "custom").unwrap();
    write_session_config(dir.path(), &snapshot.revision, Vec::new()).unwrap();
    let saved = read_session_config(dir.path(), "custom").unwrap();
    let yaml = fs::read_to_string(dir.path().join(PROJECT_CONFIG)).unwrap();

    assert!(saved.tasks.is_empty());
    assert!(yaml.contains("theme: nord"));
    assert!(yaml.contains("note: keep"));
    assert!(yaml.contains("enabled: false"));
    assert!(!yaml.contains("web:"));
}

#[test]
fn write_config_rejects_stale_revisions() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
            dir.path().join(PROJECT_CONFIG),
            "version: 1\ntasks:\n  api:\n    command: cargo\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",
        )
        .unwrap();

    let snapshot = read_session_config(dir.path(), "demo").unwrap();
    fs::write(
            dir.path().join(PROJECT_CONFIG),
            "version: 1\ntasks:\n  api:\n    command: cargo\n    args: [test]\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",
        )
        .unwrap();

    let error = write_session_config(dir.path(), &snapshot.revision, snapshot.tasks_to_inputs())
        .unwrap_err();

    assert!(matches!(error, WriteConfigError::StaleRevision { .. }));
}

#[test]
fn write_config_validates_editable_tasks() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
            dir.path().join(PROJECT_CONFIG),
            "version: 1\ntasks:\n  api:\n    command: cargo\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",
        )
        .unwrap();
    let snapshot = read_session_config(dir.path(), "demo").unwrap();
    let error = write_session_config(
        dir.path(),
        &snapshot.revision,
        vec![EditableTaskInput {
            label: "".to_string(),
            command: "".to_string(),
            args: Vec::new(),
            cwd: "".to_string(),
            env: BTreeMap::new(),
            shell: true,
            auto_start: false,
            stop_timeout_ms: 0,
            clear_logs_on_restart: false,
            schedule: None,
        }],
    )
    .unwrap_err();

    assert!(matches!(error, WriteConfigError::Validation { .. }));
}

#[test]
fn imported_task_noop_round_trip_does_not_write_redundant_known_overrides() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".vscode")).unwrap();
    fs::write(
        dir.path().join(".vscode/tasks.json"),
        r#"{
                "tasks": [{
                    "label": "api",
                    "type": "process",
                    "command": "cargo",
                    "args": ["run"]
                }]
            }"#,
    )
    .unwrap();
    fs::write(
        dir.path().join(PROJECT_CONFIG),
        "version: 1\ntasks:\n  api:\n    note: keep\n",
    )
    .unwrap();

    let snapshot = read_session_config(dir.path(), "demo").unwrap();
    write_session_config(dir.path(), &snapshot.revision, snapshot.tasks_to_inputs()).unwrap();
    let yaml = fs::read_to_string(dir.path().join(PROJECT_CONFIG)).unwrap();

    assert!(yaml.contains("note: keep"));
    assert!(!yaml.contains("command: cargo"));
    assert!(!yaml.contains("args:"));
}

#[test]
fn imported_env_removal_is_persisted_as_a_tombstone() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".vscode")).unwrap();
    fs::write(
        dir.path().join(".vscode/tasks.json"),
        r#"{
                "tasks": [{
                    "label": "api",
                    "type": "process",
                    "command": "cargo",
                    "args": ["run"],
                    "options": {
                        "env": {
                            "KEEP": "1",
                            "DROP": "2"
                        }
                    }
                }]
            }"#,
    )
    .unwrap();
    fs::write(dir.path().join(PROJECT_CONFIG), "version: 1\n").unwrap();

    let snapshot = read_session_config(dir.path(), "demo").unwrap();
    let mut task = snapshot.tasks_to_inputs().remove(0);
    task.env.remove("DROP");
    write_session_config(dir.path(), &snapshot.revision, vec![task]).unwrap();

    let yaml: Value =
        serde_yaml::from_str(&fs::read_to_string(dir.path().join(PROJECT_CONFIG)).unwrap())
            .unwrap();
    let env = yaml
        .get("tasks")
        .and_then(|value| value.get("api"))
        .and_then(|value| value.get("env"))
        .and_then(Value::as_mapping)
        .unwrap();

    assert_eq!(env.get(yaml_key("DROP")), Some(&Value::Null));
    assert!(!env.contains_key(yaml_key("KEEP")));

    let saved = read_session_config(dir.path(), "demo").unwrap();
    let saved_task = saved.tasks.iter().find(|task| task.label == "api").unwrap();
    assert_eq!(saved_task.env.get("KEEP").map(String::as_str), Some("1"));
    assert!(!saved_task.env.contains_key("DROP"));
}

#[test]
fn imported_env_diff_only_serializes_changed_and_added_keys() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".vscode")).unwrap();
    fs::write(
        dir.path().join(".vscode/tasks.json"),
        r#"{
                "tasks": [{
                    "label": "api",
                    "type": "process",
                    "command": "cargo",
                    "args": ["run"],
                    "options": {
                        "env": {
                            "KEEP": "1",
                            "CHANGE": "base"
                        }
                    }
                }]
            }"#,
    )
    .unwrap();
    fs::write(dir.path().join(PROJECT_CONFIG), "version: 1\n").unwrap();

    let snapshot = read_session_config(dir.path(), "demo").unwrap();
    let mut task = snapshot.tasks_to_inputs().remove(0);
    task.env
        .insert("CHANGE".to_string(), "override".to_string());
    task.env.insert("ADD".to_string(), "new".to_string());
    write_session_config(dir.path(), &snapshot.revision, vec![task]).unwrap();

    let yaml: Value =
        serde_yaml::from_str(&fs::read_to_string(dir.path().join(PROJECT_CONFIG)).unwrap())
            .unwrap();
    let env = yaml
        .get("tasks")
        .and_then(|value| value.get("api"))
        .and_then(|value| value.get("env"))
        .and_then(Value::as_mapping)
        .unwrap();

    assert_eq!(
        env.get(yaml_key("CHANGE")),
        Some(&Value::String("override".to_string()))
    );
    assert_eq!(
        env.get(yaml_key("ADD")),
        Some(&Value::String("new".to_string()))
    );
    assert!(!env.contains_key(yaml_key("KEEP")));

    let saved = read_session_config(dir.path(), "demo").unwrap();
    let saved_task = saved.tasks.iter().find(|task| task.label == "api").unwrap();
    assert_eq!(saved_task.env.get("KEEP").map(String::as_str), Some("1"));
    assert_eq!(
        saved_task.env.get("CHANGE").map(String::as_str),
        Some("override")
    );
    assert_eq!(saved_task.env.get("ADD").map(String::as_str), Some("new"));
}

#[test]
fn discover_rejects_stop_timeout_above_maximum() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
            dir.path().join(PROJECT_CONFIG),
            "version: 1\ntasks:\n  api:\n    command: cargo\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 300001\n",
        )
        .unwrap();

    let error = discover(dir.path(), None).unwrap_err().to_string();

    assert!(error.contains("stop_timeout_ms must be between 1 and 300000"));
}

#[test]
fn write_config_rejects_stop_timeout_above_maximum() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
            dir.path().join(PROJECT_CONFIG),
            "version: 1\ntasks:\n  api:\n    command: cargo\n    cwd: .\n    shell: true\n    auto_start: false\n    stop_timeout_ms: 3000\n",
        )
        .unwrap();
    let snapshot = read_session_config(dir.path(), "demo").unwrap();
    let error = write_session_config(
        dir.path(),
        &snapshot.revision,
        vec![EditableTaskInput {
            label: "api".to_string(),
            command: "cargo".to_string(),
            args: Vec::new(),
            cwd: ".".to_string(),
            env: BTreeMap::new(),
            shell: true,
            auto_start: false,
            stop_timeout_ms: 300_001,
            clear_logs_on_restart: false,
            schedule: None,
        }],
    )
    .unwrap_err();

    assert!(matches!(error, WriteConfigError::Validation { .. }));
}

#[test]
fn task_order_and_restart_history_setting_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
            dir.path().join(PROJECT_CONFIG),
            "version: 1\ntask_order: [web]\ntasks:\n  api:\n    command: echo api\n  web:\n    command: echo web\n",
        )
        .unwrap();
    let snapshot = read_session_config(dir.path(), "demo").unwrap();
    assert_eq!(
        snapshot
            .tasks
            .iter()
            .map(|task| task.label.as_str())
            .collect::<Vec<_>>(),
        ["web", "api"]
    );

    let mut tasks = snapshot.tasks_to_inputs();
    tasks.reverse();
    tasks[0].clear_logs_on_restart = true;
    write_session_config(dir.path(), &snapshot.revision, tasks).unwrap();

    let saved = read_session_config(dir.path(), "demo").unwrap();
    assert_eq!(
        saved
            .tasks
            .iter()
            .map(|task| task.label.as_str())
            .collect::<Vec<_>>(),
        ["api", "web"]
    );
    assert!(saved.tasks[0].clear_logs_on_restart);
    let yaml = fs::read_to_string(dir.path().join(PROJECT_CONFIG)).unwrap();
    assert!(yaml.contains("task_order:"));
    assert!(yaml.contains("clear_logs_on_restart: true"));
}

#[test]
fn task_order_rejects_unknown_and_duplicate_labels() {
    for order in ["[missing]", "[api, api]"] {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(PROJECT_CONFIG),
            format!("version: 1\ntask_order: {order}\ntasks:\n  api:\n    command: echo api\n"),
        )
        .unwrap();
        assert!(read_session_config(dir.path(), "demo").is_err());
    }
}

#[test]
fn workspace_env_overrides_daemon_and_task_env_overrides_workspace() {
    // The "daemon environment" is intentionally represented by the VS Code default VARIABLE=vscode;
    // this test proves the two layers below it without unsafe process-environment mutation.
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".vscode")).unwrap();
    fs::write(dir.path().join(".vscode/tasks.json"), r#"{"version":"2.0.0","tasks":[{"label":"api","type":"shell","command":"echo ready","options":{"env":{"VARIABLE":"vscode"}}}]}"#).unwrap();
    std::fs::write(dir.path().join(PROJECT_CONFIG),"version: 1\nworkspace_env:\n  VARIABLE: workspace\n  WORKSPACE_ONLY: yes\ntasks:\n  api:\n    env:\n      VARIABLE: task\n").unwrap();
    let definition = discover(dir.path(), Some("demo")).unwrap();
    let env = definition.tasks["api"].env.clone();
    assert_eq!(env["VARIABLE"], "task");
    assert_eq!(env["WORKSPACE_ONLY"], "yes");
}

#[test]
fn schedule_is_validated_loaded_and_persisted_by_editor() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(PROJECT_CONFIG),"version: 1\nworkspace_env:\n  APP_ENV: development\ntasks:\n  cleanup:\n    command: ./cleanup.sh\n    shell: true\n    cwd: .\n    auto_start: false\n    stop_timeout_ms: 3000\n    clear_logs_on_restart: false\n    schedule: \"*/10 * * * *\"\n").unwrap();
    let definition = discover(dir.path(), Some("demo")).unwrap();
    assert_eq!(
        definition.tasks["cleanup"].schedule.as_deref(),
        Some("*/10 * * * *")
    );
    let mut snapshot = read_session_config(dir.path(), "demo").unwrap();
    assert_eq!(
        snapshot.workspace_env.get("APP_ENV").map(String::as_str),
        Some("development")
    );
    snapshot.tasks[0].schedule = Some("bad cron".into());
    let error = write_session_config(dir.path(), &snapshot.revision, snapshot.tasks_to_inputs())
        .unwrap_err();
    assert!(matches!(error, WriteConfigError::Validation { .. }));
    snapshot = read_session_config(dir.path(), "demo").unwrap();
    snapshot.tasks[0].schedule = None;
    write_session_config(dir.path(), &snapshot.revision, snapshot.tasks_to_inputs()).unwrap();
    let saved = fs::read_to_string(dir.path().join(PROJECT_CONFIG)).unwrap();
    assert!(saved.contains("APP_ENV"));
    assert!(!saved.contains("*/10"));
}

#[test]
fn init_project_materializes_vscode_tasks_without_overwriting_source() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".vscode")).unwrap();
    let vscode = r#"{
        "tasks": [
            {"label":"api","type":"process","command":"cargo","args":["run"],"options":{"cwd":"."}},
            {"label":"web","type":"shell","command":"npm","args":["run","dev"]}
        ]
    }"#;
    fs::write(dir.path().join(".vscode/tasks.json"), vscode).unwrap();

    let initialized = init_project(dir.path(), Some("demo")).unwrap();
    assert_eq!(initialized.session, "demo");
    assert_eq!(
        fs::read_to_string(dir.path().join(".vscode/tasks.json")).unwrap(),
        vscode
    );
    let yaml = fs::read_to_string(initialized.config_path).unwrap();
    assert!(yaml.contains("session: demo"));
    assert!(yaml.contains("command: cargo"));
    assert!(yaml.contains("command: npm"));
    assert!(yaml.contains("task_order:"));
}

#[test]
fn init_project_creates_empty_template_and_refuses_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let initialized = init_project(dir.path(), None).unwrap();
    let yaml = fs::read_to_string(&initialized.config_path).unwrap();
    assert!(yaml.contains("version: 1"));
    assert!(yaml.contains("tasks: {}"));

    let error = init_project(dir.path(), None).unwrap_err().to_string();
    assert!(error.contains("refusing to overwrite"));
}
