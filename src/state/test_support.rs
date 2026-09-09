//! Test-only in-memory store constructor.

use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::StateStore;

impl StateStore {
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
                 alias TEXT,
                 project TEXT NOT NULL,
                 registered_at_ms INTEGER NOT NULL
             );
             CREATE UNIQUE INDEX idx_registrations_alias
                 ON registrations(alias) WHERE alias IS NOT NULL;
             CREATE TABLE workers (
                 node_id TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 last_seen_ms INTEGER NOT NULL,
                 inventory_json TEXT NOT NULL
             );
             CREATE TABLE workflow_groups (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL UNIQUE,
                 created_at_ms INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE workflow_group_members (
                 group_id TEXT NOT NULL,
                 position INTEGER NOT NULL,
                 node_id TEXT NOT NULL,
                 session TEXT NOT NULL,
                 task TEXT NOT NULL,
                 PRIMARY KEY(group_id, position),
                 UNIQUE(group_id, node_id, session, task),
                 FOREIGN KEY(group_id) REFERENCES workflow_groups(id) ON DELETE CASCADE
             );
             CREATE INDEX idx_workflow_group_members_target
                 ON workflow_group_members(node_id, session, task);
             CREATE TABLE boards (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL UNIQUE,
                 created_at_ms INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE board_cards (
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
             CREATE INDEX idx_board_cards_target
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
        let store = Self {
            connection: Mutex::new(connection),
        };
        store.initialize()?;
        Ok(store)
    }
}
