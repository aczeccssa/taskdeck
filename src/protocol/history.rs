//! History record types: task runs, events and MCP calls.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::base::Response;
use super::util::{parse_history_page_size, parse_positive_usize};

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
