//! Event log and task-run history persistence.

use std::path::PathBuf;

use anyhow::Result;
use rusqlite::{params, params_from_iter};

use super::StateStore;
use super::pagination::*;
use super::util::*;
use crate::protocol::*;

impl StateStore {
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
}

impl StateStore {
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
}

impl StateStore {
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
        let status = if finished_at_ms.is_some() {
            "failed"
        } else {
            "running"
        };
        let duration_ms = finished_at_ms.map(|value| value.saturating_sub(snapshot.started_at_ms));
        connection.execute("INSERT INTO task_runs(node_id,session,task,trigger,status,started_at_ms,finished_at_ms,duration_ms,command,cwd,pid,run_generation,exit_code,error_message) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)", params![node_id,session,snapshot.label,trigger,status,snapshot.started_at_ms as i64,finished_at_ms.map(|v| v as i64),duration_ms.map(|v| v as i64),command,cwd,snapshot.pid.map(|v| v as i64),snapshot.run_generation as i64,None::<i64>,error_message])?;
        Ok(Some(TaskRunRecord {
            id: connection.last_insert_rowid() as u64,
            node_id: node_id.to_string(),
            session: session.to_string(),
            task: snapshot.label.clone(),
            trigger: trigger.to_string(),
            status: status.to_string(),
            started_at_ms: snapshot.started_at_ms,
            finished_at_ms,
            duration_ms,
            command,
            cwd: snapshot.cwd.clone(),
            pid: snapshot.pid,
            run_generation: snapshot.run_generation,
            exit_code: None,
            error_message,
        }))
    }

    pub fn record_task_run_failure(
        &self,
        node_id: &str,
        snapshot: &crate::protocol::TaskSnapshot,
        trigger: &str,
        session: &str,
        error_message: impl Into<String>,
    ) -> Result<Option<TaskRunRecord>> {
        self.record_terminal_task_run_attempt(
            node_id,
            snapshot,
            trigger,
            session,
            "failed",
            error_message,
        )
    }

    pub fn record_task_run_skipped(
        &self,
        node_id: &str,
        snapshot: &crate::protocol::TaskSnapshot,
        trigger: &str,
        session: &str,
        reason: impl Into<String>,
    ) -> Result<Option<TaskRunRecord>> {
        self.record_terminal_task_run_attempt(
            node_id, snapshot, trigger, session, "skipped", reason,
        )
    }

    fn record_terminal_task_run_attempt(
        &self,
        node_id: &str,
        snapshot: &crate::protocol::TaskSnapshot,
        trigger: &str,
        session: &str,
        status: &str,
        error_message: impl Into<String>,
    ) -> Result<Option<TaskRunRecord>> {
        let finished_at_ms = current_timestamp_ms();
        let mut attempt = snapshot.clone();
        attempt.started_at_ms = finished_at_ms;
        let command = attempt.command.clone();
        let cwd = attempt.cwd.to_string_lossy().into_owned();
        let error_message = error_message.into();
        let connection = self.connection.lock().expect("state store lock");
        connection.execute("INSERT INTO task_runs(node_id,session,task,trigger,status,started_at_ms,finished_at_ms,duration_ms,command,cwd,pid,run_generation,exit_code,error_message) VALUES (?1,?2,?3,?4,?5,?6,?7,0,?8,?9,?10,?11,?12,?13)", params![node_id,session,attempt.label,trigger,status,finished_at_ms as i64,finished_at_ms as i64,command,cwd,attempt.pid.map(|v| v as i64),attempt.run_generation as i64,None::<i64>,error_message])?;
        Ok(Some(TaskRunRecord {
            id: connection.last_insert_rowid() as u64,
            node_id: node_id.to_string(),
            session: session.to_string(),
            task: attempt.label,
            trigger: trigger.to_string(),
            status: status.to_string(),
            started_at_ms: finished_at_ms,
            finished_at_ms: Some(finished_at_ms),
            duration_ms: Some(0),
            command,
            cwd: attempt.cwd,
            pid: attempt.pid,
            run_generation: attempt.run_generation,
            exit_code: None,
            error_message: Some(error_message),
        }))
    }

    pub fn finish_running_task_runs(
        &self,
        node_id: &str,
        status: &str,
        reason: &str,
    ) -> Result<usize> {
        let finished = current_timestamp_ms();
        let connection = self.connection.lock().expect("state store lock");
        let updated = connection.execute(
            "UPDATE task_runs SET status=?2,finished_at_ms=?3,duration_ms=MAX(0,?3-started_at_ms),error_message=?4 WHERE node_id=?1 AND status='running'",
            params![node_id, status, finished as i64, reason],
        )?;
        Ok(updated)
    }

    /// Avoid the scheduler/history sampler double-write without treating a
    /// runtime-local generation as globally unique across daemon restarts.
    pub fn has_task_run(
        &self,
        node_id: &str,
        session: &str,
        task: &str,
        run_generation: u64,
        started_at_ms: u64,
    ) -> Result<bool> {
        let connection = self.connection.lock().expect("state store lock");
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM task_runs WHERE node_id=?1 AND session=?2 AND task=?3 AND run_generation=?4 AND started_at_ms=?5",
            params![node_id, session, task, run_generation as i64, started_at_ms as i64],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn record_task_run_start(
        &self,
        node_id: &str,
        snapshot: &crate::protocol::TaskSnapshot,
        trigger: &str,
        session: &str,
    ) -> Result<Option<TaskRunRecord>> {
        if self.has_task_run(
            node_id,
            session,
            &snapshot.label,
            snapshot.run_generation,
            snapshot.started_at_ms,
        )? {
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
}

impl StateStore {
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

pub(super) fn map_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRecord> {
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

pub(super) fn map_task_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRunRecord> {
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

pub(super) fn paginated_task_runs(
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

pub(super) fn paginated_events(
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
