use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    pub seq: u64,
    pub stream: String,
    pub text: String,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub name: String,
    pub project: PathBuf,
    pub source: String,
    pub tasks: BTreeMap<String, TaskSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCallRecord {
    pub id: u64,
    pub tool: String,
    pub operation: Option<String>,
    pub started_at_ms: u64,
    pub duration_ms: u64,
    pub success: bool,
    pub request: Value,
    pub response: Value,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}
