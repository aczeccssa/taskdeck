//! MCP call history persistence.


use anyhow::Result;
use rusqlite::{OptionalExtension, params, params_from_iter};

use super::pagination::*;
use super::util::*;
use super::StateStore;
use crate::protocol::*;

impl StateStore {
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

}

pub(super) fn paginated_mcp_calls(
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

pub(super) fn build_mcp_search_text(
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

