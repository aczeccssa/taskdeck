//! Node identity/settings types and node-settings persistence.

use std::env;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use super::util::*;
use super::StateStore;
use crate::protocol::*;

impl StateStore {
    pub fn node_settings(&self) -> Result<NodeSettings> {
        let connection = self.connection.lock().expect("state store lock");
        let mut settings = read_node_settings(&connection)?;
        drop(connection);
        apply_environment(&mut settings)?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn configure(&self, update: NodeSettingsUpdate) -> Result<NodeSettings> {
        let mut connection = self.connection.lock().expect("state store lock");
        let mut settings = read_node_settings(&connection)?;
        if let Some(role) = update.role {
            settings.role = role;
            if role == NodeRole::Worker {
                settings.leader_mode = LeaderMode::Standard;
            } else {
                settings.leader_url = None;
            }
        }
        if let Some(mode) = update.leader_mode {
            settings.leader_mode = mode;
        }
        if let Some(name) = update.name {
            settings.name = name;
        }
        if let Some(leader_url) = update.leader_url {
            settings.leader_url = normalize_optional(leader_url);
        }
        if let Some(token) = update.enrollment_token {
            settings.enrollment_token = normalize_optional(token);
        }
        if let Some(bind_host) = update.bind_host {
            settings.bind_host = bind_host;
        }
        if let Some(web_port) = update.web_port {
            settings.web_port = web_port;
        }
        settings.validate()?;
        if !settings.execution_enabled() {
            let count: i64 =
                connection.query_row("SELECT COUNT(*) FROM registrations", [], |row| row.get(0))?;
            if count > 0 {
                bail!(
                    "cannot enable pure master while {count} local registration(s) remain; remove them first"
                );
            }
        }
        let transaction = connection.transaction()?;
        write_node_settings(&transaction, &settings)?;
        transaction.commit()?;
        Ok(settings)
    }

    pub fn node_settings_view(&self) -> Result<crate::protocol::NodeSettingsView> {
        let settings = self.node_settings()?;
        let overrides = environment_overrides();
        Ok(crate::protocol::NodeSettingsView {
            settings: settings.public(),
            environment_overrides: overrides,
        })
    }

    pub fn configure_patch(
        &self,
        patch: crate::protocol::NodeSettingsPatch,
    ) -> Result<crate::protocol::NodeSettingsWriteResult> {
        let original = self.read_node_settings()?;
        let update = NodeSettingsUpdate {
            role: patch.role.as_deref().map(NodeRole::parse).transpose()?,
            leader_mode: patch
                .leader_mode
                .as_deref()
                .map(LeaderMode::parse)
                .transpose()?,
            name: patch.name.map(|value| value.trim().to_string()),
            leader_url: patch.leader_url,
            enrollment_token: match patch.enrollment_token {
                Some(crate::protocol::EnrollmentTokenUpdate::Keep) => None,
                Some(crate::protocol::EnrollmentTokenUpdate::Clear) => Some(None),
                Some(crate::protocol::EnrollmentTokenUpdate::Set { value }) => Some(Some(value)),
                None => None,
            },
            bind_host: patch.bind_host.map(|value| value.trim().to_string()),
            web_port: patch.web_port,
        };
        let written = self.configure(update)?;
        let restart_required = original != written;
        Ok(crate::protocol::NodeSettingsWriteResult {
            settings: written.public(),
            restart_required,
            environment_overrides: environment_overrides(),
        })
    }

    fn read_node_settings(&self) -> Result<NodeSettings> {
        let connection = self.connection.lock().expect("state store lock");
        read_node_settings(&connection)
    }

}

impl StateStore {
    pub fn upsert_worker(
        &self,
        node_id: &str,
        name: &str,
        last_seen_ms: u64,
        inventory_json: &str,
    ) -> Result<()> {
        let connection = self.connection.lock().expect("state store lock");
        connection.execute(
            "INSERT INTO workers(node_id, name, last_seen_ms, inventory_json)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(node_id) DO UPDATE SET
                 name=excluded.name,
                 last_seen_ms=excluded.last_seen_ms,
                 inventory_json=excluded.inventory_json",
            params![node_id, name, last_seen_ms as i64, inventory_json],
        )?;
        Ok(())
    }

}

impl StateStore {
    pub fn known_workers(&self) -> Result<Vec<KnownWorker>> {
        let connection = self.connection.lock().expect("state store lock");
        let mut statement = connection.prepare(
            "SELECT node_id, name, last_seen_ms, inventory_json FROM workers ORDER BY name, node_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(KnownWorker {
                node_id: row.get(0)?,
                name: row.get(1)?,
                last_seen_ms: row.get::<_, i64>(2)? as u64,
                inventory_json: row.get(3)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read known workers")
    }

}

impl NodeRole {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::Leader => "leader",
        }
    }

    pub(super) fn parse(value: &str) -> Result<Self> {
        match value {
            "worker" => Ok(Self::Worker),
            "leader" => Ok(Self::Leader),
            _ => bail!("invalid node role '{value}'"),
        }
    }

    pub fn as_label(self) -> &'static str {
        self.as_str()
    }
}

impl LeaderMode {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::PureMaster => "pure_master",
        }
    }

    pub(super) fn parse(value: &str) -> Result<Self> {
        match value {
            "standard" => Ok(Self::Standard),
            "pure_master" | "pure-master" => Ok(Self::PureMaster),
            _ => bail!("invalid leader mode '{value}'"),
        }
    }

    pub fn as_label(self) -> &'static str {
        self.as_str()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeSettings {
    pub node_id: String,
    pub name: String,
    pub role: NodeRole,
    pub leader_mode: LeaderMode,
    pub leader_url: Option<String>,
    #[serde(skip_serializing)]
    pub enrollment_token: Option<String>,
    pub bind_host: String,
    pub web_port: u16,
}

