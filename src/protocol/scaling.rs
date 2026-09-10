//! Scaling policy types.

use serde::{Deserialize, Serialize};

use super::util::{default_cooldown_seconds, default_true};
use super::workflow::WorkflowTargetView;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScalingMetric {
    CpuPercent,
    MemoryBytes,
}

impl ScalingMetric {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScalingMetric::CpuPercent => "cpu_percent",
            ScalingMetric::MemoryBytes => "memory_bytes",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScalingPolicy {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub watch_node_id: String,
    pub watch_session: String,
    pub watch_task: String,
    pub metric: ScalingMetric,
    pub scale_out_threshold: f64,
    pub scale_in_threshold: f64,
    pub scale_out_node_id: String,
    pub scale_out_session: String,
    pub scale_out_task: String,
    pub cooldown_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_action_ms: Option<u64>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScalingPolicyInput {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub watch_node_id: String,
    pub watch_session: String,
    pub watch_task: String,
    pub metric: ScalingMetric,
    pub scale_out_threshold: f64,
    pub scale_in_threshold: f64,
    pub scale_out_node_id: String,
    pub scale_out_session: String,
    pub scale_out_task: String,
    #[serde(default = "default_cooldown_seconds")]
    pub cooldown_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScalingPoliciesView {
    #[serde(default)]
    pub policies: Vec<ScalingPolicy>,
    #[serde(default)]
    pub targets: Vec<WorkflowTargetView>,
}
