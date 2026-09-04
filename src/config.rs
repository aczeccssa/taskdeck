use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

use crate::protocol::{EditableTask, EditableTaskInput, EditableTaskOrigin, SessionConfigSnapshot};
use crate::state::validate_cron_expression;

pub const PROJECT_CONFIG: &str = "taskdeck.yaml";
const DEFAULT_STOP_TIMEOUT_MS: u64 = 3_000;
pub const MAX_STOP_TIMEOUT_MS: u64 = 300_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSpec {
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub shell: bool,
    pub auto_start: bool,
    pub stop_timeout_ms: u64,
    pub clear_logs_on_restart: bool,
    pub schedule: Option<String>,
}

impl TaskSpec {
    pub fn display_command(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug, Clone)]
pub struct ProjectDefinition {
    pub session: String,
    pub project: PathBuf,
    pub source: String,
    pub tasks: BTreeMap<String, TaskSpec>,
    pub task_order: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct VscodeFile {
    #[serde(default)]
    tasks: Vec<VscodeTask>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VscodeTask {
    label: String,
    #[serde(rename = "type", default)]
    kind: String,
    command: String,
    #[serde(default)]
    args: Vec<JsonArg>,
    #[serde(default)]
    options: VscodeOptions,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum JsonArg {
    Text(String),
    Number(serde_json::Number),
    Bool(bool),
}

impl JsonArg {
    fn render(&self) -> String {
        match self {
            Self::Text(value) => value.clone(),
            Self::Number(value) => value.to_string(),
            Self::Bool(value) => value.to_string(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct VscodeOptions {
    cwd: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct YamlConfig {
    #[serde(default = "default_version")]
    version: u32,
    session: Option<String>,
    #[serde(default)]
    workspace_env: BTreeMap<String, String>,
    #[serde(default)]
    task_order: Vec<String>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(default)]
struct TaskOverride {
    enabled: Option<bool>,
    command: Option<String>,
    args: Option<Vec<String>>,
    cwd: Option<String>,
    env: Option<BTreeMap<String, Option<String>>>,
    shell: Option<bool>,
    auto_start: Option<bool>,
    stop_timeout_ms: Option<u64>,
    clear_logs_on_restart: Option<bool>,
    schedule: Option<String>,
}

#[derive(Debug, Clone)]
struct TaskYamlEntry {
    raw: Mapping,
    patch: TaskOverride,
}

#[derive(Debug, Clone)]
struct YamlDocument {
    workspace_env: BTreeMap<String, String>,
    root: Mapping,
    tasks: BTreeMap<String, TaskYamlEntry>,
    declared_order: Vec<String>,
    task_order: Vec<String>,
    session: Option<String>,
    raw_content: String,
}

#[derive(Debug, Clone)]
struct ProjectConfigState {
    project: PathBuf,
    source: String,
    vscode_raw_content: Option<String>,
    vscode_tasks: BTreeMap<String, EditableTaskInput>,
    merged_tasks: BTreeMap<String, EditableTaskInput>,
    workspace_env: BTreeMap<String, String>,
    task_order: Vec<String>,
    yaml: Option<YamlDocument>,
}

#[derive(Debug)]
pub enum WriteConfigError {
    StaleRevision { current_revision: String },
    Validation { message: String },
    Other(anyhow::Error),
}

pub struct PreparedSessionConfigWrite {
    project: PathBuf,
    source: String,
    expected_revision: String,
    vscode_tasks: BTreeMap<String, EditableTaskInput>,
    merged_tasks: BTreeMap<String, EditableTaskInput>,
    workspace_env: BTreeMap<String, String>,
    task_order: Vec<String>,
    yaml_task_labels: HashSet<String>,
    temp_path: PathBuf,
    finalized: bool,
}

impl WriteConfigError {
    fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
        }
    }
}

impl PreparedSessionConfigWrite {
    pub fn project_definition(&self, session: &str) -> Result<ProjectDefinition> {
        validate_session_name(session)?;
        let tasks = self
            .merged_tasks
            .values()
            .map(|task| {
                Ok((
                    task.label.clone(),
                    compile_task(&self.project, task, &self.workspace_env).map_err(|error| {
                        anyhow::anyhow!("invalid task '{}': {error}", task.label)
                    })?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        Ok(ProjectDefinition {
            session: session.to_string(),
            project: self.project.clone(),
            source: self.source.clone(),
            tasks,
            task_order: self.task_order.clone(),
        })
    }

    pub fn session_snapshot(&self, session: &str) -> Result<SessionConfigSnapshot> {
        validate_session_name(session)?;
        let tasks = self
            .task_order
            .iter()
            .filter_map(|label| self.merged_tasks.get(label))
            .map(|task| EditableTask {
                label: task.label.clone(),
                command: task.command.clone(),
                args: task.args.clone(),
                cwd: task.cwd.clone(),
                env: task.env.clone(),
                shell: task.shell,
                auto_start: task.auto_start,
                stop_timeout_ms: task.stop_timeout_ms,
                clear_logs_on_restart: task.clear_logs_on_restart,
                schedule: task.schedule.clone(),
                origin: EditableTaskOrigin {
                    imported: self.vscode_tasks.contains_key(&task.label),
                    has_yaml_override: self.yaml_task_labels.contains(&task.label)
                        && self.vscode_tasks.contains_key(&task.label),
                },
            })
            .collect();
        Ok(SessionConfigSnapshot {
            session: session.to_string(),
            project: self.project.clone(),
            source: self.source.clone(),
            revision: revision_for_project(&self.project)?,
            workspace_env: self.workspace_env.clone(),
            tasks,
        })
    }

    pub fn finalize(&mut self) -> std::result::Result<(), WriteConfigError> {
        let current_revision =
            revision_for_project(&self.project).map_err(WriteConfigError::Other)?;
        if current_revision != self.expected_revision {
            return Err(WriteConfigError::StaleRevision { current_revision });
        }
        rename_temp_file(&self.temp_path, &self.project.join(PROJECT_CONFIG))
            .map_err(WriteConfigError::Other)?;
        sync_parent_directory(&self.project.join(PROJECT_CONFIG))
            .map_err(WriteConfigError::Other)?;
        self.finalized = true;
        Ok(())
    }
}

impl Drop for PreparedSessionConfigWrite {
    fn drop(&mut self) {
        if !self.finalized {
            let _ = fs::remove_file(&self.temp_path);
        }
    }
}

fn default_version() -> u32 {
    1
}

pub fn discover(project: &Path, requested_session: Option<&str>) -> Result<ProjectDefinition> {
    discover_inner(project, requested_session, false)
}

fn discover_inner(
    project: &Path,
    requested_session: Option<&str>,
    allow_empty: bool,
) -> Result<ProjectDefinition> {
    let state = load_project_state(project)?;
    if !allow_empty && state.merged_tasks.is_empty() {
        bail!(
            "no tasks found; add .vscode/tasks.json or {}",
            state.project.join(PROJECT_CONFIG).display()
        );
    }

    let default_session = state
        .project
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project");
    let session = requested_session
        .map(str::to_owned)
        .or_else(|| state.yaml.as_ref().and_then(|yaml| yaml.session.clone()))
        .unwrap_or_else(|| default_session.to_owned());
    validate_session_name(&session)?;

    let tasks = state
        .merged_tasks
        .values()
        .map(|task| {
            Ok((
                task.label.clone(),
                compile_task(&state.project, task, &state.workspace_env)
                    .map_err(|error| anyhow::anyhow!("invalid task '{}': {error}", task.label))?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    Ok(ProjectDefinition {
        session,
        project: state.project,
        source: state.source,
        tasks,
        task_order: state.task_order,
    })
}

pub fn read_session_config(project: &Path, session: &str) -> Result<SessionConfigSnapshot> {
    validate_session_name(session)?;
    let state = load_project_state(project)?;
    let tasks = state
        .task_order
        .iter()
        .filter_map(|label| state.merged_tasks.get(label))
        .map(|task| EditableTask {
            label: task.label.clone(),
            command: task.command.clone(),
            args: task.args.clone(),
            cwd: task.cwd.clone(),
            env: task.env.clone(),
            shell: task.shell,
            auto_start: task.auto_start,
            stop_timeout_ms: task.stop_timeout_ms,
            clear_logs_on_restart: task.clear_logs_on_restart,
            schedule: task.schedule.clone(),
            origin: EditableTaskOrigin {
                imported: state.vscode_tasks.contains_key(&task.label),
                has_yaml_override: state
                    .yaml
                    .as_ref()
                    .is_some_and(|yaml| yaml.tasks.contains_key(&task.label))
                    && state.vscode_tasks.contains_key(&task.label),
            },
        })
        .collect();

    let revision = state.revision();
    Ok(SessionConfigSnapshot {
        session: session.to_string(),
        project: state.project,
        source: state.source,
        revision,
        workspace_env: state.workspace_env.clone(),
        tasks,
    })
}

#[cfg(test)]
pub fn write_session_config(
    project: &Path,
    revision: &str,
    tasks: Vec<EditableTaskInput>,
) -> std::result::Result<(), WriteConfigError> {
    let mut prepared = prepare_session_config_write(project, revision, tasks, None)?;
    prepared.finalize()
}

pub fn prepare_session_config_write(
    project: &Path,
    revision: &str,
    tasks: Vec<EditableTaskInput>,
    workspace_env: Option<&BTreeMap<String, String>>,
) -> std::result::Result<PreparedSessionConfigWrite, WriteConfigError> {
    prepare_session_config_write_inner(project, revision, tasks, workspace_env, None)
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn prepare_session_config_write_for_test(
    project: &Path,
    revision: &str,
    tasks: Vec<EditableTaskInput>,
    workspace_env: Option<&BTreeMap<String, String>>,
    post_check_delay: Option<Duration>,
) -> std::result::Result<PreparedSessionConfigWrite, WriteConfigError> {
    prepare_session_config_write_inner(project, revision, tasks, workspace_env, post_check_delay)
}

fn prepare_session_config_write_inner(
    project: &Path,
    revision: &str,
    tasks: Vec<EditableTaskInput>,
    workspace_env: Option<&BTreeMap<String, String>>,
    post_check_delay: Option<Duration>,
) -> std::result::Result<PreparedSessionConfigWrite, WriteConfigError> {
    let state = load_project_state(project).map_err(WriteConfigError::Other)?;
    let current_revision = state.revision();
    if revision != current_revision {
        return Err(WriteConfigError::StaleRevision { current_revision });
    }
    if let Some(delay) = post_check_delay {
        std::thread::sleep(delay);
    }

    let effective_workspace = match workspace_env {
        Some(values) => values,
        None => &state.workspace_env,
    };
    validate_workspace_env(effective_workspace).map_err(WriteConfigError::validation)?;
    let (submitted, task_order) = validate_submitted_tasks(tasks)?;
    let mut root = state
        .yaml
        .as_ref()
        .map(|yaml| yaml.root.clone())
        .unwrap_or_default();
    root.insert(yaml_key("version"), Value::from(1u32));
    root.insert(
        yaml_key("task_order"),
        Value::Sequence(task_order.iter().cloned().map(Value::String).collect()),
    );
    let saved_workspace = effective_workspace.clone();
    if saved_workspace.is_empty() {
        root.remove(yaml_key("workspace_env"));
    } else {
        root.insert(yaml_key("workspace_env"), yaml_string_map(&saved_workspace));
    }

    let mut task_entries = BTreeMap::new();
    for (label, task) in &submitted {
        let existing_raw = state
            .yaml
            .as_ref()
            .and_then(|yaml| yaml.tasks.get(label))
            .map(|entry| entry.raw.clone())
            .unwrap_or_default();
        let mapping = match state.vscode_tasks.get(label) {
            Some(base) => build_imported_task_mapping(base, task, existing_raw),
            None => build_yaml_task_mapping(task, existing_raw),
        };
        if !mapping.is_empty() {
            task_entries.insert(label.clone(), mapping);
        }
    }

    for label in state.vscode_tasks.keys() {
        if submitted.contains_key(label) {
            continue;
        }
        let existing_raw = state
            .yaml
            .as_ref()
            .and_then(|yaml| yaml.tasks.get(label))
            .map(|entry| entry.raw.clone())
            .unwrap_or_default();
        let mapping = build_disabled_import_mapping(existing_raw);
        task_entries.insert(label.clone(), mapping);
    }

    let yaml_task_labels = task_entries.keys().cloned().collect::<HashSet<_>>();
    let mut tasks_value = Mapping::new();
    for (label, entry) in task_entries {
        tasks_value.insert(yaml_key(&label), Value::Mapping(entry));
    }
    root.insert(yaml_key("tasks"), Value::Mapping(tasks_value));

    let serialized = serde_yaml::to_string(&Value::Mapping(root))
        .map_err(|error| WriteConfigError::Other(error.into()))?;
    let temp_path = write_temp_config_file(&state.project.join(PROJECT_CONFIG), &serialized)
        .map_err(WriteConfigError::Other)?;
    Ok(PreparedSessionConfigWrite {
        project: state.project,
        source: if state.vscode_tasks.is_empty() {
            "taskdeck.yaml".to_string()
        } else {
            ".vscode/tasks.json + taskdeck.yaml".to_string()
        },
        expected_revision: revision.to_string(),
        vscode_tasks: state.vscode_tasks,
        merged_tasks: submitted,
        workspace_env: saved_workspace,
        task_order,
        yaml_task_labels,
        temp_path,
        finalized: false,
    })
}

fn load_project_state(project: &Path) -> Result<ProjectConfigState> {
    let project = project
        .canonicalize()
        .with_context(|| format!("project directory does not exist: {}", project.display()))?;
    let (vscode_tasks, vscode_order, vscode_raw_content) = load_vscode_tasks(&project)?;
    let yaml = load_yaml_document(&project)?;
    let merged_tasks = merge_tasks(&vscode_tasks, yaml.as_ref())?;
    let mut source_order = vscode_order;
    if let Some(yaml) = &yaml {
        for label in &yaml.declared_order {
            if !source_order.contains(label) {
                source_order.push(label.clone());
            }
        }
    }
    let configured_order = yaml
        .as_ref()
        .map(|yaml| yaml.task_order.as_slice())
        .unwrap_or_default();
    let task_order = resolve_task_order(configured_order, source_order, &merged_tasks)?;
    let source = match (vscode_raw_content.is_some(), yaml.is_some()) {
        (true, true) => ".vscode/tasks.json + taskdeck.yaml",
        (true, false) => ".vscode/tasks.json",
        (false, true) => "taskdeck.yaml",
        (false, false) => "taskdeck.yaml",
    };

    Ok(ProjectConfigState {
        project,
        source: source.to_string(),
        vscode_raw_content,
        vscode_tasks,
        merged_tasks,
        workspace_env: yaml
            .as_ref()
            .map(|yaml| yaml.workspace_env.clone())
            .unwrap_or_default(),
        task_order,
        yaml,
    })
}

type LoadedVscodeTasks = (
    BTreeMap<String, EditableTaskInput>,
    Vec<String>,
    Option<String>,
);

fn load_vscode_tasks(project: &Path) -> Result<LoadedVscodeTasks> {
    let path = project.join(".vscode/tasks.json");
    if !path.exists() {
        return Ok((BTreeMap::new(), Vec::new(), None));
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let file: VscodeFile =
        json5::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    let mut tasks = BTreeMap::new();
    let mut order = Vec::new();
    for task in file.tasks {
        let editable = EditableTaskInput {
            label: task.label.clone(),
            command: task.command,
            args: task.args.iter().map(JsonArg::render).collect(),
            cwd: task.options.cwd.unwrap_or_else(|| ".".to_string()),
            env: task.options.env,
            shell: task.kind == "shell",
            auto_start: false,
            stop_timeout_ms: DEFAULT_STOP_TIMEOUT_MS,
            clear_logs_on_restart: false,
            schedule: None,
        };
        validate_task_input(&editable).map_err(|error| {
            anyhow::anyhow!("invalid VS Code task '{}': {error}", editable.label)
        })?;
        if tasks.insert(task.label.clone(), editable).is_some() {
            bail!("duplicate VS Code task label '{}'", task.label);
        }
        order.push(task.label);
    }
    Ok((tasks, order, Some(content)))
}

fn load_yaml_document(project: &Path) -> Result<Option<YamlDocument>> {
    let path = project.join(PROJECT_CONFIG);
    if !path.exists() {
        return Ok(None);
    }
    let raw_content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let raw_value: Value = serde_yaml::from_str(&raw_content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let root = match raw_value {
        Value::Mapping(mapping) => mapping,
        Value::Null => Mapping::new(),
        _ => bail!("{} must contain a YAML mapping", path.display()),
    };
    let config: YamlConfig = serde_yaml::from_value(Value::Mapping(root.clone()))
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if config.version != 1 {
        bail!(
            "unsupported taskdeck.yaml version {}; expected 1",
            config.version
        );
    }

    let mut tasks = BTreeMap::new();
    let mut declared_order = Vec::new();
    if let Some(value) = root.get(yaml_key("tasks")) {
        match value {
            Value::Mapping(mapping) => {
                for (key, value) in mapping {
                    let label = key
                        .as_str()
                        .with_context(|| format!("{} task names must be strings", path.display()))?
                        .to_string();
                    let raw = match value {
                        Value::Mapping(raw) => raw.clone(),
                        Value::Null => Mapping::new(),
                        _ => bail!("task '{label}' in {} must be a mapping", path.display()),
                    };
                    let patch: TaskOverride = serde_yaml::from_value(Value::Mapping(raw.clone()))
                        .with_context(|| {
                        format!("failed to parse task '{label}' in {}", path.display())
                    })?;
                    if let Some(timeout) = patch.stop_timeout_ms {
                        if timeout == 0 || timeout > MAX_STOP_TIMEOUT_MS {
                            bail!(
                                "task '{label}' stop_timeout_ms must be between 1 and {MAX_STOP_TIMEOUT_MS}"
                            );
                        }
                    }
                    declared_order.push(label.clone());
                    tasks.insert(label, TaskYamlEntry { raw, patch });
                }
            }
            Value::Null => {}
            _ => bail!("{} tasks must be a mapping", path.display()),
        }
    }

    Ok(Some(YamlDocument {
        workspace_env: config.workspace_env,
        root,
        tasks,
        declared_order,
        task_order: config.task_order,
        session: config.session,
        raw_content,
    }))
}

fn merge_tasks(
    vscode_tasks: &BTreeMap<String, EditableTaskInput>,
    yaml: Option<&YamlDocument>,
) -> Result<BTreeMap<String, EditableTaskInput>> {
    let mut tasks = vscode_tasks.clone();
    if let Some(yaml) = yaml {
        for (label, entry) in &yaml.tasks {
            if entry.patch.enabled == Some(false) {
                tasks.remove(label);
                continue;
            }

            let task = tasks
                .entry(label.clone())
                .or_insert_with(|| default_task_input(label));
            apply_override(task, &entry.patch);
            validate_task_input(task)
                .map_err(|error| anyhow::anyhow!("invalid task '{label}': {error}"))?;
        }
    }
    Ok(tasks)
}

fn resolve_task_order(
    configured: &[String],
    source_order: Vec<String>,
    tasks: &BTreeMap<String, EditableTaskInput>,
) -> Result<Vec<String>> {
    let mut seen = HashSet::new();
    let mut order = Vec::with_capacity(tasks.len());
    for label in configured {
        if !seen.insert(label.clone()) {
            bail!("task_order contains duplicate task '{label}'");
        }
        if !tasks.contains_key(label) {
            bail!("task_order references unknown or disabled task '{label}'");
        }
        order.push(label.clone());
    }
    for label in source_order.into_iter().chain(tasks.keys().cloned()) {
        if tasks.contains_key(&label) && seen.insert(label.clone()) {
            order.push(label);
        }
    }
    Ok(order)
}

fn apply_override(task: &mut EditableTaskInput, patch: &TaskOverride) {
    if let Some(command) = &patch.command {
        task.command = command.clone();
    }
    if let Some(args) = &patch.args {
        task.args = args.clone();
    }
    if let Some(cwd) = &patch.cwd {
        task.cwd = cwd.clone();
    }
    if let Some(env) = &patch.env {
        for (key, value) in env {
            match value {
                Some(value) => {
                    task.env.insert(key.clone(), value.clone());
                }
                None => {
                    task.env.remove(key);
                }
            }
        }
    }
    if let Some(shell) = patch.shell {
        task.shell = shell;
    }
    if let Some(auto_start) = patch.auto_start {
        task.auto_start = auto_start;
    }
    if let Some(timeout) = patch.stop_timeout_ms {
        task.stop_timeout_ms = timeout;
    }
    if let Some(clear) = patch.clear_logs_on_restart {
        task.clear_logs_on_restart = clear;
    }
    if let Some(schedule) = &patch.schedule {
        task.schedule = Some(schedule.clone());
    }
}

fn compile_task(
    project: &Path,
    task: &EditableTaskInput,
    workspace_env: &BTreeMap<String, String>,
) -> Result<TaskSpec> {
    validate_task_input(task).map_err(anyhow::Error::msg)?;
    if let Some(schedule) = &task.schedule {
        validate_cron_expression(schedule)?;
    }
    let expanded_cwd = expand(&task.cwd, project);
    let cwd = PathBuf::from(&expanded_cwd);
    Ok(TaskSpec {
        label: task.label.clone(),
        program: expand(&task.command, project),
        args: task.args.iter().map(|arg| expand(arg, project)).collect(),
        cwd: if cwd.is_absolute() {
            cwd
        } else {
            project.join(cwd)
        },
        env: {
            let mut merged = workspace_env.clone();
            for (key, value) in task.env.iter() {
                merged.insert(key.clone(), expand(value, project));
            }
            let values = merged.clone();
            values
                .into_iter()
                .map(|(key, value)| (key, expand_with_overrides(&value, project, &merged)))
                .collect::<BTreeMap<_, _>>()
        },
        shell: task.shell,
        auto_start: task.auto_start,
        stop_timeout_ms: task.stop_timeout_ms,
        clear_logs_on_restart: task.clear_logs_on_restart,
        schedule: task.schedule.clone(),
    })
}

fn validate_submitted_tasks(
    tasks: Vec<EditableTaskInput>,
) -> std::result::Result<(BTreeMap<String, EditableTaskInput>, Vec<String>), WriteConfigError> {
    let mut labels = HashSet::new();
    let mut submitted = BTreeMap::new();
    let mut order = Vec::new();
    for task in tasks {
        validate_task_input(&task).map_err(WriteConfigError::validation)?;
        if !labels.insert(task.label.clone()) {
            return Err(WriteConfigError::validation(format!(
                "duplicate task label '{}'",
                task.label
            )));
        }
        order.push(task.label.clone());
        submitted.insert(task.label.clone(), task);
    }
    Ok((submitted, order))
}

fn validate_workspace_env(values: &BTreeMap<String, String>) -> std::result::Result<(), String> {
    if values.keys().any(|key| key.trim().is_empty()) {
        return Err("workspace environment keys must not be empty".to_string());
    }
    Ok(())
}

fn validate_task_input(task: &EditableTaskInput) -> std::result::Result<(), String> {
    if task.label.trim().is_empty() {
        return Err("task label must not be empty".to_string());
    }
    if task.command.trim().is_empty() {
        return Err(format!("task '{}' command must not be empty", task.label));
    }
    if task.cwd.trim().is_empty() {
        return Err(format!("task '{}' cwd must not be empty", task.label));
    }
    if task.stop_timeout_ms == 0 {
        return Err(format!(
            "task '{}' stop_timeout_ms must be between 1 and {MAX_STOP_TIMEOUT_MS}",
            task.label,
        ));
    }
    if task.stop_timeout_ms > MAX_STOP_TIMEOUT_MS {
        return Err(format!(
            "task '{}' stop_timeout_ms must be between 1 and {MAX_STOP_TIMEOUT_MS}",
            task.label,
        ));
    }
    if task.env.keys().any(|key| key.trim().is_empty()) {
        return Err(format!("task '{}' env keys must not be empty", task.label));
    }
    if let Some(schedule) = &task.schedule {
        if crate::state::validate_cron_expression(schedule).is_err() {
            return Err(format!(
                "task '{}' has an invalid cron expression '{schedule}'",
                task.label
            ));
        }
    }
    Ok(())
}

fn build_imported_task_mapping(
    base: &EditableTaskInput,
    submitted: &EditableTaskInput,
    mut raw: Mapping,
) -> Mapping {
    clear_known_task_fields(&mut raw);
    if submitted.command != base.command {
        raw.insert(yaml_key("command"), yaml_string(&submitted.command));
    }
    if submitted.args != base.args {
        raw.insert(yaml_key("args"), yaml_string_list(&submitted.args));
    }
    if submitted.cwd != base.cwd {
        raw.insert(yaml_key("cwd"), yaml_string(&submitted.cwd));
    }
    let env_diff = build_imported_env_diff(base, submitted);
    if !env_diff.is_empty() {
        raw.insert(yaml_key("env"), yaml_optional_string_map(&env_diff));
    }
    if submitted.shell != base.shell {
        raw.insert(yaml_key("shell"), Value::Bool(submitted.shell));
    }
    if submitted.auto_start != base.auto_start {
        raw.insert(yaml_key("auto_start"), Value::Bool(submitted.auto_start));
    }
    if submitted.stop_timeout_ms != base.stop_timeout_ms {
        raw.insert(
            yaml_key("stop_timeout_ms"),
            Value::from(submitted.stop_timeout_ms),
        );
    }
    if submitted.schedule != base.schedule {
        match &submitted.schedule {
            Some(schedule) => raw.insert(yaml_key("schedule"), yaml_string(schedule)),
            None => raw.insert(yaml_key("schedule"), Value::Null),
        };
    }
    if submitted.clear_logs_on_restart != base.clear_logs_on_restart {
        raw.insert(
            yaml_key("clear_logs_on_restart"),
            Value::Bool(submitted.clear_logs_on_restart),
        );
    }
    raw
}

fn build_yaml_task_mapping(submitted: &EditableTaskInput, mut raw: Mapping) -> Mapping {
    clear_known_task_fields(&mut raw);
    raw.insert(yaml_key("command"), yaml_string(&submitted.command));
    raw.insert(yaml_key("args"), yaml_string_list(&submitted.args));
    raw.insert(yaml_key("cwd"), yaml_string(&submitted.cwd));
    raw.insert(yaml_key("env"), yaml_string_map(&submitted.env));
    raw.insert(yaml_key("shell"), Value::Bool(submitted.shell));
    raw.insert(yaml_key("auto_start"), Value::Bool(submitted.auto_start));
    raw.insert(
        yaml_key("stop_timeout_ms"),
        Value::from(submitted.stop_timeout_ms),
    );
    raw.insert(
        yaml_key("clear_logs_on_restart"),
        Value::Bool(submitted.clear_logs_on_restart),
    );
    if let Some(schedule) = &submitted.schedule {
        raw.insert(yaml_key("schedule"), yaml_string(schedule));
    }
    raw
}

fn build_disabled_import_mapping(mut raw: Mapping) -> Mapping {
    clear_known_task_fields(&mut raw);
    raw.insert(yaml_key("enabled"), Value::Bool(false));
    raw
}

fn clear_known_task_fields(mapping: &mut Mapping) {
    for key in [
        "enabled",
        "command",
        "args",
        "cwd",
        "env",
        "shell",
        "auto_start",
        "stop_timeout_ms",
        "clear_logs_on_restart",
        "schedule",
    ] {
        mapping.remove(yaml_key(key));
    }
}

fn default_task_input(label: &str) -> EditableTaskInput {
    EditableTaskInput {
        label: label.to_string(),
        command: String::new(),
        args: Vec::new(),
        cwd: ".".to_string(),
        env: BTreeMap::new(),
        shell: true,
        auto_start: false,
        stop_timeout_ms: DEFAULT_STOP_TIMEOUT_MS,
        clear_logs_on_restart: false,
        schedule: None,
    }
}

fn write_temp_config_file(path: &Path, content: &str) -> Result<PathBuf> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = parent.join(format!(".{}.{}.tmp", PROJECT_CONFIG, nonce));
    let result = (|| -> Result<PathBuf> {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);

        let mut file = options
            .open(&temp)
            .with_context(|| format!("failed to create {}", temp.display()))?;
        file.write_all(content.as_bytes())
            .with_context(|| format!("failed to write {}", temp.display()))?;
        if let Some(permissions) = target_permissions(path)
            .with_context(|| format!("failed to determine permissions for {}", path.display()))?
        {
            fs::set_permissions(&temp, permissions)
                .with_context(|| format!("failed to apply permissions to {}", temp.display()))?;
        }
        file.sync_all()
            .with_context(|| format!("failed to sync {}", temp.display()))?;
        Ok(temp.clone())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn target_permissions(path: &Path) -> Result<Option<fs::Permissions>> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(Some(metadata.permissions())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn sync_parent_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .with_context(|| format!("{} has no parent directory", path.display()))?;
        File::open(parent)
            .with_context(|| format!("failed to open {}", parent.display()))?
            .sync_all()
            .with_context(|| format!("failed to sync {}", parent.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn revision_for_project(project: &Path) -> Result<String> {
    load_project_state(project).map(|state| state.revision())
}

#[cfg(unix)]
fn rename_temp_file(temp: &Path, path: &Path) -> Result<()> {
    match fs::rename(temp, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(temp);
            Err(error).with_context(|| {
                format!(
                    "failed to replace {} with {}",
                    path.display(),
                    temp.display()
                )
            })
        }
    }
}

#[cfg(windows)]
fn rename_temp_file(temp: &Path, path: &Path) -> Result<()> {
    let temp_wide = temp
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let path_wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let replaced = unsafe {
        MoveFileExW(
            temp_wide.as_ptr(),
            path_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced != 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    let _ = fs::remove_file(temp);
    Err(error).with_context(|| {
        format!(
            "failed to replace {} with {}",
            path.display(),
            temp.display()
        )
    })
}

fn yaml_key(key: &str) -> Value {
    Value::String(key.to_string())
}

fn yaml_string(value: &str) -> Value {
    Value::String(value.to_string())
}

fn yaml_string_list(values: &[String]) -> Value {
    Value::Sequence(values.iter().map(|value| yaml_string(value)).collect())
}

fn yaml_string_map(values: &BTreeMap<String, String>) -> Value {
    let mut mapping = Mapping::new();
    for (key, value) in values {
        mapping.insert(yaml_key(key), yaml_string(value));
    }
    Value::Mapping(mapping)
}

fn yaml_optional_string_map(values: &BTreeMap<String, Option<String>>) -> Value {
    let mut mapping = Mapping::new();
    for (key, value) in values {
        mapping.insert(
            yaml_key(key),
            match value {
                Some(value) => yaml_string(value),
                None => Value::Null,
            },
        );
    }
    Value::Mapping(mapping)
}

fn build_imported_env_diff(
    base: &EditableTaskInput,
    submitted: &EditableTaskInput,
) -> BTreeMap<String, Option<String>> {
    let mut diff = BTreeMap::new();
    for (key, value) in &submitted.env {
        if base.env.get(key) != Some(value) {
            diff.insert(key.clone(), Some(value.clone()));
        }
    }
    for key in base.env.keys() {
        if !submitted.env.contains_key(key) {
            diff.insert(key.clone(), None);
        }
    }
    diff
}

impl ProjectConfigState {
    fn revision(&self) -> String {
        let mut hasher = Fnv64::default();
        hasher.write(self.project.to_string_lossy().as_bytes());
        hasher.write(&[0]);
        if let Some(content) = &self.vscode_raw_content {
            hasher.write(content.as_bytes());
        }
        hasher.write(&[0xff]);
        if let Some(yaml) = &self.yaml {
            hasher.write(yaml.raw_content.as_bytes());
        }
        format!("{:016x}", hasher.finish())
    }
}

#[derive(Default)]
struct Fnv64(u64);

impl Fnv64 {
    fn write(&mut self, bytes: &[u8]) {
        if self.0 == 0 {
            self.0 = 0xcbf29ce484222325;
        }
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }

    fn finish(self) -> u64 {
        self.0
    }
}

fn expand(input: &str, project: &Path) -> String {
    expand_with_overrides(input, project, &BTreeMap::new())
}

fn expand_with_overrides(
    input: &str,
    project: &Path,
    overrides: &BTreeMap<String, String>,
) -> String {
    let basename = project
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let mut output = input
        .replace("${workspaceFolderBasename}", basename)
        .replace("${workspaceFolder}", &project.to_string_lossy());
    let mut environment: HashMap<String, String> = env::vars().collect();
    for (key, value) in overrides {
        environment.insert(key.clone(), value.clone());
    }
    for (key, value) in environment {
        output = output.replace(&format!("${{env:{key}}}"), &value);
    }
    output
}

pub fn validate_session_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        bail!("session name must be 1-64 ASCII letters, digits, '.', '-' or '_'");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::protocol::EditableTaskInput;

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

        let error =
            write_session_config(dir.path(), &snapshot.revision, snapshot.tasks_to_inputs())
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
        let error =
            write_session_config(dir.path(), &snapshot.revision, snapshot.tasks_to_inputs())
                .unwrap_err();
        assert!(matches!(error, WriteConfigError::Validation { .. }));
        snapshot = read_session_config(dir.path(), "demo").unwrap();
        snapshot.tasks[0].schedule = None;
        write_session_config(dir.path(), &snapshot.revision, snapshot.tasks_to_inputs()).unwrap();
        let saved = fs::read_to_string(dir.path().join(PROJECT_CONFIG)).unwrap();
        assert!(saved.contains("APP_ENV"));
        assert!(!saved.contains("*/10"));
    }
}
