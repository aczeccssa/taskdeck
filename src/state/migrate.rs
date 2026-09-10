//! Schema migrations: column backfills and version bumping.

use anyhow::{Context, Result, bail};

use super::StateStore;
use rusqlite::Connection;
use uuid::Uuid;

use super::schema::SCHEMA_VERSION;
use super::util::{get_metadata, set_metadata};
use super::{DEFAULT_BIND_HOST, DEFAULT_WEB_PORT};
use crate::protocol::{LeaderMode, NodeRole};

impl StateStore {
    pub(super) fn initialize(&self) -> Result<()> {
        let connection = self.connection.lock().expect("state store lock");
        ensure_registration_alias_column(&connection)?;
        ensure_workflow_graph_column(&connection)?;
        let version = get_metadata(&connection, "schema_version")?;
        match version.as_deref() {
            None => set_metadata(&connection, "schema_version", SCHEMA_VERSION)?,
            Some("1" | "2" | "3" | "4" | "5" | "6" | "7") => {
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
        connection.execute_batch(&format!("PRAGMA user_version={SCHEMA_VERSION}"))?;
        Ok(())
    }
}

pub(super) fn ensure_registration_alias_column(connection: &Connection) -> Result<()> {
    let exists: bool = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('registrations') WHERE name='alias'",
            [],
            |row| row.get::<_, i64>(0).map(|count| count > 0),
        )
        .context("failed to inspect registrations schema")?;
    if !exists {
        connection
            .execute("DROP INDEX IF EXISTS idx_registrations_alias", [])
            .context("failed to remove stale workspace alias index")?;
        connection
            .execute("ALTER TABLE registrations ADD COLUMN alias TEXT", [])
            .context("failed to add workspace alias column")?;
    }
    connection.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_registrations_alias
             ON registrations(alias) WHERE alias IS NOT NULL;",
    )?;
    Ok(())
}

pub(super) fn ensure_workflow_graph_column(connection: &Connection) -> Result<()> {
    let exists: bool = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('workflow_groups') WHERE name='graph_json'",
            [],
            |row| row.get::<_, i64>(0).map(|count| count > 0),
        )
        .context("failed to inspect workflow_groups schema")?;
    if !exists {
        connection
            .execute("ALTER TABLE workflow_groups ADD COLUMN graph_json TEXT", [])
            .context("failed to add workflow graph column")?;
    }
    Ok(())
}
