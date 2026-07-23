use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_casefold::UnicodeCaseFold;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Idle,
    Running,
    Paused,
    Exited,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Start,
    Stop,
    Restart,
    Pause,
    Resume,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogLine {
    pub seq: u64,
    pub stream: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskLogsSnapshot {
    pub generation: u64,
    pub reset: bool,
    pub lines: Vec<LogLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ServiceClassification {
    Service,
    Process,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ServiceConfidence {
    High,
    Medium,
    Low,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TechnologyProfile {
    pub runtime: Option<String>,
    pub framework: Option<String>,
    pub confidence: ServiceConfidence,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceEndpoint {
    pub bind_host: String,
    pub port: u16,
    pub protocol: String,
    pub pid: Option<u32>,
    pub source: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ServiceInspectionState {
    Listening,
    NoListener,
    NotRunning,
    Unsupported,
    #[default]
    Pending,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ServiceObservation {
    pub classification: ServiceClassification,
    pub technology: TechnologyProfile,
    pub endpoints: Vec<ServiceEndpoint>,
    pub inspection: ServiceInspectionState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub label: String,
    pub status: TaskStatus,
    pub pid: Option<u32>,
    pub command: String,
    pub cwd: PathBuf,
    pub auto_start: bool,
    pub last_exit: Option<String>,
    pub logs: Vec<LogLine>,
    #[serde(default)]
    pub run_generation: u64,
    #[serde(default)]
    pub service: ServiceObservation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub name: String,
    pub project: PathBuf,
    pub source: String,
    pub tasks: BTreeMap<String, TaskSnapshot>,
    #[serde(default)]
    pub task_order: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeSummary {
    pub id: String,
    pub name: String,
    pub role: String,
    pub mode: String,
    pub online: bool,
    pub is_self: bool,
    pub last_seen_ms: Option<u64>,
    pub sessions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TaskMetricsAggregate {
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub process_count: u32,
}

impl TaskMetricsAggregate {
    pub fn zero() -> Self {
        Self {
            cpu_percent: 0.0,
            memory_bytes: 0,
            process_count: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskMetricsSample {
    pub timestamp_ms: u64,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub process_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskProcessSnapshot {
    pub pid: u32,
    pub ppid: Option<u32>,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub status: String,
    pub run_time_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskMetricsSnapshot {
    pub sample_interval_ms: u64,
    pub window_seconds: u64,
    pub cpu_percent_unit: String,
    pub running: bool,
    pub current: TaskMetricsAggregate,
    pub samples: Vec<TaskMetricsSample>,
    pub processes: Vec<TaskProcessSnapshot>,
    #[serde(default)]
    pub restart_markers_ms: Vec<u64>,
}

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
    pub auto_start: bool,
    pub stop_timeout_ms: u64,
    #[serde(default)]
    pub clear_logs_on_restart: bool,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionConfigSnapshot {
    pub session: String,
    pub project: PathBuf,
    pub source: String,
    pub revision: String,
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
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCallRecord {
    pub id: u64,
    pub tool: String,
    pub operation: Option<String>,
    pub started_at_ms: u64,
    pub duration_ms: u64,
    pub success: bool,
    #[serde(default)]
    pub target_node: Option<String>,
    pub request: Value,
    pub response: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCallListItem {
    pub id: u64,
    pub tool: String,
    pub operation: Option<String>,
    pub started_at_ms: u64,
    pub duration_ms: u64,
    pub success: bool,
    #[serde(default)]
    pub target_node: Option<String>,
    pub input: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCallListPage {
    pub items: Vec<McpCallListItem>,
    pub page: usize,
    pub page_size: usize,
    pub total: usize,
    pub total_pages: usize,
    pub has_next: bool,
    pub has_previous: bool,
}

pub fn casefold_search_text(value: &str) -> String {
    value.case_fold().collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Ping,
    Register {
        project: PathBuf,
        session: Option<String>,
    },
    Update {
        project: PathBuf,
        session: Option<String>,
    },
    ListSessions,
    Snapshot {
        session: String,
        tail: Option<usize>,
    },
    TaskLogs {
        session: String,
        task: String,
        after: Option<u64>,
        limit: usize,
    },
    TaskMetrics {
        session: String,
        task: String,
        window_seconds: usize,
    },
    ClearTaskHistory {
        session: String,
        task: String,
    },
    GetSessionConfig {
        session: String,
    },
    PutSessionConfig {
        session: String,
        revision: String,
        tasks: Vec<EditableTaskInput>,
    },
    Action {
        session: String,
        task: Option<String>,
        action: Action,
    },
    RemoveSession {
        session: String,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub ok: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl Response {
    pub fn ok(message: impl Into<String>, data: impl Serialize) -> Self {
        Self {
            ok: true,
            message: message.into(),
            data: serde_json::to_value(data).ok(),
        }
    }

    pub fn empty(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
            data: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            data: None,
        }
    }

    pub fn error_with_data(message: impl Into<String>, data: impl Serialize) -> Self {
        Self {
            ok: false,
            message: message.into(),
            data: serde_json::to_value(data).ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::casefold_search_text;

    #[test]
    fn casefold_search_text_matches_expanding_equivalents() {
        assert_eq!(
            casefold_search_text("Straße"),
            casefold_search_text("STRASSE")
        );
    }

    #[test]
    fn casefold_search_text_matches_sigma_and_final_sigma() {
        assert_eq!(casefold_search_text("οσ"), casefold_search_text("ος"));
    }
}
