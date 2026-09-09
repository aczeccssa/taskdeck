//! Workflow group and revision persistence.

use std::collections::HashSet;

use anyhow::{Context, Result, bail};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::util::*;
use super::{StateStore, WORKFLOW_REVISION_RETENTION_LIMIT};
use crate::protocol::*;

impl StateStore {
    pub fn workflow_groups(&self) -> Result<Vec<WorkflowGroup>> {
        let connection = self.connection.lock().expect("state store lock");
        let mut statement = connection.prepare(
            "SELECT id, name, created_at_ms, updated_at_ms, graph_json
             FROM workflow_groups
             ORDER BY name COLLATE NOCASE, created_at_ms, id",
        )?;
        let mut groups = statement
            .query_map([], |row| {
                Ok(WorkflowGroup {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at_ms: row.get::<_, i64>(2)? as u64,
                    updated_at_ms: row.get::<_, i64>(3)? as u64,
                    members: Vec::new(),
                    graph: row
                        .get::<_, Option<String>>(4)?
                        .and_then(|json| serde_json::from_str(&json).ok())
                        .unwrap_or_default(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut members = connection.prepare(
            "SELECT node_id, session, task
             FROM workflow_group_members
             WHERE group_id=?1
             ORDER BY position",
        )?;
        for group in &mut groups {
            group.members = members
                .query_map(params![group.id], |row| {
                    Ok(WorkflowGroupMember {
                        node_id: row.get(0)?,
                        session: row.get(1)?,
                        task: row.get(2)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        }
        Ok(groups)
    }

    pub fn workflow_group(&self, id: &str) -> Result<Option<WorkflowGroup>> {
        Ok(self
            .workflow_groups()?
            .into_iter()
            .find(|group| group.id == id))
    }

    pub fn create_workflow_group(&self, input: WorkflowGroupInput) -> Result<WorkflowGroup> {
        let input = normalize_workflow_group_input(input)?;
        let now = current_timestamp_ms();
        let id = Uuid::new_v4().to_string();
        {
            let mut connection = self.connection.lock().expect("state store lock");
            let transaction = connection.transaction()?;
            transaction
                .execute(
                    "INSERT INTO workflow_groups(id, name, created_at_ms, updated_at_ms, graph_json) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        id,
                        input.name,
                        now as i64,
                        now as i64,
                        serde_json::to_string(&input.graph)?
                    ],
                )
                .with_context(|| format!("failed to create workflow group '{}'", input.name))?;
            write_workflow_members(&transaction, &id, &input.members)?;
            record_workflow_revision_in_tx(
                &transaction,
                &id,
                1,
                &input.name,
                &input.members,
                &input.graph,
                None,
                now,
            )?;
            transaction.commit()?;
        }
        self.workflow_group(&id)?
            .with_context(|| format!("workflow group '{id}' disappeared after create"))
    }

    pub fn update_workflow_group(
        &self,
        id: &str,
        input: WorkflowGroupInput,
        note: Option<&str>,
    ) -> Result<WorkflowGroup> {
        let input = normalize_workflow_group_input(input)?;
        let now = current_timestamp_ms();
        {
            let mut connection = self.connection.lock().expect("state store lock");
            let transaction = connection.transaction()?;
            let changed = transaction
                .execute(
                    "UPDATE workflow_groups SET name=?2, updated_at_ms=?3, graph_json=?4 WHERE id=?1",
                    params![id, input.name, now as i64, serde_json::to_string(&input.graph)?],
                )
                .with_context(|| format!("failed to update workflow group '{id}'"))?;
            if changed == 0 {
                bail!("workflow group '{id}' not found");
            }
            transaction.execute(
                "DELETE FROM workflow_group_members WHERE group_id=?1",
                params![id],
            )?;
            write_workflow_members(&transaction, id, &input.members)?;
            let revision = next_workflow_revision(&transaction, id)?;
            record_workflow_revision_in_tx(
                &transaction,
                id,
                revision,
                &input.name,
                &input.members,
                &input.graph,
                note,
                now,
            )?;
            transaction.commit()?;
        }
        self.workflow_group(id)?
            .with_context(|| format!("workflow group '{id}' disappeared after update"))
    }

    pub fn delete_workflow_group(&self, id: &str) -> Result<bool> {
        let connection = self.connection.lock().expect("state store lock");
        Ok(connection.execute("DELETE FROM workflow_groups WHERE id=?1", params![id])? > 0)
    }

}

impl StateStore {
    pub fn workflow_revisions(&self, group_id: &str) -> Result<Vec<WorkflowRevision>> {
        let connection = self.connection.lock().expect("state store lock");
        let mut statement = connection.prepare(
            "SELECT revision, snapshot_json, note, created_at_ms
             FROM workflow_revisions
             WHERE group_id=?1
             ORDER BY revision DESC",
        )?;
        let rows = statement
            .query_map(params![group_id], |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)? as u64,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut revisions = Vec::new();
        for (revision, snapshot_json, note, created_at_ms) in rows {
            let snapshot: WorkflowRevisionSnapshot = serde_json::from_str(&snapshot_json)
                .with_context(|| {
                    format!("failed to parse revision {revision} of workflow group '{group_id}'")
                })?;
            revisions.push(WorkflowRevision {
                group_id: group_id.to_string(),
                revision,
                name: snapshot.name,
                members: snapshot.members,
                graph: snapshot.graph,
                note,
                created_at_ms,
            });
        }
        Ok(revisions)
    }

}

pub(super) fn normalize_workflow_group_input(mut input: WorkflowGroupInput) -> Result<WorkflowGroupInput> {
    input.name = input.name.trim().to_string();
    if input.name.is_empty() {
        bail!("workflow group name cannot be empty");
    }

    let mut seen = HashSet::new();
    for member in &mut input.members {
        member.node_id = member.node_id.trim().to_string();
        member.session = member.session.trim().to_string();
        member.task = member.task.trim().to_string();
        if member.node_id.is_empty() || member.session.is_empty() || member.task.is_empty() {
            bail!("workflow group members require node_id, session, and task");
        }
        let key = (
            member.node_id.clone(),
            member.session.clone(),
            member.task.clone(),
        );
        if !seen.insert(key) {
            bail!(
                "duplicate workflow group member '{}:{}:{}'",
                member.node_id,
                member.session,
                member.task
            );
        }
    }

    let mut seen_edges = HashSet::new();
    for edge in &input.graph.edges {
        if edge.from >= input.members.len() || edge.to >= input.members.len() {
            bail!("workflow graph edge references a member that does not exist");
        }
        if edge.from == edge.to {
            bail!("workflow graph edges cannot connect a member to itself");
        }
        if !seen_edges.insert((edge.from, edge.to)) {
            bail!("duplicate workflow graph edge");
        }
    }
    if workflow_graph_has_cycle(&input.graph.edges, input.members.len()) {
        bail!("workflow graph edges cannot contain cycles");
    }
    if input.graph.positions.len() > input.members.len() {
        input.graph.positions.truncate(input.members.len());
    }

    Ok(input)
}

pub(super) fn write_workflow_members(
    transaction: &rusqlite::Transaction<'_>,
    group_id: &str,
    members: &[WorkflowGroupMember],
) -> Result<()> {
    for (position, member) in members.iter().enumerate() {
        transaction.execute(
            "INSERT INTO workflow_group_members(group_id, position, node_id, session, task)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                group_id,
                position as i64,
                &member.node_id,
                &member.session,
                &member.task
            ],
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkflowRevisionSnapshot {
    name: String,
    members: Vec<WorkflowGroupMember>,
    graph: WorkflowGraph,
}

pub(super) fn next_workflow_revision(transaction: &rusqlite::Transaction<'_>, group_id: &str) -> Result<u64> {
    let current = transaction
        .query_row(
            "SELECT COALESCE(MAX(revision), 0) FROM workflow_revisions WHERE group_id=?1",
            params![group_id],
            |row| row.get::<_, i64>(0),
        )
        .context("failed to read workflow revision counter")?;
    Ok(current as u64 + 1)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn record_workflow_revision_in_tx(
    transaction: &rusqlite::Transaction<'_>,
    group_id: &str,
    revision: u64,
    name: &str,
    members: &[WorkflowGroupMember],
    graph: &WorkflowGraph,
    note: Option<&str>,
    at_ms: u64,
) -> Result<()> {
    let snapshot = WorkflowRevisionSnapshot {
        name: name.to_string(),
        members: members.to_vec(),
        graph: graph.clone(),
    };
    transaction.execute(
        "INSERT INTO workflow_revisions(group_id, revision, snapshot_json, note, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            group_id,
            revision as i64,
            serde_json::to_string(&snapshot)?,
            note,
            at_ms as i64
        ],
    )?;
    transaction.execute(
        "DELETE FROM workflow_revisions WHERE group_id=?1 AND revision <= (
             SELECT MAX(revision) - ?2 FROM workflow_revisions WHERE group_id=?1
         )",
        params![group_id, WORKFLOW_REVISION_RETENTION_LIMIT as i64],
    )?;
    Ok(())
}

pub(super) fn workflow_graph_has_cycle(
    edges: &[crate::protocol::WorkflowGraphEdge],
    member_count: usize,
) -> bool {
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); member_count];
    for edge in edges {
        adjacency[edge.from].push(edge.to);
    }
    // 0 = unvisited, 1 = in progress, 2 = done
    let mut colors = vec![0u8; member_count];
    for start in 0..member_count {
        let mut stack = vec![(start, 0usize)];
        while let Some((node, cursor)) = stack.pop() {
            if cursor == 0 {
                if colors[node] == 1 {
                    return true;
                }
                if colors[node] == 2 {
                    continue;
                }
                colors[node] = 1;
            }
            if let Some(&next) = adjacency[node].get(cursor) {
                stack.push((node, cursor + 1));
                if colors[next] != 2 {
                    stack.push((next, 0));
                }
            } else {
                colors[node] = 2;
            }
        }
    }
    false
}

