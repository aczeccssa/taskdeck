//! Database open, DDL and schema migrations.

use std::fs::{self, File, OpenOptions};
use std::path::Path;
use std::sync::Mutex;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use anyhow::{anyhow, Context, Result, bail};
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags, OptionalExtension};

use super::StateStore;

const DATABASE_FILE: &str = "state.db";
pub(crate) const SCHEMA_VERSION: &str = "8";

impl StateStore {
    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root).with_context(|| format!("failed to create {}", root.display()))?;
        let database = root.join(DATABASE_FILE);
        let initial_version = probe_existing_database(&database, false)?;
        let needs_migration_lock = !database.exists()
            || initial_version.is_none()
            || initial_version.is_some_and(|version| version < current_schema_version());
        let _migration_lock = needs_migration_lock
            .then(|| acquire_migration_lock(root))
            .transpose()?;
        let legacy_version = if needs_migration_lock {
            probe_existing_database(&database, true)?
        } else {
            initial_version
        };
        let connection = Connection::open(&database)
            .with_context(|| format!("failed to open {}/{}", root.display(), DATABASE_FILE))?;
        restrict_database_permissions(&database)?;
        if let Some(version) = legacy_version.filter(|version| *version < current_schema_version())
        {
            create_backup(&connection, root, version)?;
        }
        if needs_migration_lock {
            connection.execute_batch(
                "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS metadata (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS registrations (
                 session TEXT PRIMARY KEY,
                 alias TEXT,
                 project TEXT NOT NULL,
                 registered_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS workers (
                 node_id TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 last_seen_ms INTEGER NOT NULL,
                 inventory_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS workflow_groups (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL UNIQUE,
                 created_at_ms INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS workflow_group_members (
                 group_id TEXT NOT NULL,
                 position INTEGER NOT NULL,
                 node_id TEXT NOT NULL,
                 session TEXT NOT NULL,
                 task TEXT NOT NULL,
                 PRIMARY KEY(group_id, position),
                 UNIQUE(group_id, node_id, session, task),
                 FOREIGN KEY(group_id) REFERENCES workflow_groups(id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_workflow_group_members_target
                 ON workflow_group_members(node_id, session, task);
             CREATE TABLE IF NOT EXISTS boards (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL UNIQUE,
                 created_at_ms INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS board_cards (
                 board_id TEXT NOT NULL,
                 position INTEGER NOT NULL,
                 card_id TEXT NOT NULL,
                 node_id TEXT NOT NULL,
                 session TEXT NOT NULL,
                 task TEXT NOT NULL,
                 mode TEXT NOT NULL,
                 pinned INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY(board_id, position),
                 FOREIGN KEY(board_id) REFERENCES boards(id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_board_cards_target
                 ON board_cards(node_id, session, task);
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
             CREATE INDEX IF NOT EXISTS idx_auth_sessions_expiry ON auth_sessions(expires_at_ms);
             CREATE TABLE IF NOT EXISTS workflow_revisions (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 group_id TEXT NOT NULL,
                 revision INTEGER NOT NULL,
                 snapshot_json TEXT NOT NULL,
                 note TEXT,
                 created_at_ms INTEGER NOT NULL,
                 UNIQUE(group_id, revision)
             );
             CREATE INDEX IF NOT EXISTS idx_workflow_revisions_group
                 ON workflow_revisions(group_id, revision DESC);
             CREATE TABLE IF NOT EXISTS workspace_quotas (
                 id TEXT PRIMARY KEY,
                 node_id TEXT NOT NULL,
                 session TEXT,
                 max_running_tasks INTEGER NOT NULL,
                 created_at_ms INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS notification_rules (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 event_types_json TEXT NOT NULL,
                 scope_session TEXT,
                 scope_task TEXT,
                 webhook_url TEXT,
                 enabled INTEGER NOT NULL DEFAULT 1,
                 created_at_ms INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS notifications (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 rule_id TEXT,
                 rule_name TEXT,
                 event_type TEXT NOT NULL,
                 severity TEXT NOT NULL,
                 node_id TEXT NOT NULL,
                 session TEXT,
                 task TEXT,
                 title TEXT NOT NULL,
                 message TEXT NOT NULL,
                 details_json TEXT NOT NULL DEFAULT '{}',
                 read INTEGER NOT NULL DEFAULT 0,
                 created_at_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_notifications_recent ON notifications(created_at_ms DESC);
             CREATE TABLE IF NOT EXISTS api_tokens (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 token_hash TEXT NOT NULL UNIQUE,
                 token_prefix TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL,
                 last_used_at_ms INTEGER,
                 revoked INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS board_templates (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL UNIQUE,
                 description TEXT,
                 cards_json TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS task_dependencies (
                 id TEXT PRIMARY KEY,
                 node_id TEXT NOT NULL,
                 session TEXT NOT NULL,
                 task TEXT NOT NULL,
                 depends_node_id TEXT NOT NULL,
                 depends_session TEXT NOT NULL,
                 depends_task TEXT NOT NULL,
                 required_state TEXT NOT NULL DEFAULT 'running',
                 created_at_ms INTEGER NOT NULL,
                 UNIQUE(node_id, session, task, depends_node_id, depends_session, depends_task)
             );
             CREATE INDEX IF NOT EXISTS idx_task_dependencies_target
                 ON task_dependencies(depends_node_id, depends_session, depends_task);
             CREATE TABLE IF NOT EXISTS scaling_policies (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 enabled INTEGER NOT NULL DEFAULT 1,
                 watch_node_id TEXT NOT NULL,
                 watch_session TEXT NOT NULL,
                 watch_task TEXT NOT NULL,
                 metric TEXT NOT NULL,
                 scale_out_threshold REAL NOT NULL,
                 scale_in_threshold REAL NOT NULL,
                 scale_out_node_id TEXT NOT NULL,
                 scale_out_session TEXT NOT NULL,
                 scale_out_task TEXT NOT NULL,
                 cooldown_seconds INTEGER NOT NULL DEFAULT 300,
                 last_action TEXT,
                 last_action_ms INTEGER,
                 created_at_ms INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL
                 );",
            )?;
        } else {
            // The current schema is already initialized; only set this
            // connection-local pragma without touching database metadata.
            connection.execute_batch("PRAGMA foreign_keys=ON;")?;
        }
        restrict_database_permissions(&database)?;
        restrict_database_sidecar_permissions(&database)?;
        let store = Self {
            connection: Mutex::new(connection),
            root: Some(root.to_path_buf()),
        };
        // A current database has already completed all schema/metadata
        // initialization. Avoid mutating it on every CLI open so concurrent
        // current-schema readers do not race through migration code.
        if needs_migration_lock {
            store.initialize()?;
        }
        Ok(store)
    }

    pub fn integrity_check(&self) -> Result<()> {
        let connection = self.connection.lock().expect("state store lock");
        let integrity: String =
            connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if integrity == "ok" {
            Ok(())
        } else {
            bail!("state database failed integrity check: {integrity}");
        }
    }
}

fn restrict_database_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(path)
            .with_context(|| format!("failed to inspect {}", path.display()))?
            .permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("failed to restrict permissions on {}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn restrict_database_sidecar_permissions(path: &Path) -> Result<()> {
    for suffix in ["-wal", "-shm"] {
        let sidecar_path = format!("{}{suffix}", path.display());
        let sidecar = Path::new(&sidecar_path);
        if sidecar.exists() {
            restrict_database_permissions(sidecar)?;
        }
    }
    Ok(())
}

fn current_schema_version() -> u32 {
    SCHEMA_VERSION.parse().expect("schema version is numeric")
}

fn acquire_migration_lock(root: &Path) -> Result<File> {
    let path = root.join("state-migration.lock");
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let lock = options
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    lock.try_lock_exclusive().map_err(|error| {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            anyhow!(
                "state migration is already in progress for {}; retry shortly",
                root.display()
            )
        } else {
            anyhow!(error).context(format!("failed to acquire migration lock {}", path.display()))
        }
    })?;
    restrict_database_permissions(&path)?;
    Ok(lock)
}

fn probe_existing_database(path: &Path, verify_integrity: bool) -> Result<Option<u32>> {
    if !path.exists() {
        return Ok(None);
    }
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open {} read-only", path.display()))?;
    if verify_integrity {
        let integrity: String =
            connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            bail!(
                "state database {} failed integrity check: {integrity}",
                path.display()
            );
        }
    }
    let object_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('table', 'index', 'view', 'trigger')",
        [],
        |row| row.get(0),
    )?;
    if object_count == 0 {
        return Ok(None);
    }
    let user_version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let current = current_schema_version();
    if user_version > current {
        bail!(
            "state database user_version {user_version} is newer than supported version {current}"
        );
    }
    let has_metadata: bool = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='metadata'",
        [],
        |row| row.get::<_, i64>(0).map(|count| count > 0),
    )?;
    if !has_metadata {
        bail!(
            "state database {} has data but no metadata table",
            path.display()
        );
    }
    let metadata_version: Option<String> = connection
        .query_row(
            "SELECT value FROM metadata WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let metadata_version = metadata_version
        .with_context(|| {
            format!(
                "state database {} is missing schema_version",
                path.display()
            )
        })?
        .parse::<u32>()
        .with_context(|| {
            format!(
                "state database {} has invalid schema_version",
                path.display()
            )
        })?;
    if metadata_version > current {
        bail!(
            "state database schema_version {metadata_version} is newer than supported version {current}"
        );
    }
    if user_version != 0 && user_version != metadata_version {
        bail!(
            "state database version conflict: user_version {user_version} does not match metadata schema_version {metadata_version}"
        );
    }
    Ok(Some(metadata_version))
}

fn create_backup(connection: &Connection, root: &Path, version: u32) -> Result<()> {
    let backup = root.join(format!("state.db.bak-v{version}.sqlite"));
    if backup.exists() {
        return Ok(());
    }
    let destination = backup.to_string_lossy().replace('\'', "''");
    connection
        .execute_batch(&format!("VACUUM INTO '{destination}'"))
        .with_context(|| format!("failed to create migration backup {}", backup.display()))?;
    restrict_database_permissions(&backup)?;
    let backup_connection = Connection::open_with_flags(&backup, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let integrity: String =
        backup_connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        bail!(
            "migration backup {} failed integrity check: {integrity}",
            backup.display()
        );
    }
    Ok(())
}
