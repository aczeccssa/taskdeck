use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DEFAULT_BIND_HOST: &str = "0.0.0.0";
pub const DEFAULT_WEB_PORT: u16 = 9837;
const DATABASE_FILE: &str = "state.db";
const SCHEMA_VERSION: &str = "2";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    Worker,
    Leader,
}

impl NodeRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::Leader => "leader",
        }
    }

    fn parse(value: &str) -> Result<Self> {
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum LeaderMode {
    Standard,
    #[value(name = "pure-master")]
    PureMaster,
}

impl LeaderMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::PureMaster => "pure_master",
        }
    }

    fn parse(value: &str) -> Result<Self> {
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
pub struct Registration {
    pub session: String,
    pub project: PathBuf,
    pub registered_at_ms: u64,
}

pub struct StateStore {
    connection: Mutex<Connection>,
}

impl StateStore {
    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root).with_context(|| format!("failed to create {}", root.display()))?;
        let connection = Connection::open(root.join(DATABASE_FILE))
            .with_context(|| format!("failed to open {}/{}", root.display(), DATABASE_FILE))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS metadata (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS registrations (
                 session TEXT PRIMARY KEY,
                 project TEXT NOT NULL,
                 registered_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS workers (
                 node_id TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 last_seen_ms INTEGER NOT NULL,
                 inventory_json TEXT NOT NULL
             );",
        )?;
        let store = Self {
            connection: Mutex::new(connection),
        };
        store.initialize()?;
        Ok(store)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory().context("failed to open in-memory state")?;
        connection.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE metadata (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE registrations (
                 session TEXT PRIMARY KEY,
                 project TEXT NOT NULL,
                 registered_at_ms INTEGER NOT NULL
             );
             CREATE TABLE workers (
                 node_id TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 last_seen_ms INTEGER NOT NULL,
                 inventory_json TEXT NOT NULL
             );",
        )?;
        let store = Self {
            connection: Mutex::new(connection),
        };
        store.initialize()?;
        Ok(store)
    }

    fn initialize(&self) -> Result<()> {
        let connection = self.connection.lock().expect("state store lock");
        let version = get_metadata(&connection, "schema_version")?;
        match version.as_deref() {
            None => set_metadata(&connection, "schema_version", SCHEMA_VERSION)?,
            Some("1") => {
                if get_metadata(&connection, "bind_host")?.as_deref() == Some("127.0.0.1") {
                    set_metadata(&connection, "bind_host", DEFAULT_BIND_HOST)?;
                }
                set_metadata(&connection, "schema_version", SCHEMA_VERSION)?;
            }
            Some(SCHEMA_VERSION) => {}
            Some(other) => bail!("unsupported state database schema version '{other}'"),
        }
        if get_metadata(&connection, "node_id")?.is_none() {
            let node_id = Uuid::new_v4().to_string();
            set_metadata(&connection, "node_id", &node_id)?;
            let short_id = node_id.split('-').next().unwrap_or("local");
            set_metadata(&connection, "node_name", &format!("taskdeck-{short_id}"))?;
            set_metadata(&connection, "role", NodeRole::Worker.as_str())?;
            set_metadata(&connection, "leader_mode", LeaderMode::Standard.as_str())?;
            set_metadata(&connection, "bind_host", DEFAULT_BIND_HOST)?;
            set_metadata(&connection, "web_port", &DEFAULT_WEB_PORT.to_string())?;
        }
        Ok(())
    }

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

    pub fn registrations(&self) -> Result<Vec<Registration>> {
        let connection = self.connection.lock().expect("state store lock");
        let mut statement = connection.prepare(
            "SELECT session, project, registered_at_ms FROM registrations ORDER BY session",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Registration {
                session: row.get(0)?,
                project: PathBuf::from(row.get::<_, String>(1)?),
                registered_at_ms: row.get::<_, i64>(2)? as u64,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read registrations")
    }

    pub fn upsert_registration(&self, session: &str, project: &Path) -> Result<()> {
        let project = project
            .to_str()
            .with_context(|| format!("project path is not valid UTF-8: {}", project.display()))?;
        let connection = self.connection.lock().expect("state store lock");
        connection.execute(
            "INSERT INTO registrations(session, project, registered_at_ms)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(session) DO UPDATE SET
                 project=excluded.project,
                 registered_at_ms=excluded.registered_at_ms",
            params![session, project, current_timestamp_ms() as i64],
        )?;
        Ok(())
    }

    pub fn remove_registration(&self, session: &str) -> Result<bool> {
        let connection = self.connection.lock().expect("state store lock");
        Ok(connection.execute(
            "DELETE FROM registrations WHERE session=?1",
            params![session],
        )? > 0)
    }

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownWorker {
    pub node_id: String,
    pub name: String,
    pub last_seen_ms: u64,
    pub inventory_json: String,
}

fn get_metadata(connection: &Connection, key: &str) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT value FROM metadata WHERE key=?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .with_context(|| format!("failed to read state key '{key}'"))
}

fn set_metadata(connection: &Connection, key: &str, value: &str) -> Result<()> {
    connection.execute(
        "INSERT INTO metadata(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn required_metadata(connection: &Connection, key: &str) -> Result<String> {
    get_metadata(connection, key)?.with_context(|| format!("missing state key '{key}'"))
}

fn read_node_settings(connection: &Connection) -> Result<NodeSettings> {
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

fn write_node_settings(connection: &Connection, settings: &NodeSettings) -> Result<()> {
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

fn write_optional_metadata(connection: &Connection, key: &str, value: Option<&str>) -> Result<()> {
    match value {
        Some(value) => set_metadata(connection, key, value),
        None => {
            connection.execute("DELETE FROM metadata WHERE key=?1", params![key])?;
            Ok(())
        }
    }
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

fn apply_environment(settings: &mut NodeSettings) -> Result<()> {
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

fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_store_defaults_to_unlinked_worker_and_keeps_identity() {
        let dir = tempfile::tempdir().unwrap();
        let first = StateStore::open(dir.path())
            .unwrap()
            .node_settings()
            .unwrap();
        let second = StateStore::open(dir.path())
            .unwrap()
            .node_settings()
            .unwrap();
        assert_eq!(first.role, NodeRole::Worker);
        assert_eq!(first.leader_mode, LeaderMode::Standard);
        assert_eq!(first.node_id, second.node_id);
        assert!(first.leader_url.is_none());
        assert_eq!(first.bind_host, DEFAULT_BIND_HOST);
    }

    #[test]
    fn schema_one_migrates_the_old_default_bind_host() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::open(dir.path()).unwrap();
        {
            let connection = store.connection.lock().unwrap();
            set_metadata(&connection, "schema_version", "1").unwrap();
            set_metadata(&connection, "bind_host", "127.0.0.1").unwrap();
        }
        drop(store);

        let migrated = StateStore::open(dir.path()).unwrap();
        assert_eq!(migrated.node_settings().unwrap().bind_host, "0.0.0.0");
        let connection = migrated.connection.lock().unwrap();
        assert_eq!(
            get_metadata(&connection, "schema_version")
                .unwrap()
                .as_deref(),
            Some(SCHEMA_VERSION)
        );
    }

    #[test]
    fn schema_one_preserves_a_custom_bind_host() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::open(dir.path()).unwrap();
        {
            let connection = store.connection.lock().unwrap();
            set_metadata(&connection, "schema_version", "1").unwrap();
            set_metadata(&connection, "bind_host", "192.168.1.20").unwrap();
        }
        drop(store);

        let migrated = StateStore::open(dir.path()).unwrap();
        assert_eq!(migrated.node_settings().unwrap().bind_host, "192.168.1.20");
    }

    #[test]
    fn registrations_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::open(dir.path()).unwrap();
        store
            .upsert_registration("api", Path::new("/tmp/api"))
            .unwrap();
        drop(store);
        let registrations = StateStore::open(dir.path())
            .unwrap()
            .registrations()
            .unwrap();
        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].session, "api");
        assert_eq!(registrations[0].project, Path::new("/tmp/api"));
    }

    #[test]
    fn pure_master_requires_empty_local_registry() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::open(dir.path()).unwrap();
        store
            .upsert_registration("api", Path::new("/tmp/api"))
            .unwrap();
        let error = store
            .configure(NodeSettingsUpdate {
                role: Some(NodeRole::Leader),
                leader_mode: Some(LeaderMode::PureMaster),
                ..NodeSettingsUpdate::default()
            })
            .unwrap_err();
        assert!(error.to_string().contains("local registration"));
    }

    #[test]
    fn public_settings_redact_token() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::open(dir.path()).unwrap();
        let settings = store
            .configure(NodeSettingsUpdate {
                enrollment_token: Some(Some("secret".to_string())),
                ..NodeSettingsUpdate::default()
            })
            .unwrap();
        let serialized = serde_json::to_string(&settings.public()).unwrap();
        assert!(!serialized.contains("secret"));
        assert!(settings.public().has_enrollment_token);
    }
}