impl NodeSettings {
    pub fn execution_enabled(&self) -> bool {
        self.role == NodeRole::Worker || self.leader_mode == LeaderMode::Standard
    }

    pub fn public(&self) -> PublicNodeSettings {
        PublicNodeSettings {
            node_id: self.node_id.clone(),
            name: self.name.clone(),
            role: self.role,
            leader_mode: self.leader_mode,
            leader_url: self.leader_url.clone(),
            has_enrollment_token: self.enrollment_token.is_some(),
            bind_host: self.bind_host.clone(),
            web_port: self.web_port,
            execution_enabled: self.execution_enabled(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("node name cannot be empty");
        }
        if self.bind_host.trim().is_empty() {
            bail!("bind host cannot be empty");
        }
        if self.web_port == 0 {
            bail!("web port must be greater than zero");
        }
        match self.role {
            NodeRole::Worker => {
                if self.leader_mode != LeaderMode::Standard {
                    bail!("leader mode is only valid when role is leader");
                }
            }
            NodeRole::Leader => {
                if self.leader_url.is_some() {
                    bail!("a leader cannot connect to an upstream leader");
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct NodeSettingsUpdate {
    pub role: Option<NodeRole>,
    pub leader_mode: Option<LeaderMode>,
    pub name: Option<String>,
    pub leader_url: Option<Option<String>>,
    pub enrollment_token: Option<Option<String>>,
    pub bind_host: Option<String>,
    pub web_port: Option<u16>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSettingsWrite {
    pub settings: NodeSettings,
    pub restart_required: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownWorker {
    pub node_id: String,
    pub name: String,
    pub last_seen_ms: u64,
    pub inventory_json: String,
}
pub(super) fn read_node_settings(connection: &Connection) -> Result<NodeSettings> {
    let web_port = required_metadata(connection, "web_port")?
        .parse::<u16>()
        .context("invalid persisted web port")?;
    Ok(NodeSettings {
        node_id: required_metadata(connection, "node_id")?,
        name: required_metadata(connection, "node_name")?,
        role: NodeRole::parse(&required_metadata(connection, "role")?)?,
        leader_mode: LeaderMode::parse(&required_metadata(connection, "leader_mode")?)?,
        leader_url: get_metadata(connection, "leader_url")?,
        enrollment_token: get_metadata(connection, "enrollment_token")?,
        bind_host: required_metadata(connection, "bind_host")?,
        web_port,
    })
}

pub(super) fn write_node_settings(connection: &Connection, settings: &NodeSettings) -> Result<()> {
    set_metadata(connection, "node_name", &settings.name)?;
    set_metadata(connection, "role", settings.role.as_str())?;
    set_metadata(connection, "leader_mode", settings.leader_mode.as_str())?;
    set_metadata(connection, "bind_host", &settings.bind_host)?;
    set_metadata(connection, "web_port", &settings.web_port.to_string())?;
    write_optional_metadata(connection, "leader_url", settings.leader_url.as_deref())?;
    write_optional_metadata(
        connection,
        "enrollment_token",
        settings.enrollment_token.as_deref(),
    )?;
    Ok(())
}

pub(super) fn apply_environment(settings: &mut NodeSettings) -> Result<()> {
    if let Ok(value) = env::var("TASKDECK_ROLE") {
        settings.role = NodeRole::parse(&value)?;
    }
    if let Ok(value) = env::var("TASKDECK_LEADER_MODE") {
        settings.leader_mode = LeaderMode::parse(&value)?;
    }
    if let Ok(value) = env::var("TASKDECK_NODE_NAME") {
        settings.name = value;
    }
    if let Ok(value) = env::var("TASKDECK_LEADER_URL") {
        settings.leader_url = normalize_optional(Some(value));
    }
    if let Ok(value) = env::var("TASKDECK_ENROLLMENT_TOKEN") {
        settings.enrollment_token = normalize_optional(Some(value));
    }
    if let Ok(value) = env::var("TASKDECK_BIND_HOST") {
        settings.bind_host = value;
    }
    if let Ok(value) = env::var("TASKDECK_WEB_PORT") {
        settings.web_port = value.parse().context("invalid TASKDECK_WEB_PORT")?;
    }
    if settings.role == NodeRole::Worker {
        settings.leader_mode = LeaderMode::Standard;
    } else {
        settings.leader_url = None;
    }
    Ok(())
}

pub fn environment_overrides() -> Vec<crate::protocol::EnvironmentOverride> {
    [
        ("role", "TASKDECK_ROLE"),
        ("leader_mode", "TASKDECK_LEADER_MODE"),
        ("name", "TASKDECK_NODE_NAME"),
        ("leader_url", "TASKDECK_LEADER_URL"),
        ("enrollment_token", "TASKDECK_ENROLLMENT_TOKEN"),
        ("bind_host", "TASKDECK_BIND_HOST"),
        ("web_port", "TASKDECK_WEB_PORT"),
    ]
    .into_iter()
    .filter_map(|(field, variable)| {
        env::var(variable)
            .is_ok()
            .then_some(crate::protocol::EnvironmentOverride {
                field: field.to_string(),
                variable: variable.to_string(),
            })
    })
    .collect()
}

