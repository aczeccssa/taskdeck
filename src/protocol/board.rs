//! Board and board-template types.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::workflow::WorkflowTargetView;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoardCardMode {
    #[default]
    Status,
    Logs,
    Metrics,
}

impl BoardCardMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            BoardCardMode::Status => "status",
            BoardCardMode::Logs => "logs",
            BoardCardMode::Metrics => "metrics",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardCard {
    pub id: String,
    pub node_id: String,
    pub session: String,
    pub task: String,
    #[serde(default)]
    pub mode: BoardCardMode,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardCardInput {
    pub node_id: String,
    pub session: String,
    pub task: String,
    #[serde(default)]
    pub mode: BoardCardMode,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Board {
    pub id: String,
    pub name: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default)]
    pub cards: Vec<BoardCard>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardInput {
    pub name: String,
    #[serde(default)]
    pub cards: Vec<BoardCardInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardCardView {
    #[serde(flatten)]
    pub card: BoardCard,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    pub node_online: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_alias: Option<String>,
    pub workspace_display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<PathBuf>,
    pub task_exists: bool,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardView {
    pub id: String,
    pub name: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default)]
    pub cards: Vec<BoardCardView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardsView {
    #[serde(default)]
    pub boards: Vec<BoardView>,
    #[serde(default)]
    pub targets: Vec<WorkflowTargetView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardTemplate {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub cards: Vec<BoardCardInput>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardTemplateInput {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub cards: Vec<BoardCardInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_board_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardTemplateApplyInput {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardTemplateExport {
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub cards: Vec<BoardCardInput>,
    pub exported_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardTemplatesView {
    #[serde(default)]
    pub templates: Vec<BoardTemplate>,
}
