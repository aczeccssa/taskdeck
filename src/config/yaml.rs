//! taskdeck.yaml parsing and mapping helpers.

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
#[derive(Debug, Default, Clone, Deserialize)]

pub(crate) struct YamlConfig {

    #[serde(default = "default_version")]

    pub(crate) version: u32,

    pub(crate) session: Option<String>,

    #[serde(default)]

    pub(crate) workspace_env: BTreeMap<String, String>,

    #[serde(default)]

    pub(crate) task_order: Vec<String>,

}



#[derive(Debug, Default, Clone, Deserialize, Serialize)]

#[serde(default)]

pub(crate) struct TaskOverride {

    pub(crate) enabled: Option<bool>,

    pub(crate) command: Option<String>,

    pub(crate) args: Option<Vec<String>>,

    pub(crate) cwd: Option<String>,

    pub(crate) env: Option<BTreeMap<String, Option<String>>>,

    pub(crate) shell: Option<bool>,

    pub(crate) auto_start: Option<bool>,

    pub(crate) stop_timeout_ms: Option<u64>,

    pub(crate) clear_logs_on_restart: Option<bool>,

    pub(crate) schedule: Option<String>,

}



#[derive(Debug, Clone)]

pub(crate) struct TaskYamlEntry {

    pub(crate) raw: Mapping,

    pub(crate) patch: TaskOverride,

}



#[derive(Debug, Clone)]

pub(crate) struct YamlDocument {

    pub(crate) workspace_env: BTreeMap<String, String>,

    pub(crate) root: Mapping,

    pub(crate) tasks: BTreeMap<String, TaskYamlEntry>,

    pub(crate) declared_order: Vec<String>,

    pub(crate) task_order: Vec<String>,

    pub(crate) session: Option<String>,

    pub(crate) raw_content: String,

}




pub(crate) fn load_yaml_document(project: &Path) -> Result<Option<YamlDocument>> {

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



pub(crate) fn build_yaml_task_mapping(submitted: &EditableTaskInput, mut raw: Mapping) -> Mapping {

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



pub(crate) fn build_disabled_import_mapping(mut raw: Mapping) -> Mapping {

    clear_known_task_fields(&mut raw);

    raw.insert(yaml_key("enabled"), Value::Bool(false));

    raw

}



pub(crate) fn clear_known_task_fields(mapping: &mut Mapping) {

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

pub(crate) fn yaml_key(key: &str) -> Value {

    Value::String(key.to_string())

}



pub(crate) fn yaml_string(value: &str) -> Value {

    Value::String(value.to_string())

}



pub(crate) fn yaml_string_list(values: &[String]) -> Value {

    Value::Sequence(values.iter().map(|value| yaml_string(value)).collect())

}



pub(crate) fn yaml_string_map(values: &BTreeMap<String, String>) -> Value {

    let mut mapping = Mapping::new();

    for (key, value) in values {

        mapping.insert(yaml_key(key), yaml_string(value));

    }

    Value::Mapping(mapping)

}



pub(crate) fn yaml_optional_string_map(values: &BTreeMap<String, Option<String>>) -> Value {

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

