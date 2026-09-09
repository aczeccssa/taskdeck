//! Cross-domain state helpers (metadata, JSON columns, timestamps).

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};


pub(super) fn parse_sql_json(value: String, column: usize) -> rusqlite::Result<serde_json::Value> {
    serde_json::from_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

pub(super) fn get_metadata(connection: &Connection, key: &str) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT value FROM metadata WHERE key=?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .with_context(|| format!("failed to read state key '{key}'"))
}

pub(super) fn set_metadata(connection: &Connection, key: &str, value: &str) -> Result<()> {
    connection.execute(
        "INSERT INTO metadata(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub(super) fn required_metadata(connection: &Connection, key: &str) -> Result<String> {
    get_metadata(connection, key)?.with_context(|| format!("missing state key '{key}'"))
}

pub(super) fn write_optional_metadata(connection: &Connection, key: &str, value: Option<&str>) -> Result<()> {
    match value {
        Some(value) => set_metadata(connection, key, value),
        None => {
            connection.execute("DELETE FROM metadata WHERE key=?1", params![key])?;
            Ok(())
        }
    }
}

pub(super) fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

pub(super) fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

