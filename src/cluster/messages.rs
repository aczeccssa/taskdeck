//! Cluster wire messages and request types.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use axum::extract::ws::{Message as AxumMessage, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use uuid::Uuid;

use crate::daemon::{DaemonState, dispatch_async_with_audit};
use crate::protocol::NodeMetricsSample;
use crate::protocol::{
    Action, AuditContext, AuditRecord, AuditStatus, EditableTaskInput, EventFilter,
    NodeSettingsPatch, NodeSummary, Request, Response, SessionSnapshot, TaskRunFilter,
};
use crate::state::{NodeSettings, StateStore};

use super::MAX_AGENT_MESSAGE_BYTES;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum AgentMessage {
    Hello {
        protocol: u32,
        node_id: String,
        name: String,
        version: String,
        token: Option<String>,
    },
    Welcome {
        protocol: u32,
    },
    Inventory {
        sessions: Vec<SessionSnapshot>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_metrics: Option<NodeMetricsSample>,
    },
    Heartbeat {
        timestamp_ms: u64,
    },
    Command {
        id: String,
        request: RemoteRequest,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        audit: Option<AuditContext>,
    },
    CommandResult {
        id: String,
        response: Response,
    },
    AuditBatch {
        records: Vec<AuditRecord>,
    },
    AuditAck {
        audit_ids: Vec<String>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RemoteRequest {
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
    ListTaskRuns {
        filter: TaskRunFilter,
    },
    ListEvents {
        filter: EventFilter,
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
        workspace_env: Option<std::collections::BTreeMap<String, String>>,
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
}

impl RemoteRequest {
    pub fn into_local(self) -> Request {
        match self {
            Self::ListSessions => Request::ListSessions,
            Self::ListWorkspaces => Request::ListWorkspaces,
            Self::SetWorkspaceAlias { session, alias } => {
                Request::SetWorkspaceAlias { session, alias }
            }
            Self::GetNodeSettings => Request::GetNodeSettings,
            Self::PutNodeSettings { patch } => Request::PutNodeSettings { patch },
            Self::ListTaskRuns { filter } => Request::ListTaskRuns { filter },
            Self::ListEvents { filter } => Request::ListEvents { filter },
            Self::Snapshot { session, tail } => Request::Snapshot { session, tail },
            Self::TaskLogs {
                session,
                task,
                after,
                limit,
            } => Request::TaskLogs {
                session,
                task,
                after,
                limit,
            },
            Self::TaskMetrics {
                session,
                task,
                window_seconds,
            } => Request::TaskMetrics {
                session,
                task,
                window_seconds,
            },
            Self::ClearTaskHistory { session, task } => Request::ClearTaskHistory { session, task },
            Self::GetSessionConfig { session } => Request::GetSessionConfig { session },
            Self::PutSessionConfig {
                session,
                revision,
                workspace_env,
                tasks,
            } => Request::PutSessionConfig {
                session,
                revision,
                workspace_env,
                tasks,
            },
            Self::Action {
                session,
                task,
                action,
            } => Request::Action {
                session,
                task,
                action,
            },
            Self::RemoveSession { session } => Request::RemoveSession { session },
        }
    }
}

pub(super) fn parse_axum_message(message: AxumMessage) -> Result<AgentMessage> {
    let text = match message {
        AxumMessage::Text(text) => text,
        _ => bail!("agent messages must be text JSON"),
    };
    if text.len() > MAX_AGENT_MESSAGE_BYTES {
        bail!("agent message exceeds size limit");
    }
    serde_json::from_str(&text).context("invalid agent message")
}
pub(super) fn parse_worker_message(message: WsMessage) -> Result<AgentMessage> {
    let text = match message {
        WsMessage::Text(text) => text,
        _ => bail!("leader messages must be text JSON"),
    };
    if text.len() > MAX_AGENT_MESSAGE_BYTES {
        bail!("leader message exceeds size limit");
    }
    serde_json::from_str(&text).context("invalid leader message")
}

pub(super) fn agent_url(leader_url: &str) -> Result<String> {
    let mut url = leader_url.trim_end_matches('/').to_string();
    if let Some(rest) = url.strip_prefix("http://") {
        url = format!("ws://{rest}");
    } else if let Some(rest) = url.strip_prefix("https://") {
        url = format!("wss://{rest}");
    } else if !url.starts_with("ws://") && !url.starts_with("wss://") {
        bail!("leader URL must use http, https, ws, or wss");
    }
    Ok(format!("{url}/api/agent/connect"))
}
