//! Workflow group / graph / revision types.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::base::Action;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowGroupMember {
    pub node_id: String,
    pub session: String,
    pub task: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WorkflowGraphNodePosition {
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowGraphEdge {
    #[serde(default)]
    pub from: usize,
    #[serde(default)]
    pub to: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WorkflowGraph {
    #[serde(default)]
    pub positions: Vec<WorkflowGraphNodePosition>,
    #[serde(default)]
    pub edges: Vec<WorkflowGraphEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowGroup {
    pub id: String,
    pub name: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default)]
    pub members: Vec<WorkflowGroupMember>,
    #[serde(default)]
    pub graph: WorkflowGraph,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowGroupInput {
    pub name: String,
    #[serde(default)]
    pub members: Vec<WorkflowGroupMember>,
    #[serde(default)]
    pub graph: WorkflowGraph,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowTargetView {
    pub node_id: String,
    pub node_name: String,
    pub node_online: bool,
    pub session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_alias: Option<String>,
    pub workspace_display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<PathBuf>,
    #[serde(default)]
    pub tasks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowGroupMemberView {
    #[serde(flatten)]
    pub member: WorkflowGroupMember,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowGroupView {
    pub id: String,
    pub name: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default)]
    pub members: Vec<WorkflowGroupMemberView>,
    #[serde(default)]
    pub graph: WorkflowGraph,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowGroupsView {
    #[serde(default)]
    pub groups: Vec<WorkflowGroupView>,
    #[serde(default)]
    pub targets: Vec<WorkflowTargetView>,
    #[serde(default)]
    pub ungrouped: Vec<WorkflowTargetView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowGroupActionItemStatus {
    Success,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowGroupActionItem {
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    pub session: String,
    pub workspace_display_name: String,
    pub task: String,
    pub status: WorkflowGroupActionItemStatus,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowGroupActionSummary {
    pub group_id: String,
    pub group_name: String,
    pub action: Action,
    #[serde(default)]
    pub results: Vec<WorkflowGroupActionItem>,
    pub success_count: usize,
    pub failed_count: usize,
    pub skipped_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowRevision {
    pub group_id: String,
    pub revision: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub members: Vec<WorkflowGroupMember>,
    #[serde(default)]
    pub graph: WorkflowGraph,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowRevisionsView {
    pub group_id: String,
    pub group_name: String,
    #[serde(default)]
    pub revisions: Vec<WorkflowRevision>,
}
