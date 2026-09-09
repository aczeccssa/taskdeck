//! Workspace registration persistence.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rusqlite::params;

use super::util::*;
use super::StateStore;

impl StateStore {
    pub fn registrations(&self) -> Result<Vec<Registration>> {
        let connection = self.connection.lock().expect("state store lock");
        let mut statement = connection.prepare(
            "SELECT session, alias, project, registered_at_ms FROM registrations ORDER BY session",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Registration {
                session: row.get(0)?,
                alias: row.get(1)?,
                project: PathBuf::from(row.get::<_, String>(2)?),
                registered_at_ms: row.get::<_, i64>(3)? as u64,
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

    pub fn set_registration_alias(&self, session: &str, alias: Option<&str>) -> Result<()> {
        let alias = alias
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let connection = self.connection.lock().expect("state store lock");
        let changed = connection.execute(
            "UPDATE registrations SET alias=?2 WHERE session=?1",
            params![session, alias],
        )?;
        if changed == 0 {
            bail!("session '{session}' is not registered");
        }
        Ok(())
    }

    pub fn workspace_summaries(&self) -> Result<Vec<crate::protocol::WorkspaceSummary>> {
        Ok(self
            .registrations()?
            .into_iter()
            .map(|registration| {
                let display_name = registration
                    .alias
                    .clone()
                    .unwrap_or_else(|| registration.session.clone());
                crate::protocol::WorkspaceSummary {
                    session: registration.session,
                    alias: registration.alias,
                    display_name,
                    project: registration.project,
                }
            })
            .collect())
    }

}

impl StateStore {
    pub fn remove_registration(&self, session: &str) -> Result<bool> {
        let connection = self.connection.lock().expect("state store lock");
        Ok(connection.execute(
            "DELETE FROM registrations WHERE session=?1",
            params![session],
        )? > 0)
    }

}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    pub session: String,
    pub alias: Option<String>,
    pub project: PathBuf,
    pub registered_at_ms: u64,
}
