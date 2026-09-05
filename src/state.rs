use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use chrono::{DateTime, Local, TimeZone, Utc};
use clap::ValueEnum;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::protocol::{
    AuditFilter, AuditListItem, AuditListPage, AuditRecord, AuditSource, AuditStatus,
    AuditTransport, EventFilter, EventListPage, EventRecord, McpCallListItem, McpCallListPage,
    McpCallRecord, TaskRunFilter, TaskRunListPage, TaskRunRecord, casefold_search_text,
    sanitize_audit_value,
};

pub const DEFAULT_BIND_HOST: &str = "0.0.0.0";
pub const DEFAULT_WEB_PORT: u16 = 9837;
const DATABASE_FILE: &str = "state.db";
pub const AUTH_SESSION_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;
const SCHEMA_VERSION: &str = "4";
pub const AUDIT_RETENTION_LIMIT: usize = 10_000;

#[cfg(test)]
use crate::protocol::TaskStatus;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthSettings {
    pub enabled: bool,
    #[serde(skip_serializing)]
    pub password_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicAuthStatus {
    pub enabled: bool,
    pub configured: bool,
}

impl AuthSettings {
    pub fn public(&self) -> PublicAuthStatus {
        PublicAuthStatus {
            enabled: self.enabled,
            configured: self.password_hash.is_some(),
        }
    }
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
             );
             CREATE TABLE IF NOT EXISTS task_runs (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 node_id TEXT NOT NULL,
                 session TEXT NOT NULL,
                 task TEXT NOT NULL,
                 trigger TEXT NOT NULL,
                 status TEXT NOT NULL,
                 started_at_ms INTEGER NOT NULL,
                 finished_at_ms INTEGER,
                 duration_ms INTEGER,
                 command TEXT NOT NULL,
                 cwd TEXT NOT NULL,
                 pid INTEGER,
                 run_generation INTEGER NOT NULL DEFAULT 0,
                 exit_code INTEGER,
                 error_message TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_task_runs_recent ON task_runs(started_at_ms DESC);
             CREATE INDEX IF NOT EXISTS idx_task_runs_target ON task_runs(session, task, run_generation);
             CREATE TABLE IF NOT EXISTS mcp_calls (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 tool TEXT NOT NULL,
                 operation TEXT,
                 started_at_ms INTEGER NOT NULL,
                 duration_ms INTEGER NOT NULL,
                 success INTEGER NOT NULL,
                 target_node TEXT,
                 request_json TEXT NOT NULL,
                 response_json TEXT NOT NULL,
                 input_json TEXT NOT NULL,
                 searchable_text TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_mcp_calls_recent ON mcp_calls(started_at_ms DESC);
             CREATE TABLE IF NOT EXISTS events (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 timestamp_ms INTEGER NOT NULL,
                 category TEXT NOT NULL,
                 message TEXT NOT NULL,
                 details_json TEXT NOT NULL DEFAULT '{}'
             );
             CREATE INDEX IF NOT EXISTS idx_events_recent ON events(timestamp_ms DESC);

             CREATE TABLE IF NOT EXISTS audit_records (
                 audit_id TEXT PRIMARY KEY,
                 correlation_id TEXT NOT NULL,
                 timestamp_ms INTEGER NOT NULL,
                 duration_ms INTEGER NOT NULL,
                 source TEXT NOT NULL,
                 transport TEXT NOT NULL,
                 origin_node_id TEXT,
                 executor_node_id TEXT,
                 request_kind TEXT NOT NULL,
                 operation TEXT NOT NULL,
                 session TEXT,
                 task TEXT,
                 status TEXT NOT NULL,
                 success INTEGER NOT NULL,
                 error TEXT,
                 request_json TEXT NOT NULL,
                 response_json TEXT NOT NULL,
                 details_json TEXT NOT NULL DEFAULT '{}',
                 searchable_text TEXT NOT NULL,
                 replicated_at_ms INTEGER
             );
             CREATE INDEX IF NOT EXISTS idx_audit_records_recent ON audit_records(timestamp_ms DESC, audit_id DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_records_correlation ON audit_records(correlation_id);
             CREATE INDEX IF NOT EXISTS idx_audit_records_source ON audit_records(source, timestamp_ms DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_records_status ON audit_records(status, timestamp_ms DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_records_origin ON audit_records(origin_node_id, timestamp_ms DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_records_executor ON audit_records(executor_node_id, timestamp_ms DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_records_operation ON audit_records(operation, timestamp_ms DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_records_target ON audit_records(session, task, timestamp_ms DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_records_unreplicated ON audit_records(replicated_at_ms, timestamp_ms);

             CREATE TABLE IF NOT EXISTS auth_settings (
                 id INTEGER PRIMARY KEY CHECK(id = 1),
                 enabled INTEGER NOT NULL DEFAULT 0,
                 password_hash TEXT,
                 updated_at_ms INTEGER NOT NULL
             );
             INSERT OR IGNORE INTO auth_settings(id, enabled, updated_at_ms) VALUES (1, 0, 0);
             CREATE TABLE IF NOT EXISTS auth_sessions (
                 token_hash TEXT PRIMARY KEY,
                 created_at_ms INTEGER NOT NULL,
                 expires_at_ms INTEGER NOT NULL,
                 last_seen_at_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_auth_sessions_expiry ON auth_sessions(expires_at_ms);",
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
             );
             CREATE TABLE IF NOT EXISTS task_runs (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 node_id TEXT NOT NULL,
                 session TEXT NOT NULL,
                 task TEXT NOT NULL,
                 trigger TEXT NOT NULL,
                 status TEXT NOT NULL,
                 started_at_ms INTEGER NOT NULL,
                 finished_at_ms INTEGER,
                 duration_ms INTEGER,
                 command TEXT NOT NULL,
                 cwd TEXT NOT NULL,
                 pid INTEGER,
                 run_generation INTEGER NOT NULL DEFAULT 0,
                 exit_code INTEGER,
                 error_message TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_task_runs_recent ON task_runs(started_at_ms DESC);
             CREATE INDEX IF NOT EXISTS idx_task_runs_target ON task_runs(session, task, run_generation);
             CREATE TABLE IF NOT EXISTS mcp_calls (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 tool TEXT NOT NULL,
                 operation TEXT,
                 started_at_ms INTEGER NOT NULL,
                 duration_ms INTEGER NOT NULL,
                 success INTEGER NOT NULL,
                 target_node TEXT,
                 request_json TEXT NOT NULL,
                 response_json TEXT NOT NULL,
                 input_json TEXT NOT NULL,
                 searchable_text TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_mcp_calls_recent ON mcp_calls(started_at_ms DESC);
             CREATE TABLE IF NOT EXISTS events (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 timestamp_ms INTEGER NOT NULL,
                 category TEXT NOT NULL,
                 message TEXT NOT NULL,
                 details_json TEXT NOT NULL DEFAULT '{}'
             );
             CREATE INDEX IF NOT EXISTS idx_events_recent ON events(timestamp_ms DESC);

             CREATE TABLE IF NOT EXISTS audit_records (
                 audit_id TEXT PRIMARY KEY,
                 correlation_id TEXT NOT NULL,
                 timestamp_ms INTEGER NOT NULL,
                 duration_ms INTEGER NOT NULL,
                 source TEXT NOT NULL,
                 transport TEXT NOT NULL,
                 origin_node_id TEXT,
                 executor_node_id TEXT,
                 request_kind TEXT NOT NULL,
                 operation TEXT NOT NULL,
                 session TEXT,
                 task TEXT,
                 status TEXT NOT NULL,
                 success INTEGER NOT NULL,
                 error TEXT,
                 request_json TEXT NOT NULL,
                 response_json TEXT NOT NULL,
                 details_json TEXT NOT NULL DEFAULT '{}',
                 searchable_text TEXT NOT NULL,
                 replicated_at_ms INTEGER
             );
             CREATE INDEX IF NOT EXISTS idx_audit_records_recent ON audit_records(timestamp_ms DESC, audit_id DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_records_correlation ON audit_records(correlation_id);
             CREATE INDEX IF NOT EXISTS idx_audit_records_source ON audit_records(source, timestamp_ms DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_records_status ON audit_records(status, timestamp_ms DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_records_origin ON audit_records(origin_node_id, timestamp_ms DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_records_executor ON audit_records(executor_node_id, timestamp_ms DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_records_operation ON audit_records(operation, timestamp_ms DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_records_target ON audit_records(session, task, timestamp_ms DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_records_unreplicated ON audit_records(replicated_at_ms, timestamp_ms);

             CREATE TABLE IF NOT EXISTS auth_settings (
                 id INTEGER PRIMARY KEY CHECK(id = 1),
                 enabled INTEGER NOT NULL DEFAULT 0,
                 password_hash TEXT,
                 updated_at_ms INTEGER NOT NULL
             );
             INSERT OR IGNORE INTO auth_settings(id, enabled, updated_at_ms) VALUES (1, 0, 0);
             CREATE TABLE IF NOT EXISTS auth_sessions (
                 token_hash TEXT PRIMARY KEY,
                 created_at_ms INTEGER NOT NULL,
                 expires_at_ms INTEGER NOT NULL,
                 last_seen_at_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_auth_sessions_expiry ON auth_sessions(expires_at_ms);",
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
            Some("1" | "2" | "3") => {
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

    pub fn auth_settings(&self) -> Result<AuthSettings> {
        let connection = self.connection.lock().expect("state store lock");
        read_auth_settings(&connection)
    }

    pub fn apply_auth_environment(&self) -> Result<AuthSettings> {
        let connection = self.connection.lock().expect("state store lock");
        let mut settings = read_auth_settings(&connection)?;
        if let Ok(value) = std::env::var("TASKDECK_AUTH_ENABLED") {
            settings.enabled = matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            );
        }
        if settings.enabled && settings.password_hash.is_none() {
            let default = std::env::var("TASKDECK_ACCESS_KEY").map_err(|_| {
                anyhow::anyhow!("auth is enabled but no TASKDECK_ACCESS_KEY is configured")
            })?;
            settings.password_hash = Some(hash_access_key(&default)?);
        }
        write_auth_settings(&connection, &settings)?;
        Ok(settings)
    }

    pub fn configure_auth(&self, enabled: bool) -> Result<AuthSettings> {
        let connection = self.connection.lock().expect("state store lock");
        let mut settings = read_auth_settings(&connection)?;
        if !enabled {
            settings.password_hash = None;
        }
        settings.enabled = enabled;
        write_auth_settings(&connection, &settings)?;
        Ok(settings)
    }

    pub fn set_access_key(&self, key: &str) -> Result<()> {
        if key.trim().is_empty() {
            bail!("access key cannot be empty");
        }
        let connection = self.connection.lock().expect("state store lock");
        let mut settings = read_auth_settings(&connection)?;
        settings.password_hash = Some(hash_access_key(key)?);
        write_auth_settings(&connection, &settings)?;
        Ok(())
    }

    pub fn create_auth_session(&self) -> Result<String> {
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let now = current_timestamp_ms();
        let expires = now + AUTH_SESSION_TTL_SECONDS * 1000;
        self.purge_expired_auth_sessions(now)?;
        let connection = self.connection.lock().expect("state store lock");
        connection.execute(
            "INSERT INTO auth_sessions(token_hash, created_at_ms, expires_at_ms, last_seen_at_ms) VALUES (?1, ?2, ?3, ?3)",
            params![sha256_hex(token.as_bytes()), now as i64, expires as i64],
        )?;
        Ok(token)
    }

    pub fn valid_auth_session(&self, token: Option<&str>) -> bool {
        let Some(token) = token else { return false };
        let now = current_timestamp_ms();
        let connection = match self.connection.try_lock() {
            Ok(connection) => connection,
            Err(_) => return false,
        };
        let _ = connection.execute(
            "DELETE FROM auth_sessions WHERE expires_at_ms <= ?1",
            params![now as i64],
        );
        connection.execute(
            "UPDATE auth_sessions SET last_seen_at_ms=?2 WHERE token_hash=?1 AND expires_at_ms > ?2",
            params![sha256_hex(token.as_bytes()), now as i64],
        ).is_ok_and(|count| count == 1)
    }

    pub fn delete_auth_session(&self, token: Option<&str>) {
        if let Some(token) = token {
            let _ = self.connection.lock().expect("state store lock").execute(
                "DELETE FROM auth_sessions WHERE token_hash=?1",
                params![sha256_hex(token.as_bytes())],
            );
        }
    }

    pub fn purge_expired_auth_sessions(&self, now_ms: u64) -> Result<()> {
        let connection = self.connection.lock().expect("state store lock");
        connection.execute(
            "DELETE FROM auth_sessions WHERE expires_at_ms <= ?1",
            params![now_ms as i64],
        )?;
        Ok(())
    }

    pub fn verify_access_key(&self, candidate: &str) -> Result<bool> {
        let settings = self.auth_settings()?;
        let Some(hash) = settings.password_hash else {
            return Ok(false);
        };
        Ok(verify_access_key(candidate, &hash))
    }

    pub fn record_event(
        &self,
        category: &str,
        message: &str,
        details: serde_json::Value,
    ) -> Result<EventRecord> {
        let details_json = serde_json::to_string(&details)?;
        let timestamp = current_timestamp_ms();
        let connection = self.connection.lock().expect("state store lock");
        connection.execute(
            "INSERT INTO events(timestamp_ms,category,message,details_json) VALUES (?1,?2,?3,?4)",
            params![timestamp as i64, category, message, details_json],
        )?;
        Ok(EventRecord {
            id: connection.last_insert_rowid() as u64,
            timestamp_ms: timestamp,
            category: category.to_string(),
            message: message.to_string(),
            details,
        })
    }

    pub fn record_audit(&self, mut record: AuditRecord) -> Result<AuditRecord> {
        record.request = sanitize_audit_value(&record.request);
        record.response = sanitize_audit_value(&record.response);
        record.details = sanitize_audit_value(&record.details);
        if record.audit_id.trim().is_empty() {
            record.audit_id = Uuid::new_v4().to_string();
        }
        if record.correlation_id.trim().is_empty() {
            record.correlation_id = Uuid::new_v4().to_string();
        }
        let request_json = serde_json::to_string(&record.request)?;
        let response_json = serde_json::to_string(&record.response)?;
        let details_json = serde_json::to_string(&record.details)?;
        let searchable_text =
            build_audit_search_text(&record, &request_json, &response_json, &details_json);
        let connection = self.connection.lock().expect("state store lock");
        connection.execute(
            "INSERT INTO audit_records(
                audit_id,correlation_id,timestamp_ms,duration_ms,source,transport,origin_node_id,executor_node_id,
                request_kind,operation,session,task,status,success,error,request_json,response_json,details_json,searchable_text,replicated_at_ms
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)
            ON CONFLICT(audit_id) DO UPDATE SET
                correlation_id=excluded.correlation_id,
                timestamp_ms=excluded.timestamp_ms,
                duration_ms=excluded.duration_ms,
                source=excluded.source,
                transport=excluded.transport,
                origin_node_id=excluded.origin_node_id,
                executor_node_id=excluded.executor_node_id,
                request_kind=excluded.request_kind,
                operation=excluded.operation,
                session=excluded.session,
                task=excluded.task,
                status=excluded.status,
                success=excluded.success,
                error=excluded.error,
                request_json=excluded.request_json,
                response_json=excluded.response_json,
                details_json=excluded.details_json,
                searchable_text=excluded.searchable_text,
                replicated_at_ms=COALESCE(audit_records.replicated_at_ms, excluded.replicated_at_ms)",
            params![
                record.audit_id,
                record.correlation_id,
                record.timestamp_ms as i64,
                record.duration_ms as i64,
                record.source.as_str(),
                record.transport.as_str(),
                record.origin_node_id,
                record.executor_node_id,
                record.request_kind,
                record.operation,
                record.session,
                record.task,
                record.status.as_str(),
                i64::from(record.success),
                record.error,
                request_json,
                response_json,
                details_json,
                searchable_text,
                record.replicated_at_ms.map(|value| value as i64),
            ],
        )?;
        drop(connection);
        self.prune_audit_records()?;
        Ok(record)
    }

    pub fn list_audit(&self, filter: &AuditFilter) -> Result<AuditListPage> {
        let mut sql_conditions = Vec::<String>::new();
        let mut values = Vec::<rusqlite::types::Value>::new();
        if let Some(q) = filter
            .q
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            sql_conditions.push("searchable_text LIKE ? ESCAPE '\\'".to_string());
            values.push(format!("%{}%", escape_like(&casefold_search_text(q))).into());
        }
        if let Some(source) = filter.source.as_deref() {
            sql_conditions.push("source = ?".to_string());
            values.push(source.to_string().into());
        }
        if let Some(status) = filter.status.as_deref() {
            sql_conditions.push("status = ?".to_string());
            values.push(status.to_string().into());
        }
        if let Some(node) = filter.node.as_deref() {
            sql_conditions.push("(origin_node_id = ? OR executor_node_id = ?)".to_string());
            values.push(node.to_string().into());
            values.push(node.to_string().into());
        }
        if let Some(session) = filter.session.as_deref() {
            sql_conditions.push("session = ?".to_string());
            values.push(session.to_string().into());
        }
        if let Some(task) = filter.task.as_deref() {
            sql_conditions.push("task = ?".to_string());
            values.push(task.to_string().into());
        }
        if let Some(operation) = filter.operation.as_deref() {
            sql_conditions.push("operation = ?".to_string());
            values.push(operation.to_string().into());
        }
        let where_sql = where_clause(&sql_conditions);
        let connection = self.connection.lock().expect("state store lock");
        let total: i64 = connection.query_row(
            format!("SELECT COUNT(*) FROM audit_records{where_sql}").as_str(),
            params_from_iter(values.clone()),
            |row| row.get(0),
        )?;
        let offset = (filter
            .page
            .saturating_sub(1)
            .saturating_mul(filter.page_size)) as i64;
        values.push((filter.page_size as i64).into());
        values.push(offset.into());
        let sql = format!(
            "SELECT audit_id,correlation_id,timestamp_ms,duration_ms,source,transport,origin_node_id,executor_node_id,request_kind,operation,session,task,status,success,error,replicated_at_ms FROM audit_records{where_sql} ORDER BY timestamp_ms DESC, audit_id DESC LIMIT ? OFFSET ?"
        );
        let mut statement = connection.prepare(sql.as_str())?;
        let rows = statement
            .query_map(params_from_iter(values), map_audit_list_item)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(paginated_audit(rows, total, filter.page, filter.page_size))
    }

    pub fn audit_detail(&self, audit_id: &str) -> Result<Option<AuditRecord>> {
        let connection = self.connection.lock().expect("state store lock");
        Ok(connection
            .query_row(
                "SELECT audit_id,correlation_id,timestamp_ms,duration_ms,source,transport,origin_node_id,executor_node_id,request_kind,operation,session,task,status,success,error,request_json,response_json,details_json,replicated_at_ms FROM audit_records WHERE audit_id=?1",
                params![audit_id],
                map_audit_record,
            )
            .optional()?)
    }

    pub fn unreplicated_audit_records(&self, limit: usize) -> Result<Vec<AuditRecord>> {
        let connection = self.connection.lock().expect("state store lock");
        let mut statement = connection.prepare(
            "SELECT audit_id,correlation_id,timestamp_ms,duration_ms,source,transport,origin_node_id,executor_node_id,request_kind,operation,session,task,status,success,error,request_json,response_json,details_json,replicated_at_ms FROM audit_records WHERE replicated_at_ms IS NULL ORDER BY timestamp_ms ASC, audit_id ASC LIMIT ?1",
        )?;
        let rows = statement
            .query_map(params![limit as i64], map_audit_record)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn mark_audit_replicated(
        &self,
        audit_ids: &[String],
        replicated_at_ms: u64,
    ) -> Result<usize> {
        if audit_ids.is_empty() {
            return Ok(0);
        }
        let connection = self.connection.lock().expect("state store lock");
        let mut updated = 0usize;
        for audit_id in audit_ids {
            updated += connection.execute(
                "UPDATE audit_records SET replicated_at_ms=?1 WHERE audit_id=?2 AND replicated_at_ms IS NULL",
                params![replicated_at_ms as i64, audit_id],
            )?;
        }
        drop(connection);
        self.prune_audit_records()?;
        Ok(updated)
    }

    pub fn ingest_replicated_audit(&self, mut record: AuditRecord) -> Result<AuditRecord> {
        record.replicated_at_ms =
            Some(record.replicated_at_ms.unwrap_or_else(current_timestamp_ms));
        self.record_audit(record)
    }

    pub fn prune_audit_records(&self) -> Result<usize> {
        let connection = self.connection.lock().expect("state store lock");
        let deleted = connection.execute(
            "DELETE FROM audit_records
             WHERE replicated_at_ms IS NOT NULL
               AND audit_id IN (
                    SELECT audit_id FROM audit_records
                    WHERE replicated_at_ms IS NOT NULL
                    ORDER BY timestamp_ms DESC, audit_id DESC
                    LIMIT -1 OFFSET ?1
               )",
            params![AUDIT_RETENTION_LIMIT as i64],
        )?;
        Ok(deleted)
    }

    pub fn list_events(&self, filter: &EventFilter) -> Result<EventListPage> {
        let mut sql_conditions = Vec::new();
        if filter.category.is_some() {
            sql_conditions.push("category = ?".to_string());
        }
        let where_sql = where_clause(&sql_conditions);
        let connection = self.connection.lock().expect("state store lock");
        let total: i64 = match &filter.category {
            Some(category) => connection.query_row(
                format!("SELECT COUNT(*) FROM events{where_sql}").as_str(),
                params![category],
                |row| row.get(0),
            )?,
            None => connection.query_row(
                format!("SELECT COUNT(*) FROM events{where_sql}").as_str(),
                [],
                |row| row.get(0),
            )?,
        };
        let offset = (filter
            .page
            .saturating_sub(1)
            .saturating_mul(filter.page_size)) as i64;
        let limit = filter.page_size as i64;
        let sql = format!(
            "SELECT id,timestamp_ms,category,message,details_json FROM events{where_sql} ORDER BY id DESC LIMIT ? OFFSET ?"
        );
        let rows = if let Some(category) = &filter.category {
            let mut st = connection.prepare(sql.as_str())?;
            let rows = st.query_map(params![category, limit, offset], map_event)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let mut st = connection.prepare(sql.as_str())?;
            let rows = st.query_map(params![limit, offset], map_event)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(paginated_events(rows, total, filter.page, filter.page_size))
    }

    pub fn start_task_run(
        &self,
        node_id: &str,
        snapshot: &crate::protocol::TaskSnapshot,
        trigger: &str,
        session: &str,
        error_message: Option<String>,
        finished_at_ms: Option<u64>,
    ) -> Result<Option<TaskRunRecord>> {
        let command = snapshot.command.clone();
        let cwd = snapshot.cwd.to_string_lossy().into_owned();
        let connection = self.connection.lock().expect("state store lock");
        let _timestamp = finished_at_ms.unwrap_or_else(current_timestamp_ms);
        connection.execute("INSERT INTO task_runs(node_id,session,task,trigger,status,started_at_ms,finished_at_ms,duration_ms,command,cwd,pid,run_generation,exit_code,error_message) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)", params![node_id,session,snapshot.label,trigger,if finished_at_ms.is_some() {"failed"} else {"running"},snapshot.started_at_ms as i64,finished_at_ms.map(|v| v as i64),None::<i64>,command,cwd,snapshot.pid.map(|v| v as i64),snapshot.run_generation as i64,None::<i64>,error_message])?;
        Ok(Some(TaskRunRecord {
            id: connection.last_insert_rowid() as u64,
            node_id: node_id.to_string(),
            session: session.to_string(),
            task: snapshot.label.clone(),
            trigger: trigger.to_string(),
            status: "running".into(),
            started_at_ms: snapshot.started_at_ms,
            finished_at_ms,
            duration_ms: None,
            command,
            cwd: snapshot.cwd.clone(),
            pid: snapshot.pid,
            run_generation: snapshot.run_generation,
            exit_code: None,
            error_message,
        }))
    }

    pub fn has_task_run(
        &self,
        node_id: &str,
        session: &str,
        task: &str,
        run_generation: u64,
    ) -> Result<bool> {
        let connection = self.connection.lock().expect("state store lock");
        let count:i64=connection.query_row("SELECT COUNT(*) FROM task_runs WHERE node_id=?1 AND session=?2 AND task=?3 AND run_generation=?4",params![node_id,session,task,run_generation as i64],|row|row.get(0))?;
        Ok(count > 0)
    }

    pub fn record_task_run_start(
        &self,
        node_id: &str,
        snapshot: &crate::protocol::TaskSnapshot,
        trigger: &str,
        session: &str,
    ) -> Result<Option<TaskRunRecord>> {
        if self.has_task_run(node_id, session, &snapshot.label, snapshot.run_generation)? {
            return Ok(None);
        }
        self.start_task_run(node_id, snapshot, trigger, session, None, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finish_task_run(
        &self,
        node_id: &str,
        session: &str,
        task: &str,
        run_generation: u64,
        status: &str,
        exit_code: Option<i32>,
        error_message: Option<&str>,
    ) -> Result<bool> {
        let finished = current_timestamp_ms();
        let connection = self.connection.lock().expect("state store lock");
        let updated=connection.execute("WITH target AS (SELECT id FROM task_runs WHERE node_id=?1 AND session=?2 AND task=?3 AND run_generation=?4 AND status='running' ORDER BY id DESC LIMIT 1) UPDATE task_runs SET status=?5,finished_at_ms=?6,duration_ms=(SELECT ?6-started_at_ms FROM task_runs WHERE id IN(SELECT id FROM target)),exit_code=?7,error_message=?8 WHERE id IN(SELECT id FROM target)",params![node_id,session,task,run_generation as i64,status,finished as i64,exit_code,error_message])?;
        Ok(updated == 1)
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

    pub fn record_mcp_call(&self, mut record: McpCallRecord) -> Result<McpCallRecord> {
        let raw_input = record
            .request
            .pointer("/params/arguments")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let input = sanitize_audit_value(&raw_input);
        let session = input
            .get("session")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let task = input
            .get("task")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let searchable_text = build_mcp_search_text(
            &record.tool,
            record.operation.as_deref(),
            record.target_node.as_deref(),
            session.as_deref(),
            task.as_deref(),
            &input,
        );
        record.request = sanitize_audit_value(&record.request);
        record.response = sanitize_audit_value(&record.response);
        let request_json = serde_json::to_string(&record.request)?;
        let response_json = serde_json::to_string(&record.response)?;
        let input_json = serde_json::to_string(&input)?;
        let connection = self.connection.lock().expect("state store lock");
        connection.execute("INSERT INTO mcp_calls(tool,operation,started_at_ms,duration_ms,success,target_node,request_json,response_json,input_json,searchable_text) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", params![record.tool,record.operation,record.started_at_ms as i64,record.duration_ms as i64,i64::from(record.success),record.target_node,request_json,response_json,input_json,searchable_text])?;
        record.id = connection.last_insert_rowid() as u64;
        Ok(record)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn list_mcp_calls(
        &self,
        q: Option<&str>,
        operation: Option<&str>,
        success_only: Option<bool>,
        session: Option<&str>,
        task: Option<&str>,
        page: usize,
        page_size: usize,
    ) -> Result<McpCallListPage> {
        let mut sql_conditions = Vec::<String>::new();
        let mut values = Vec::<rusqlite::types::Value>::new();
        if let Some(q) = q {
            sql_conditions.push("searchable_text LIKE ? ESCAPE '\\'".to_string());
            values.push(format!("%{}%", escape_like(&casefold_search_text(q))).into());
        }
        if let Some(value) = operation {
            sql_conditions.push("operation = ?".to_string());
            values.push(value.to_string().into());
        }
        if let Some(success) = success_only {
            sql_conditions.push("success = ?".to_string());
            values.push(i64::from(success).into());
        }
        if let Some(value) = session {
            sql_conditions.push("json_extract(input_json,'$.session') = ?".to_string());
            values.push(value.to_string().into());
        }
        if let Some(value) = task {
            sql_conditions.push("json_extract(input_json,'$.task') = ?".to_string());
            values.push(value.to_string().into());
        }
        let where_sql = where_clause(&sql_conditions);
        let connection = self.connection.lock().expect("state store lock");
        let total: i64 = connection.query_row(
            format!("SELECT COUNT(*) FROM mcp_calls{where_sql}").as_str(),
            params_from_iter(values.clone()),
            |row| row.get(0),
        )?;
        let offset = (page.saturating_sub(1).saturating_mul(page_size)) as i64;
        values.push((page_size as i64).into());
        values.push(offset.into());
        let sql = format!(
            "SELECT id,tool,operation,started_at_ms,duration_ms,success,target_node,input_json FROM mcp_calls{where_sql} ORDER BY id DESC LIMIT ? OFFSET ?"
        );
        let mut statement = connection.prepare(sql.as_str())?;
        let rows = statement
            .query_map(params_from_iter(values), |row| {
                Ok(McpCallListItem {
                    id: row.get::<_, i64>(0)? as u64,
                    tool: row.get(1)?,
                    operation: row.get(2)?,
                    started_at_ms: row.get::<_, i64>(3)? as u64,
                    duration_ms: row.get::<_, i64>(4)? as u64,
                    success: row.get::<_, i64>(5)? != 0,
                    target_node: row.get(6)?,
                    input: {
                        let json = row.get::<_, String>(7)?;
                        parse_sql_json(json, 7)?
                    },
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(paginated_mcp_calls(rows, total, page, page_size))
    }

    pub fn mcp_call_detail(&self, id: u64) -> Result<Option<McpCallRecord>> {
        let connection = self.connection.lock().expect("state store lock");
        Ok(connection.query_row("SELECT id,tool,operation,started_at_ms,duration_ms,success,target_node,request_json,response_json FROM mcp_calls WHERE id=?1",params![id as i64],|row|{Ok(McpCallRecord{id:row.get::<_,i64>(0)? as u64,tool:row.get(1)?,operation:row.get(2)?,started_at_ms:row.get::<_,i64>(3)? as u64,duration_ms:row.get::<_,i64>(4)? as u64,success:row.get::<_,i64>(5)? != 0,target_node:row.get(6)?,request:{let json=row.get::<_,String>(7)?;parse_sql_json(json,7)?},response:{let json=row.get::<_,String>(8)?;parse_sql_json(json,8)?}})}).optional()?)
    }

    pub fn list_task_runs(&self, filter: &TaskRunFilter) -> Result<TaskRunListPage> {
        let mut sql_conditions = Vec::<String>::new();
        let mut values = Vec::<rusqlite::types::Value>::new();
        if let Some(value) = filter.session.as_deref() {
            sql_conditions.push("session = ?".to_string());
            values.push(value.to_string().into());
        }
        if let Some(value) = filter.task.as_deref() {
            sql_conditions.push("task = ?".to_string());
            values.push(value.to_string().into());
        }
        if let Some(value) = filter.status.as_deref() {
            sql_conditions.push("status = ?".to_string());
            values.push(value.to_string().into());
        }
        if let Some(value) = filter.trigger.as_deref() {
            sql_conditions.push("trigger = ?".to_string());
            values.push(value.to_string().into());
        }
        let where_sql = where_clause(&sql_conditions);
        let connection = self.connection.lock().expect("state store lock");
        let total: i64 = connection.query_row(
            format!("SELECT COUNT(*) FROM task_runs{where_sql}").as_str(),
            params_from_iter(values.clone()),
            |row| row.get(0),
        )?;
        let offset = (filter
            .page
            .saturating_sub(1)
            .saturating_mul(filter.page_size)) as i64;
        values.push((filter.page_size as i64).into());
        values.push(offset.into());
        let sql = format!(
            "SELECT id,node_id,session,task,trigger,status,started_at_ms,finished_at_ms,duration_ms,command,cwd,pid,run_generation,exit_code,error_message FROM task_runs{where_sql} ORDER BY started_at_ms DESC,id DESC LIMIT ? OFFSET ?"
        );
        let mut statement = connection.prepare(sql.as_str())?;
        let rows = statement
            .query_map(params_from_iter(values), map_task_run)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(paginated_task_runs(
            rows,
            total,
            filter.page,
            filter.page_size,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownWorker {
    pub node_id: String,
    pub name: String,
    pub last_seen_ms: u64,
    pub inventory_json: String,
}

fn paginated_mcp_calls(
    items: Vec<McpCallListItem>,
    total: i64,
    page: usize,
    page_size: usize,
) -> McpCallListPage {
    let total = total.max(0) as usize;
    let total_pages = if total == 0 {
        0
    } else {
        total.div_ceil(page_size)
    };
    McpCallListPage {
        items,
        page,
        page_size,
        total,
        total_pages,
        has_next: page < total_pages,
        has_previous: page > 1 && total > 0,
    }
}

fn where_clause(conditions: &[String]) -> String {
    if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    }
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn build_mcp_search_text(
    tool: &str,
    operation: Option<&str>,
    target_node: Option<&str>,
    session: Option<&str>,
    task: Option<&str>,
    input: &serde_json::Value,
) -> String {
    let serialized = serde_json::to_string(input).unwrap_or_default();
    casefold_search_text(&format!(
        "{tool}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{serialized}",
        operation.unwrap_or(""),
        target_node.unwrap_or(""),
        session.unwrap_or(""),
        task.unwrap_or("")
    ))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut out, b| {
            use std::fmt::Write as _;
            let _ = write!(out, "{b:02x}");
            out
        })
}

pub fn hash_access_key(key: &str) -> Result<String> {
    let uuid = Uuid::new_v4();
    let salt = SaltString::encode_b64(uuid.as_bytes())
        .map_err(|error| anyhow::anyhow!("failed to create password salt: {error}"))?;
    Argon2::default()
        .hash_password(key.as_bytes(), &salt)
        .map(|value| value.to_string())
        .map_err(|error| anyhow::anyhow!("failed to hash access key: {error}"))
}

pub fn verify_access_key(candidate: &str, hash: &str) -> bool {
    PasswordHash::new(hash).ok().is_some_and(|parsed| {
        Argon2::default()
            .verify_password(candidate.as_bytes(), &parsed)
            .is_ok()
    })
}

fn read_auth_settings(connection: &Connection) -> Result<AuthSettings> {
    let (enabled, password_hash) = connection.query_row(
        "SELECT enabled,password_hash FROM auth_settings WHERE id=1",
        [],
        |row| Ok((row.get::<_, i64>(0)? != 0, row.get::<_, Option<String>>(1)?)),
    )?;
    Ok(AuthSettings {
        enabled,
        password_hash,
    })
}

fn write_auth_settings(connection: &Connection, settings: &AuthSettings) -> Result<()> {
    connection.execute(
        "UPDATE auth_settings SET enabled=?1,password_hash=?2,updated_at_ms=?3 WHERE id=1",
        params![
            i64::from(settings.enabled),
            settings.password_hash,
            current_timestamp_ms() as i64
        ],
    )?;
    Ok(())
}

fn next_after_schedule(expression: &str, after_ms: u64) -> Result<u64> {
    let fields = expression.split_whitespace().count();
    let normalized = if fields == 5 {
        format!("0 {expression}")
    } else {
        expression.to_string()
    };
    let schedule = normalized
        .parse::<cron::Schedule>()
        .context(format!("invalid cron expression '{expression}'"))?;
    let after_utc = DateTime::<Utc>::from_timestamp_millis(after_ms as i64).unwrap_or(Utc::now());
    let local_after = Local.from_utc_datetime(&after_utc.naive_utc());
    schedule
        .after(&local_after)
        .next()
        .map(|next| next.with_timezone(&Local).timestamp_millis().max(0) as u64)
        .ok_or_else(|| anyhow::anyhow!("cron expression '{expression}' has no future occurrence"))
}

pub fn validate_cron_expression(expression: &str) -> Result<()> {
    if expression.trim().is_empty() {
        bail!("schedule cannot be empty; remove it to disable scheduling")
    }
    next_after_schedule(expression.trim(), current_timestamp_ms())?;
    Ok(())
}

#[allow(dead_code)]
pub fn cron_next_after(expression: &str, after_ms: u64) -> Result<u64> {
    next_after_schedule(expression.trim(), after_ms)
}

fn parse_sql_json(value: String, column: usize) -> rusqlite::Result<serde_json::Value> {
    serde_json::from_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn map_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRecord> {
    let json = row.get::<_, String>(4)?;
    let details = serde_json::from_str(&json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(EventRecord {
        id: row.get::<_, i64>(0)? as u64,
        timestamp_ms: row.get::<_, i64>(1)? as u64,
        category: row.get(2)?,
        message: row.get(3)?,
        details,
    })
}

fn map_task_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRunRecord> {
    Ok(TaskRunRecord {
        id: row.get::<_, i64>(0)? as u64,
        node_id: row.get(1)?,
        session: row.get(2)?,
        task: row.get(3)?,
        trigger: row.get(4)?,
        status: row.get(5)?,
        started_at_ms: row.get::<_, i64>(6)? as u64,
        finished_at_ms: row.get::<_, Option<i64>>(7)?.map(|v| v as u64),
        duration_ms: row.get::<_, Option<i64>>(8)?.map(|v| v as u64),
        command: row.get(9)?,
        cwd: PathBuf::from(row.get::<_, String>(10)?),
        pid: row.get::<_, Option<i64>>(11)?.map(|v| v as u32),
        run_generation: row.get::<_, i64>(12)? as u64,
        exit_code: row.get(13)?,
        error_message: row.get(14)?,
    })
}

fn paginated_task_runs(
    items: Vec<TaskRunRecord>,
    total: i64,
    page: usize,
    page_size: usize,
) -> TaskRunListPage {
    let total = total.max(0) as usize;
    let total_pages = if total == 0 {
        0
    } else {
        total.div_ceil(page_size)
    };
    TaskRunListPage {
        items,
        page,
        page_size,
        total,
        total_pages,
        has_next: page < total_pages,
        has_previous: page > 1 && total > 0,
    }
}

fn paginated_audit(
    items: Vec<AuditListItem>,
    total: i64,
    page: usize,
    page_size: usize,
) -> AuditListPage {
    let total = total.max(0) as usize;
    let total_pages = if total == 0 {
        0
    } else {
        total.div_ceil(page_size)
    };
    AuditListPage {
        items,
        page,
        page_size,
        total,
        total_pages,
        has_next: page < total_pages,
        has_previous: page > 1 && total > 0,
    }
}

fn build_audit_search_text(
    record: &AuditRecord,
    request_json: &str,
    response_json: &str,
    details_json: &str,
) -> String {
    casefold_search_text(&format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        record.source.as_str(),
        record.transport.as_str(),
        record.origin_node_id.as_deref().unwrap_or(""),
        record.executor_node_id.as_deref().unwrap_or(""),
        record.request_kind,
        record.operation,
        record.session.as_deref().unwrap_or(""),
        record.task.as_deref().unwrap_or(""),
        record.status.as_str(),
        record.error.as_deref().unwrap_or(""),
        request_json,
        response_json,
        details_json,
    ))
}

fn parse_audit_source(value: String) -> rusqlite::Result<AuditSource> {
    AuditSource::parse(&value).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid audit source '{value}'"),
            )),
        )
    })
}

fn parse_audit_status(value: String) -> rusqlite::Result<AuditStatus> {
    AuditStatus::parse(&value).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid audit status '{value}'"),
            )),
        )
    })
}

fn parse_audit_transport(value: String) -> rusqlite::Result<AuditTransport> {
    AuditTransport::parse(&value).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid audit transport '{value}'"),
            )),
        )
    })
}

fn map_audit_list_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditListItem> {
    Ok(AuditListItem {
        audit_id: row.get(0)?,
        correlation_id: row.get(1)?,
        timestamp_ms: row.get::<_, i64>(2)? as u64,
        duration_ms: row.get::<_, i64>(3)? as u64,
        source: parse_audit_source(row.get(4)?)?,
        transport: parse_audit_transport(row.get(5)?)?,
        origin_node_id: row.get(6)?,
        executor_node_id: row.get(7)?,
        request_kind: row.get(8)?,
        operation: row.get(9)?,
        session: row.get(10)?,
        task: row.get(11)?,
        status: parse_audit_status(row.get(12)?)?,
        success: row.get::<_, i64>(13)? != 0,
        error: row.get(14)?,
        replicated_at_ms: row.get::<_, Option<i64>>(15)?.map(|value| value as u64),
    })
}

fn map_audit_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditRecord> {
    Ok(AuditRecord {
        audit_id: row.get(0)?,
        correlation_id: row.get(1)?,
        timestamp_ms: row.get::<_, i64>(2)? as u64,
        duration_ms: row.get::<_, i64>(3)? as u64,
        source: parse_audit_source(row.get(4)?)?,
        transport: parse_audit_transport(row.get(5)?)?,
        origin_node_id: row.get(6)?,
        executor_node_id: row.get(7)?,
        request_kind: row.get(8)?,
        operation: row.get(9)?,
        session: row.get(10)?,
        task: row.get(11)?,
        status: parse_audit_status(row.get(12)?)?,
        success: row.get::<_, i64>(13)? != 0,
        error: row.get(14)?,
        request: {
            let json = row.get::<_, String>(15)?;
            parse_sql_json(json, 15)?
        },
        response: {
            let json = row.get::<_, String>(16)?;
            parse_sql_json(json, 16)?
        },
        details: {
            let json = row.get::<_, String>(17)?;
            parse_sql_json(json, 17)?
        },
        replicated_at_ms: row.get::<_, Option<i64>>(18)?.map(|value| value as u64),
    })
}

fn paginated_events(
    items: Vec<EventRecord>,
    total: i64,
    page: usize,
    page_size: usize,
) -> EventListPage {
    let total = total.max(0) as usize;
    let total_pages = if total == 0 {
        0
    } else {
        total.div_ceil(page_size)
    };
    EventListPage {
        items,
        page,
        page_size,
        total,
        total_pages,
        has_next: page < total_pages,
        has_previous: page > 1 && total > 0,
    }
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

    #[test]
    fn schema_two_migrates_and_preserves_registrations() {
        let dir = tempfile::tempdir().unwrap();
        {
            let connection = rusqlite::Connection::open(dir.path().join("state.db")).unwrap();
            connection.execute_batch("PRAGMA foreign_keys=ON;
             CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
             CREATE TABLE registrations(session TEXT PRIMARY KEY,project TEXT NOT NULL,registered_at_ms INTEGER NOT NULL);
             CREATE TABLE workers(node_id TEXT PRIMARY KEY,name TEXT NOT NULL,last_seen_ms INTEGER NOT NULL,inventory_json TEXT NOT NULL);").unwrap();
            connection
                .execute("INSERT INTO registrations VALUES ('api','/tmp/api',1)", [])
                .unwrap();
            connection
                .execute("INSERT INTO metadata VALUES ('schema_version','2')", [])
                .unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            connection
                .execute("INSERT INTO metadata VALUES ('node_id',?1)", [&id])
                .unwrap();
        }
        let store = StateStore::open(dir.path()).unwrap();
        assert_eq!(store.registrations().unwrap()[0].session, "api");
        let ids = store
            .list_task_runs(&TaskRunFilter {
                session: None,
                task: None,
                status: None,
                trigger: None,
                page: 1,
                page_size: 20,
            })
            .unwrap()
            .total;
        assert_eq!(ids, 0);
    }

    #[test]
    fn task_runs_and_mcp_calls_survive_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let node_id = StateStore::open(dir.path())
            .unwrap()
            .node_settings()
            .unwrap()
            .node_id;
        let snapshot = crate::protocol::TaskSnapshot {
            label: "cleanup".into(),
            status: TaskStatus::Exited,
            pid: Some(9),
            command: "echo done".into(),
            cwd: PathBuf::from("/tmp"),
            auto_start: false,
            last_exit: Some("exit status: 0".into()),
            exit_code: Some(0),
            logs: vec![],
            run_generation: 2,
            started_at_ms: 42,
            schedule: Some("* * * * *".into()),
            service: Default::default(),
        };
        {
            let store = StateStore::open(dir.path()).unwrap();
            store
                .record_task_run_start(&node_id, &snapshot, "cron", "demo")
                .unwrap();
            assert!(
                store
                    .finish_task_run(&node_id, "demo", "cleanup", 2, "exited", Some(0), None)
                    .unwrap()
            );
            store.record_mcp_call(McpCallRecord{id:0,tool:"taskdeck_control".into(),operation:Some("start".into()),started_at_ms:99,duration_ms:5,success:true,target_node:Some("self".into()),request:serde_json::json!({"params":{"arguments":{"session":"demo","needle":"unique-request"}}}),response:serde_json::json!({"ok":true})}).unwrap();
            store
                .record_event(
                    "scheduler",
                    "scheduler started",
                    serde_json::json!({"caught_up":false}),
                )
                .unwrap();
        }
        let store = StateStore::open(dir.path()).unwrap();
        let runs = store
            .list_task_runs(&TaskRunFilter {
                session: Some("demo".into()),
                task: None,
                status: None,
                trigger: None,
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(runs.items.len(), 1);
        assert_eq!(runs.items[0].status, "exited");
        assert!(runs.items[0].duration_ms.is_some());
        let calls = store
            .list_mcp_calls(Some("unique-request"), None, None, None, None, 1, 20)
            .unwrap();
        assert_eq!(calls.items.len(), 1);
        assert_eq!(calls.total, 1);
        assert_eq!(
            store
                .mcp_call_detail(calls.items[0].id)
                .unwrap()
                .unwrap()
                .tool,
            "taskdeck_control"
        );
        assert_eq!(
            store
                .list_events(&EventFilter {
                    category: None,
                    page: 1,
                    page_size: 20
                })
                .unwrap()
                .items
                .len(),
            1
        );
    }

    #[test]
    fn cron_validation_rejects_bad_expressions_and_calculates_a_future_occurrence() {
        validate_cron_expression("*/10 * * * *").unwrap();
        validate_cron_expression("*/5 * * * * *").unwrap();
        assert!(validate_cron_expression("not-a-cron").is_err());
        assert!(validate_cron_expression("").is_err());
        assert!(cron_next_after("* * * * *", current_timestamp_ms()).is_ok());
    }

    #[test]
    fn access_keys_use_argon2id_and_sessions_are_hashed() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::open(dir.path()).unwrap();
        let key = "correct-horse-battery-staple";
        store.set_access_key(key).unwrap();
        store.configure_auth(true).unwrap();
        assert!(verify_access_key(
            key,
            &store.auth_settings().unwrap().password_hash.unwrap()
        ));
        assert!(!verify_access_key(
            "wrong",
            &store.auth_settings().unwrap().password_hash.unwrap()
        ));
        assert!(
            !store
                .auth_settings()
                .unwrap()
                .password_hash
                .as_ref()
                .unwrap()
                .contains(key)
        );
        let token = store.create_auth_session().unwrap();
        assert!(token.len() > 32);
        assert!(store.valid_auth_session(Some(&token)));
        store.delete_auth_session(Some(&token));
        assert!(!store.valid_auth_session(Some(&token)));
        store.configure_auth(false).unwrap();
        assert!(store.auth_settings().unwrap().password_hash.is_none());
    }

    fn sample_audit(audit_id: &str, replicated: bool) -> AuditRecord {
        AuditRecord {
            audit_id: audit_id.to_string(),
            correlation_id: format!("corr-{audit_id}"),
            timestamp_ms: 100,
            duration_ms: 5,
            source: AuditSource::Cli,
            transport: AuditTransport::Ipc,
            origin_node_id: Some("worker-1".into()),
            executor_node_id: Some("worker-1".into()),
            request_kind: "action".into(),
            operation: "start".into(),
            session: Some("demo".into()),
            task: Some("api".into()),
            status: AuditStatus::Success,
            success: true,
            error: None,
            request: serde_json::json!({"type":"action","token":"secret"}),
            response: serde_json::json!({"ok":true}),
            details: serde_json::json!({}),
            replicated_at_ms: replicated.then_some(200),
        }
    }

    #[test]
    fn audit_records_are_idempotent_and_survive_reopening() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = StateStore::open(dir.path()).unwrap();
            store.record_audit(sample_audit("audit-1", false)).unwrap();
            store.record_audit(sample_audit("audit-1", false)).unwrap();
            let page = store
                .list_audit(&AuditFilter {
                    q: Some("action".into()),
                    source: None,
                    status: None,
                    node: Some("worker-1".into()),
                    session: Some("demo".into()),
                    task: None,
                    operation: Some("start".into()),
                    page: 1,
                    page_size: 20,
                })
                .unwrap();
            assert_eq!(page.total, 1);
            let detail = store.audit_detail("audit-1").unwrap().unwrap();
            assert_eq!(detail.request["token"], "[REDACTED]");
            assert!(detail.replicated_at_ms.is_none());
        }
        let reopened = StateStore::open(dir.path()).unwrap();
        assert_eq!(
            reopened.audit_detail("audit-1").unwrap().unwrap().operation,
            "start"
        );
    }

    #[test]
    fn audit_retention_keeps_unreplicated_records() {
        let store = StateStore::open_in_memory().unwrap();
        for index in 0..(AUDIT_RETENTION_LIMIT + 5) {
            let mut record = sample_audit(&format!("kept-{index}"), true);
            record.timestamp_ms = index as u64;
            store.record_audit(record).unwrap();
        }
        let mut unreplicated = sample_audit("pending", false);
        unreplicated.timestamp_ms = 0;
        store.record_audit(unreplicated).unwrap();
        let unreplicated = store.unreplicated_audit_records(20).unwrap();
        assert_eq!(unreplicated.len(), 1);
        assert_eq!(unreplicated[0].audit_id, "pending");
        let page = store
            .list_audit(&AuditFilter {
                q: None,
                source: None,
                status: None,
                node: None,
                session: None,
                task: None,
                operation: None,
                page: 1,
                page_size: 100,
            })
            .unwrap();
        assert_eq!(page.total, AUDIT_RETENTION_LIMIT + 1);
        store
            .mark_audit_replicated(&["pending".to_string()], 9_999)
            .unwrap();
        let page = store
            .list_audit(&AuditFilter {
                q: None,
                source: None,
                status: None,
                node: None,
                session: None,
                task: None,
                operation: None,
                page: 1,
                page_size: 100,
            })
            .unwrap();
        assert_eq!(page.total, AUDIT_RETENTION_LIMIT);
        assert!(store.audit_detail("pending").unwrap().is_none());
    }
}
