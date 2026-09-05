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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskRunRecord {
    pub id: u64,
    #[serde(default)]
    pub node_id: String,
    pub session: String,
    pub task: String,
    pub trigger: String,
    pub status: String,
    pub started_at_ms: u64,
    #[serde(default)]
    pub finished_at_ms: Option<u64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    pub command: String,
    pub cwd: PathBuf,
    pub pid: Option<u32>,
    #[serde(default)]
    pub run_generation: u64,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRunFilter {
    pub session: Option<String>,
    pub task: Option<String>,
    pub status: Option<String>,
    pub trigger: Option<String>,
    pub page: usize,
    pub page_size: usize,
}

impl TaskRunFilter {
    pub fn parse(
        query: &std::collections::HashMap<String, String>,
    ) -> std::result::Result<Self, Response> {
        let optional = |key: &str| {
            query
                .get(key)
                .map(|value| value.trim())
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        Ok(Self {
            session: optional("session"),
            task: optional("task"),
            status: optional("status"),
            trigger: optional("trigger"),
            page: parse_positive_usize(query, "page", 1)?,
            page_size: parse_history_page_size(query)?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventRecord {
    pub id: u64,
    pub timestamp_ms: u64,
    pub category: String,
    pub message: String,
    #[serde(default)]
    pub details: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventFilter {
    pub category: Option<String>,
    pub page: usize,
    pub page_size: usize,
}

impl EventFilter {
    pub fn parse(
        query: &std::collections::HashMap<String, String>,
    ) -> std::result::Result<Self, Response> {
        let optional = |key: &str| {
            query
                .get(key)
                .map(|value| value.trim())
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        Ok(Self {
            category: optional("category"),
            page: parse_positive_usize(query, "page", 1)?,
            page_size: parse_history_page_size(query)?,
        })
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
pub struct TaskRunListPage {
    pub items: Vec<TaskRunRecord>,
    pub page: usize,
    pub page_size: usize,
    pub total: usize,
    pub total_pages: usize,
    pub has_next: bool,
    pub has_previous: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventListPage {
    pub items: Vec<EventRecord>,
    pub page: usize,
    pub page_size: usize,
    pub total: usize,
    pub total_pages: usize,
    pub has_next: bool,
    pub has_previous: bool,
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

pub fn parse_positive_usize(
    query: &std::collections::HashMap<String, String>,
    key: &str,
    default: usize,
) -> std::result::Result<usize, Response> {
    let value = query
        .get(key)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());
    match value {
        None => Ok(default),
        Some(value) => value
            .parse::<usize>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                Response::error_with_data(
                    format!("invalid {key}"),
                    serde_json::json!({"kind": "validation_error", "status": 400}),
                )
            }),
    }
}

pub fn parse_history_page_size(
    query: &std::collections::HashMap<String, String>,
) -> std::result::Result<usize, Response> {
    const SUPPORTED: [usize; 3] = [20, 50, 100];
    let requested = match query
        .get("page_size")
        .map(|value| value.trim())
        .filter(|v| !v.is_empty())
        .or_else(|| {
            query
                .get("limit")
                .map(|value| value.trim())
                .filter(|v| !v.is_empty())
        }) {
        None => return Ok(20),
        Some(value) => value
            .parse::<usize>()
            .ok()
            .filter(|v| *v > 0)
            .ok_or_else(|| {
                Response::error_with_data(
                    "invalid page_size",
                    serde_json::json!({"kind": "validation_error", "status": 400}),
                )
            })?,
    };
    Ok(*SUPPORTED
        .iter()
        .min_by_key(|size| (requested.abs_diff(**size), **size))
        .expect("supported page sizes"))
}

pub fn casefold_search_text(value: &str) -> String {
    value.case_fold().collect()
}

pub const AUDIT_PAYLOAD_LIMIT_BYTES: usize = 64 * 1024;
pub const REDACTED_VALUE: &str = "[REDACTED]";

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

fn sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .map(|ch| if ch == '-' { '_' } else { ch })
        .collect::<String>()
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "token"
            | "password"
            | "secret"
            | "api_key"
            | "apikey"
            | "authorization"
            | "cookie"
            | "credential"
            | "credentials"
            | "access_key"
            | "accesskey"
            | "private_key"
            | "privatekey"
            | "enrollment_token"
    ) || normalized.ends_with("_token")
        || normalized.ends_with("_secret")
        || normalized.ends_with("_password")
        || normalized.ends_with("_key")
}

pub fn redact_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, nested)| {
                    let redacted = if sensitive_key(key) {
                        Value::String(REDACTED_VALUE.to_string())
                    } else {
                        redact_json(nested)
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact_json).collect()),
        other => other.clone(),
    }
}

pub fn truncate_json(value: Value, limit: usize) -> Value {
    let serialized = serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string());
    if serialized.len() <= limit {
        return value;
    }
    serde_json::json!({
        "truncated": true,
        "original_bytes": serialized.len(),
        "preview": serialized.chars().take(256).collect::<String>(),
    })
}

pub fn sanitize_audit_value(value: &Value) -> Value {
    truncate_json(redact_json(value), AUDIT_PAYLOAD_LIMIT_BYTES)
}

impl Request {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Ping => "ping",
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
            | Self::RemoveSession { session } => Some(session.as_str()),
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

#[cfg(test)]
mod tests {
    use super::{
        AuditSource, AuditTransport, Envelope, Request, casefold_search_text, redact_json,
        sanitize_audit_value,
    };
    use serde_json::json;

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

    #[test]
    fn envelope_parses_bare_request_and_wrapped_request() {
        let bare = r#"{"type":"ping"}"#;
        let parsed = Envelope::parse_line(bare).unwrap();
        assert!(matches!(parsed.request, Request::Ping));
        assert!(parsed.audit.is_none());

        let wrapped = serde_json::to_string(&Envelope::new(
            Request::ListSessions,
            super::AuditContext::new(AuditSource::Cli, AuditTransport::Ipc),
        ))
        .unwrap();
        let parsed = Envelope::parse_line(&wrapped).unwrap();
        assert!(matches!(parsed.request, Request::ListSessions));
        assert_eq!(parsed.audit.unwrap().source, AuditSource::Cli);
    }

    #[test]
    fn redacts_nested_sensitive_fields_and_truncates_large_payloads() {
        let value = json!({
            "token": "secret-value",
            "nested": {"api_key": "abc", "ok": true},
            "items": [{"password": "p", "name": "keep"}]
        });
        let redacted = redact_json(&value);
        assert_eq!(redacted["token"], "[REDACTED]");
        assert_eq!(redacted["nested"]["api_key"], "[REDACTED]");
        assert_eq!(redacted["items"][0]["password"], "[REDACTED]");
        assert_eq!(redacted["items"][0]["name"], "keep");

        let huge = json!({"blob": "x".repeat(70_000)});
        let sanitized = sanitize_audit_value(&huge);
        assert_eq!(sanitized["truncated"], true);
        assert!(sanitized["original_bytes"].as_u64().unwrap() > 64_000);
    }
}
