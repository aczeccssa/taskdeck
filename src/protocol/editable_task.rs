//! Editable task definition and session config snapshot types (config layer).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditableTaskOrigin {
    pub imported: bool,
    pub has_yaml_override: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditableTask {
    pub label: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
    pub shell: bool,
    #[serde(default)]
    pub auto_start: bool,
    pub stop_timeout_ms: u64,
    #[serde(default)]
    pub clear_logs_on_restart: bool,
    #[serde(default)]
    pub schedule: Option<String>,
    pub origin: EditableTaskOrigin,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditableTaskInput {
    pub label: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
    pub shell: bool,
    pub auto_start: bool,
    pub stop_timeout_ms: u64,
    #[serde(default)]
    pub clear_logs_on_restart: bool,
    #[serde(default)]
    pub schedule: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionConfigSnapshot {
    pub session: String,
    pub project: PathBuf,
    pub source: String,
    pub revision: String,
    #[serde(default)]
    pub workspace_env: BTreeMap<String, String>,
    pub tasks: Vec<EditableTask>,
}

impl SessionConfigSnapshot {
    #[cfg(test)]
    pub fn tasks_to_inputs(&self) -> Vec<EditableTaskInput> {
        self.tasks
            .iter()
            .map(|task| EditableTaskInput {
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
            })
            .collect()
    }
}
