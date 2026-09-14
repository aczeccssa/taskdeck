//! Project configuration discovery and editing.

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

struct ProjectConfigState {
    pub(crate) project: PathBuf,
    pub(crate) source: String,
    pub(crate) vscode_raw_content: Option<String>,
    pub(crate) vscode_tasks: BTreeMap<String, EditableTaskInput>,
    pub(crate) merged_tasks: BTreeMap<String, EditableTaskInput>,
    pub(crate) workspace_env: BTreeMap<String, String>,
    pub(crate) task_order: Vec<String>,
    pub(crate) yaml: Option<YamlDocument>,
}

mod merge;
mod session_config;
mod vscode;
mod write;
mod yaml;

pub(crate) use merge::*;
pub(crate) use session_config::*;
pub(crate) use vscode::*;
pub(crate) use write::*;
use yaml::*;

pub(crate) fn default_version() -> u32 {
    1
}

pub fn discover(project: &Path, requested_session: Option<&str>) -> Result<ProjectDefinition> {
    discover_inner(project, requested_session, false)
}

#[derive(Debug, Clone)]
pub(crate) struct InitializedProject {
    pub project: PathBuf,
    pub session: String,
    pub config_path: PathBuf,
}

/// Create a new taskdeck.yaml from the project's VS Code tasks, or an empty
/// taskdeck project when no VS Code task file exists.
pub(crate) fn init_project(
    project: &Path,
    requested_session: Option<&str>,
) -> Result<InitializedProject> {
    let project = project
        .canonicalize()
        .with_context(|| format!("project directory does not exist: {}", project.display()))?;
    let config_path = project.join(PROJECT_CONFIG);
    if config_path.exists() {
        bail!(
            "{} already exists; refusing to overwrite",
            config_path.display()
        );
    }

    let (vscode_tasks, vscode_order, _) = load_vscode_tasks(&project)?;
    let default_session = project
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project");
    let session = requested_session
        .map(str::to_owned)
        .unwrap_or_else(|| default_session.to_owned());
    validate_session_name(&session)?;

    let mut root = Mapping::new();
    root.insert(yaml_key("version"), Value::from(1u32));
    root.insert(yaml_key("session"), yaml_string(&session));
    root.insert(
        yaml_key("task_order"),
        Value::Sequence(vscode_order.iter().cloned().map(Value::String).collect()),
    );

    let mut task_entries = Mapping::new();
    for label in &vscode_order {
        if let Some(task) = vscode_tasks.get(label) {
            task_entries.insert(
                yaml_key(label),
                Value::Mapping(build_yaml_task_mapping(task, Mapping::new())),
            );
        }
    }
    root.insert(yaml_key("tasks"), Value::Mapping(task_entries));

    let serialized = serde_yaml::to_string(&Value::Mapping(root))
        .context("failed to serialize initialized taskdeck.yaml")?;
    let temp_path = write_temp_config_file(&config_path, &serialized)?;
    if let Err(error) = rename_temp_file(&temp_path, &config_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    sync_parent_directory(&config_path)?;

    Ok(InitializedProject {
        project,
        session,
        config_path,
    })
}

pub(crate) fn discover_inner(
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

impl ProjectConfigState {
    pub(crate) fn revision(&self) -> String {
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
    pub(crate) fn write(&mut self, bytes: &[u8]) {
        if self.0 == 0 {
            self.0 = 0xcbf29ce484222325;
        }
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }

    pub(crate) fn finish(self) -> u64 {
        self.0
    }
}

pub(crate) fn expand(input: &str, project: &Path) -> String {
    expand_with_overrides(input, project, &BTreeMap::new())
}

pub(crate) fn expand_with_overrides(
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
mod tests;
