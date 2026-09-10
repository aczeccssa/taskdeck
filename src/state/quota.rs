//! Workspace quota persistence.

use anyhow::{Context, Result, bail};
use rusqlite::params;
use uuid::Uuid;

use super::StateStore;
use super::util::*;
use crate::protocol::*;

impl StateStore {
    pub fn quotas(&self) -> Result<Vec<WorkspaceQuota>> {
        let connection = self.connection.lock().expect("state store lock");
        let mut statement = connection.prepare(
            "SELECT id, node_id, session, max_running_tasks, created_at_ms, updated_at_ms
             FROM workspace_quotas
             ORDER BY session IS NOT NULL, session COLLATE NOCASE, created_at_ms, id",
        )?;
        let quotas = statement
            .query_map([], |row| {
                Ok(WorkspaceQuota {
                    id: row.get(0)?,
                    node_id: row.get(1)?,
                    session: row.get(2)?,
                    max_running_tasks: row.get::<_, i64>(3)? as u32,
                    created_at_ms: row.get::<_, i64>(4)? as u64,
                    updated_at_ms: row.get::<_, i64>(5)? as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(quotas)
    }

    pub fn create_quota(
        &self,
        node_id: &str,
        input: WorkspaceQuotaInput,
    ) -> Result<WorkspaceQuota> {
        let session = normalize_quota_session(input.session)?;
        if input.max_running_tasks == 0 {
            bail!("quota must allow at least one running task");
        }
        if self
            .quotas()?
            .iter()
            .any(|quota| quota.node_id == node_id && quota.session == session)
        {
            bail!("a quota already exists for this scope");
        }
        let now = current_timestamp_ms();
        let id = Uuid::new_v4().to_string();
        let connection = self.connection.lock().expect("state store lock");
        connection
            .execute(
                "INSERT INTO workspace_quotas(id, node_id, session, max_running_tasks, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, node_id, session, input.max_running_tasks as i64, now as i64, now as i64],
            )
            .with_context(|| format!("failed to create quota '{id}'"))?;
        Ok(WorkspaceQuota {
            id,
            node_id: node_id.to_string(),
            session,
            max_running_tasks: input.max_running_tasks,
            created_at_ms: now,
            updated_at_ms: now,
        })
    }

    pub fn update_quota(&self, id: &str, input: WorkspaceQuotaInput) -> Result<WorkspaceQuota> {
        let session = normalize_quota_session(input.session)?;
        if input.max_running_tasks == 0 {
            bail!("quota must allow at least one running task");
        }
        let now = current_timestamp_ms();
        {
            let connection = self.connection.lock().expect("state store lock");
            let changed = connection.execute(
                "UPDATE workspace_quotas SET session=?2, max_running_tasks=?3, updated_at_ms=?4 WHERE id=?1",
                params![id, session, input.max_running_tasks as i64, now as i64],
            )?;
            if changed == 0 {
                bail!("quota '{id}' not found");
            }
            let duplicate = connection
                .query_row(
                    "SELECT COUNT(*) FROM workspace_quotas WHERE node_id=(SELECT node_id FROM workspace_quotas WHERE id=?1) AND session IS ?2 AND id != ?1",
                    params![id, session],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0);
            if duplicate > 0 {
                bail!("a quota already exists for this scope");
            }
        }
        self.quotas()?
            .into_iter()
            .find(|quota| quota.id == id)
            .with_context(|| format!("quota '{id}' disappeared after update"))
    }

    pub fn delete_quota(&self, id: &str) -> Result<bool> {
        let connection = self.connection.lock().expect("state store lock");
        Ok(connection.execute("DELETE FROM workspace_quotas WHERE id=?1", params![id])? > 0)
    }
}

pub(super) fn normalize_quota_session(session: Option<String>) -> Result<Option<String>> {
    match session {
        Some(session) => {
            let session = session.trim();
            if session.is_empty() {
                Ok(None)
            } else {
                Ok(Some(session.to_string()))
            }
        }
        None => Ok(None),
    }
}
