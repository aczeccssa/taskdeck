//! Node identity, node settings and API token wire types.
//!
//! `NodeRole` / `LeaderMode` / `PublicNodeSettings` live here (they are wire
//! types); their state-layer impls remain in `crate::state::node`, and
//! `crate::state` re-exports these names so existing paths keep working.

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    Worker,
    Leader,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum LeaderMode {
    Standard,
    #[value(name = "pure-master")]
    PureMaster,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicNodeSettings {
    pub node_id: String,
    pub name: String,
    pub role: NodeRole,
    pub leader_mode: LeaderMode,
    pub leader_url: Option<String>,
    pub has_enrollment_token: bool,
    pub bind_host: String,
    pub web_port: u16,
    pub execution_enabled: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceScope {
    User,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentOverride {
    pub field: String,
    pub variable: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeSettingsView {
    #[serde(flatten)]
    pub settings: PublicNodeSettings,
    pub environment_overrides: Vec<EnvironmentOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum EnrollmentTokenUpdate {
    Keep,
    Clear,
    Set { value: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeSettingsPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leader_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leader_url: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enrollment_token: Option<EnrollmentTokenUpdate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeSettingsWriteResult {
    pub settings: PublicNodeSettings,
    pub restart_required: bool,
    pub environment_overrides: Vec<EnvironmentOverride>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiToken {
    pub id: String,
    pub name: String,
    pub token_prefix: String,
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at_ms: Option<u64>,
    #[serde(default)]
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiTokenInput {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiTokenCreated {
    #[serde(flatten)]
    pub token: ApiToken,
    pub secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiTokensView {
    #[serde(default)]
    pub tokens: Vec<ApiToken>,
}
