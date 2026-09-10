//! Task dependency persistence.

use anyhow::{Result, bail};
use rusqlite::params;
use uuid::Uuid;

use super::StateStore;
use super::util::*;
use crate::protocol::*;

impl StateStore {
    pub fn task_dependencies(&self) -> Result<Vec<TaskDependency>> {
        let connection = self.connection.lock().expect("state store lock");
        let mut statement = connection.prepare(
            "SELECT id, node_id, session, task, depends_node_id, depends_session, depends_task, required_state, created_at_ms
             FROM task_dependencies
             ORDER BY session COLLATE NOCASE, task COLLATE NOCASE, created_at_ms, id",
        )?;
        let dependencies = statement
            .query_map([], |row| {
                Ok(TaskDependency {
                    id: row.get(0)?,
                    node_id: row.get(1)?,
                    session: row.get(2)?,
                    task: row.get(3)?,
                    depends_node_id: row.get(4)?,
                    depends_session: row.get(5)?,
                    depends_task: row.get(6)?,
                    required_state: row.get(7)?,
                    created_at_ms: row.get::<_, i64>(8)? as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(dependencies)
    }

    pub fn create_task_dependency(&self, input: TaskDependencyInput) -> Result<TaskDependency> {
        let input = normalize_task_dependency_input(input)?;
        let now = current_timestamp_ms();
        let id = Uuid::new_v4().to_string();
        let connection = self.connection.lock().expect("state store lock");
        connection
            .execute(
                "INSERT INTO task_dependencies(id, node_id, session, task, depends_node_id, depends_session, depends_task, required_state, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id,
                    input.node_id,
                    input.session,
                    input.task,
                    input.depends_node_id,
                    input.depends_session,
                    input.depends_task,
                    input.required_state,
                    now as i64
                ],
            )
            .map_err(|error| {
                if format!("{error}").contains("UNIQUE") {
                    anyhow::Error::msg("this dependency already exists")
                } else {
                    anyhow::Error::new(error).context("failed to create task dependency")
                }
            })?;
        Ok(TaskDependency {
            id,
            node_id: input.node_id,
            session: input.session,
            task: input.task,
            depends_node_id: input.depends_node_id,
            depends_session: input.depends_session,
            depends_task: input.depends_task,
            required_state: input
                .required_state
                .unwrap_or_else(|| "running".to_string()),
            created_at_ms: now,
        })
    }

    pub fn delete_task_dependency(&self, id: &str) -> Result<bool> {
        let connection = self.connection.lock().expect("state store lock");
        Ok(connection.execute("DELETE FROM task_dependencies WHERE id=?1", params![id])? > 0)
    }

    pub fn dependencies_for_task(
        &self,
        node_id: &str,
        session: &str,
        task: &str,
    ) -> Result<Vec<TaskDependency>> {
        Ok(self
            .task_dependencies()?
            .into_iter()
            .filter(|dependency| {
                dependency.node_id == node_id
                    && dependency.session == session
                    && dependency.task == task
            })
            .collect())
    }
}

pub(super) fn normalize_task_dependency_input(
    mut input: TaskDependencyInput,
) -> Result<TaskDependencyInput> {
    input.node_id = input.node_id.trim().to_string();
    input.session = input.session.trim().to_string();
    input.task = input.task.trim().to_string();
    input.depends_node_id = input.depends_node_id.trim().to_string();
    input.depends_session = input.depends_session.trim().to_string();
    input.depends_task = input.depends_task.trim().to_string();
    if input.node_id.is_empty()
        || input.session.is_empty()
        || input.task.is_empty()
        || input.depends_node_id.is_empty()
        || input.depends_session.is_empty()
        || input.depends_task.is_empty()
    {
        bail!("task dependencies require node_id, session, and task on both sides");
    }
    if input.node_id == input.depends_node_id
        && input.session == input.depends_session
        && input.task == input.depends_task
    {
        bail!("a task cannot depend on itself");
    }
    match input.required_state.as_deref().map(str::trim) {
        None | Some("") => input.required_state = Some("running".to_string()),
        Some("running") => input.required_state = Some("running".to_string()),
        Some(other) => bail!("unsupported dependency required state '{other}'"),
    }
    Ok(input)
}
