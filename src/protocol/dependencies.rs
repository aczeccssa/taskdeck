//! Task dependency types.

use serde::{Deserialize, Serialize};

use super::util::default_required_state;
use super::workflow::WorkflowTargetView;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskDependency {
    pub id: String,
    pub node_id: String,
    pub session: String,
    pub task: String,
    pub depends_node_id: String,
    pub depends_session: String,
    pub depends_task: String,
    #[serde(default = "default_required_state")]
    pub required_state: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskDependencyInput {
    pub node_id: String,
    pub session: String,
    pub task: String,
    pub depends_node_id: String,
    pub depends_session: String,
    pub depends_task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskDependencyView {
    #[serde(flatten)]
    pub dependency: TaskDependency,
    pub target_exists: bool,
    pub target_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskDependenciesView {
    #[serde(default)]
    pub dependencies: Vec<TaskDependencyView>,
    #[serde(default)]
    pub targets: Vec<WorkflowTargetView>,
}
