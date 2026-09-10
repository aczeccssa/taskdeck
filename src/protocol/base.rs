//! Core protocol: task status, actions, logs and the request/response envelope.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::audit::AuditContext;
use super::auth::NodeSettingsPatch;
use super::editable_task::EditableTaskInput;
use super::history::{EventFilter, TaskRunFilter};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Idle,
    Running,
    Paused,
    Exited,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Start,
    Stop,
    Restart,
    Pause,
    Resume,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Pause => "pause",
            Self::Resume => "resume",
        }
    }
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
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

impl Request {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Ping => "ping",
            Self::ListWorkspaces => "list_workspaces",
            Self::SetWorkspaceAlias { .. } => "set_workspace_alias",
            Self::GetNodeSettings => "get_node_settings",
            Self::PutNodeSettings { .. } => "put_node_settings",
            Self::ListTaskRuns { .. } => "list_task_runs",
            Self::ListEvents { .. } => "list_events",
            Self::Register { .. } => "register",
            Self::Update { .. } => "update",
            Self::ListSessions => "list_sessions",
            Self::Snapshot { .. } => "snapshot",
            Self::TaskLogs { .. } => "task_logs",
            Self::TaskMetrics { .. } => "task_metrics",
            Self::ClearTaskHistory { .. } => "clear_task_history",
            Self::GetSessionConfig { .. } => "get_session_config",
            Self::PutSessionConfig { .. } => "put_session_config",
            Self::Action { .. } => "action",
            Self::RemoveSession { .. } => "remove_session",
            Self::Shutdown => "shutdown",
        }
    }

    pub fn operation(&self) -> String {
        match self {
            Self::Action { action, .. } => action.as_str().to_string(),
            other => other.kind().to_string(),
        }
    }

    pub fn session(&self) -> Option<&str> {
        match self {
            Self::Register { session, .. } | Self::Update { session, .. } => session.as_deref(),
            Self::Snapshot { session, .. }
            | Self::TaskLogs { session, .. }
            | Self::TaskMetrics { session, .. }
            | Self::ClearTaskHistory { session, .. }
            | Self::GetSessionConfig { session, .. }
            | Self::PutSessionConfig { session, .. }
            | Self::Action { session, .. }
            | Self::RemoveSession { session }
            | Self::SetWorkspaceAlias { session, .. } => Some(session.as_str()),
            Self::ListTaskRuns { filter } => filter.session.as_deref(),
            _ => None,
        }
    }

    pub fn task(&self) -> Option<&str> {
        match self {
            Self::TaskLogs { task, .. }
            | Self::TaskMetrics { task, .. }
            | Self::ClearTaskHistory { task, .. } => Some(task.as_str()),
            Self::Action { task, .. } => task.as_deref(),
            Self::ListTaskRuns { filter } => filter.task.as_deref(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Ping,
    ListTaskRuns {
        filter: TaskRunFilter,
    },
    ListEvents {
        filter: EventFilter,
    },
    Register {
        project: PathBuf,
        session: Option<String>,
    },
    Update {
        project: PathBuf,
        session: Option<String>,
    },
    ListSessions,
    ListWorkspaces,
    SetWorkspaceAlias {
        session: String,
        #[serde(default)]
        alias: Option<String>,
    },
    GetNodeSettings,
    PutNodeSettings {
        patch: NodeSettingsPatch,
    },
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
        #[serde(default)]
        workspace_env: Option<BTreeMap<String, String>>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub request: Request,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit: Option<AuditContext>,
}

impl Envelope {
    pub fn new(request: Request, audit: AuditContext) -> Self {
        Self {
            request,
            audit: Some(audit),
        }
    }

    pub fn parse_line(line: &str) -> Result<Self, String> {
        if let Ok(envelope) = serde_json::from_str::<Envelope>(line) {
            return Ok(envelope);
        }
        match serde_json::from_str::<Request>(line) {
            Ok(request) => Ok(Self {
                request,
                audit: None,
            }),
            Err(error) => Err(error.to_string()),
        }
    }
}
