//! Session / task / workspace snapshot types.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::base::{LogLine, TaskStatus};
use super::service_obs::ServiceObservation;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub label: String,
    pub status: TaskStatus,
    pub pid: Option<u32>,
    pub command: String,
    pub cwd: PathBuf,
    pub auto_start: bool,
    pub last_exit: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    pub logs: Vec<LogLine>,
    #[serde(default)]
    pub run_generation: u64,
    #[serde(default)]
    pub started_at_ms: u64,
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub service: ServiceObservation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub name: String,
    pub project: PathBuf,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub tasks: BTreeMap<String, TaskSnapshot>,
    #[serde(default)]
    pub task_order: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceSummary {
    pub session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub display_name: String,
    pub project: PathBuf,
}
