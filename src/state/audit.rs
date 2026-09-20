//! Audit record persistence.

use anyhow::Result;
use rusqlite::{OptionalExtension, params, params_from_iter};
use uuid::Uuid;

use super::pagination::*;
use super::util::*;
use super::{AUDIT_REPLICATION_QUEUE_LIMIT, AUDIT_RETENTION_LIMIT, StateStore};
use crate::protocol::*;

const AUDIT_SEARCH_EXCERPT_BYTES: usize = 4 * 1024;

impl StateStore {
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
        let deleted_replicated = connection.execute(
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
        let deleted_unreplicated = connection.execute(
            "DELETE FROM audit_records
             WHERE replicated_at_ms IS NULL
               AND audit_id IN (
                    SELECT audit_id FROM audit_records
                    WHERE replicated_at_ms IS NULL
                    ORDER BY timestamp_ms ASC, audit_id ASC
                    LIMIT -1 OFFSET ?1
               )",
            params![AUDIT_REPLICATION_QUEUE_LIMIT as i64],
        )?;
        Ok(deleted_replicated + deleted_unreplicated)
    }
}

pub(super) fn paginated_audit(
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

pub(super) fn build_audit_search_text(
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
        audit_search_excerpt(request_json),
        audit_search_excerpt(response_json),
        audit_search_excerpt(details_json),
    ))
}

fn audit_search_excerpt(value: &str) -> String {
    if value.len() <= AUDIT_SEARCH_EXCERPT_BYTES {
        return value.to_string();
    }
    let mut end = AUDIT_SEARCH_EXCERPT_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

pub(super) fn parse_audit_source(value: String) -> rusqlite::Result<AuditSource> {
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

pub(super) fn parse_audit_status(value: String) -> rusqlite::Result<AuditStatus> {
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

pub(super) fn parse_audit_transport(value: String) -> rusqlite::Result<AuditTransport> {
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

pub(super) fn map_audit_list_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditListItem> {
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

pub(super) fn map_audit_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditRecord> {
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
