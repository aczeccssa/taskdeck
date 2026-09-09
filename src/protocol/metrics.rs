//! Node / task metrics snapshot types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeMetricsSample {
    pub timestamp_ms: u64,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub memory_total_bytes: u64,
    #[serde(default)]
    pub running_tasks: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct NodeMetricsEntryView {
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    pub online: bool,
    pub is_self: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<NodeMetricsSample>,
    #[serde(default)]
    pub samples: Vec<NodeMetricsSample>,
    #[serde(default)]
    pub session_count: usize,
    #[serde(default)]
    pub task_status_counts: std::collections::BTreeMap<String, u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct NodeMetricsView {
    #[serde(default)]
    pub nodes: Vec<NodeMetricsEntryView>,
    #[serde(default)]
    pub task_status_counts: std::collections::BTreeMap<String, u32>,
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
