//! VSCode tasks.json import.

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
use super::write::*;
use super::yaml::*;
use super::session_config::LoadedVscodeTasks;
#[derive(Debug, Default, Deserialize)]

pub(crate) struct VscodeFile {

    #[serde(default)]

    pub(crate) tasks: Vec<VscodeTask>,

}



#[derive(Debug, Default, Deserialize)]

#[serde(rename_all = "camelCase")]

pub(crate) struct VscodeTask {

    pub(crate) label: String,

    #[serde(rename = "type", default)]

    pub(crate) kind: String,

    pub(crate) command: String,

    #[serde(default)]

    pub(crate) args: Vec<JsonArg>,

    #[serde(default)]

    pub(crate) options: VscodeOptions,

}



#[derive(Debug, Clone, Deserialize)]

#[serde(untagged)]

pub(crate) enum JsonArg {

    Text(String),

    Number(serde_json::Number),

    Bool(bool),

}



impl JsonArg {

pub(crate)     fn render(&self) -> String {

        match self {

            Self::Text(value) => value.clone(),

            Self::Number(value) => value.to_string(),

            Self::Bool(value) => value.to_string(),

        }

    }

}



#[derive(Debug, Default, Deserialize)]

pub(crate) struct VscodeOptions {

    pub(crate) cwd: Option<String>,

    #[serde(default)]

    pub(crate) env: BTreeMap<String, String>,

}




pub(crate) fn load_vscode_tasks(project: &Path) -> Result<LoadedVscodeTasks> {

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

