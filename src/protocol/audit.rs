//! Audit source / status / transport / context and audit record types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::base::{Request, Response};
use super::util::{parse_history_page_size, parse_positive_usize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditSource {
    Cli,
    Tui,
    Web,
    Mcp,
    Scheduler,
    Internal,
}

impl AuditSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Tui => "tui",
            Self::Web => "web",
            Self::Mcp => "mcp",
            Self::Scheduler => "scheduler",
            Self::Internal => "internal",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().replace('-', "_").to_ascii_lowercase();
        match normalized.as_str() {
            "cli" => Some(Self::Cli),
            "tui" => Some(Self::Tui),
            "web" => Some(Self::Web),
            "mcp" => Some(Self::Mcp),
            "scheduler" => Some(Self::Scheduler),
            "internal" => Some(Self::Internal),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditStatus {
    Started,
    Success,
    Error,
    Timeout,
}

impl AuditStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Success => "success",
            Self::Error => "error",
            Self::Timeout => "timeout",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().replace('-', "_").to_ascii_lowercase();
        match normalized.as_str() {
            "started" => Some(Self::Started),
            "success" => Some(Self::Success),
            "error" => Some(Self::Error),
            "timeout" => Some(Self::Timeout),
            _ => None,
        }
    }

    pub fn from_ok(ok: bool) -> Self {
        if ok { Self::Success } else { Self::Error }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditTransport {
    Ipc,
    Http,
    Mcp,
    Agent,
    Internal,
}

impl AuditTransport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ipc => "ipc",
            Self::Http => "http",
            Self::Mcp => "mcp",
            Self::Agent => "agent",
            Self::Internal => "internal",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().replace('-', "_").to_ascii_lowercase();
        match normalized.as_str() {
            "ipc" => Some(Self::Ipc),
            "http" => Some(Self::Http),
            "mcp" => Some(Self::Mcp),
            "agent" => Some(Self::Agent),
            "internal" => Some(Self::Internal),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditContext {
    pub correlation_id: String,
    pub source: AuditSource,
    pub transport: AuditTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_audit_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

impl AuditContext {
    pub fn new(source: AuditSource, transport: AuditTransport) -> Self {
        Self {
            correlation_id: uuid::Uuid::new_v4().to_string(),
            source,
            transport,
            origin_node_id: None,
            origin_audit_id: None,
            session: None,
            task: None,
            action: None,
        }
    }

    pub fn with_origin_node(mut self, node_id: impl Into<String>) -> Self {
        self.origin_node_id = Some(node_id.into());
        self
    }

    pub fn with_request_defaults(mut self, request: &Request) -> Self {
        if self.session.is_none() {
            self.session = request.session().map(str::to_string);
        }
        if self.task.is_none() {
            self.task = request.task().map(str::to_string);
        }
        if self.action.is_none() {
            self.action = Some(request.operation());
        }
        self
    }

    pub fn internal() -> Self {
        Self::new(AuditSource::Internal, AuditTransport::Internal)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditRecord {
    pub audit_id: String,
    pub correlation_id: String,
    pub timestamp_ms: u64,
    pub duration_ms: u64,
    pub source: AuditSource,
    pub transport: AuditTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor_node_id: Option<String>,
    pub request_kind: String,
    pub operation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub status: AuditStatus,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub request: Value,
    #[serde(default)]
    pub response: Value,
    #[serde(default)]
    pub details: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replicated_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditListItem {
    pub audit_id: String,
    pub correlation_id: String,
    pub timestamp_ms: u64,
    pub duration_ms: u64,
    pub source: AuditSource,
    pub transport: AuditTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor_node_id: Option<String>,
    pub request_kind: String,
    pub operation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub status: AuditStatus,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replicated_at_ms: Option<u64>,
}

impl From<&AuditRecord> for AuditListItem {
    fn from(record: &AuditRecord) -> Self {
        Self {
            audit_id: record.audit_id.clone(),
            correlation_id: record.correlation_id.clone(),
            timestamp_ms: record.timestamp_ms,
            duration_ms: record.duration_ms,
            source: record.source,
            transport: record.transport,
            origin_node_id: record.origin_node_id.clone(),
            executor_node_id: record.executor_node_id.clone(),
            request_kind: record.request_kind.clone(),
            operation: record.operation.clone(),
            session: record.session.clone(),
            task: record.task.clone(),
            status: record.status,
            success: record.success,
            error: record.error.clone(),
            replicated_at_ms: record.replicated_at_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditFilter {
    pub q: Option<String>,
    pub source: Option<String>,
    pub status: Option<String>,
    pub node: Option<String>,
    pub session: Option<String>,
    pub task: Option<String>,
    pub operation: Option<String>,
    pub page: usize,
    pub page_size: usize,
}

impl AuditFilter {
    pub fn parse(
        query: &std::collections::HashMap<String, String>,
    ) -> std::result::Result<Self, Response> {
        let optional = |key: &str| {
            query
                .get(key)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        let source = match optional("source").as_deref() {
            None | Some("all") => None,
            Some(value) => Some(
                AuditSource::parse(value)
                    .ok_or_else(|| {
                        Response::error_with_data(
                            "invalid source",
                            serde_json::json!({"kind": "validation_error", "status": 400}),
                        )
                    })?
                    .as_str()
                    .to_string(),
            ),
        };
        let status = match optional("status").as_deref() {
            None | Some("all") => None,
            Some(value) => Some(
                AuditStatus::parse(value)
                    .ok_or_else(|| {
                        Response::error_with_data(
                            "invalid status",
                            serde_json::json!({"kind": "validation_error", "status": 400}),
                        )
                    })?
                    .as_str()
                    .to_string(),
            ),
        };
        Ok(Self {
            q: optional("q"),
            source,
            status,
            node: optional("node"),
            session: optional("session"),
            task: optional("task"),
            operation: optional("operation"),
            page: parse_positive_usize(query, "page", 1)?,
            page_size: parse_history_page_size(query)?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditListPage {
    pub items: Vec<AuditListItem>,
    pub page: usize,
    pub page_size: usize,
    pub total: usize,
    pub total_pages: usize,
    pub has_next: bool,
    pub has_previous: bool,
}
