//! Session config read/write/prepare API.

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


use super::*;
use super::merge::*;
use super::vscode::*;
use super::write::*;
use super::yaml::*;
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



pub(crate) fn prepare_session_config_write_inner(

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



pub(crate) fn load_project_state(project: &Path) -> Result<ProjectConfigState> {

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



pub(crate) type LoadedVscodeTasks = (

    BTreeMap<String, EditableTaskInput>,

    Vec<String>,

    Option<String>,

);

