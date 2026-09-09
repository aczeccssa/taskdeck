//! Task merge/override/compile/validation.

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

use super::vscode::*;
use super::write::*;
use super::yaml::*;
use super::*;
use super::{expand, expand_with_overrides};
pub(crate) fn merge_tasks(
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

pub(crate) fn resolve_task_order(
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

pub(crate) fn apply_override(task: &mut EditableTaskInput, patch: &TaskOverride) {
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

pub(crate) fn compile_task(
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

pub(crate) fn validate_submitted_tasks(
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

pub(crate) fn validate_workspace_env(
    values: &BTreeMap<String, String>,
) -> std::result::Result<(), String> {
    if values.keys().any(|key| key.trim().is_empty()) {
        return Err("workspace environment keys must not be empty".to_string());
    }

    Ok(())
}

pub(crate) fn validate_task_input(task: &EditableTaskInput) -> std::result::Result<(), String> {
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

pub(crate) fn default_task_input(label: &str) -> EditableTaskInput {
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

pub(crate) fn build_imported_env_diff(
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

pub(crate) fn build_imported_task_mapping(
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
