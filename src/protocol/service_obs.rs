//! Managed-service observation types (classification, endpoints, inspection).

use serde::{Deserialize, Serialize};

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
