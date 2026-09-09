//! Shared helpers: query parsing, casefold search text and audit payload
//! redaction/truncation, plus serde default value functions.

use serde_json::Value;
use unicode_casefold::UnicodeCaseFold;

use super::base::Response;

pub(crate) fn default_true() -> bool {
    true
}

pub(crate) fn default_required_state() -> String {
    "running".to_string()
}

pub(crate) fn default_cooldown_seconds() -> u64 {
    300
}

pub fn parse_positive_usize(
    query: &std::collections::HashMap<String, String>,
    key: &str,
    default: usize,
) -> std::result::Result<usize, Response> {
    let value = query
        .get(key)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());
    match value {
        None => Ok(default),
        Some(value) => value
            .parse::<usize>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                Response::error_with_data(
                    format!("invalid {key}"),
                    serde_json::json!({"kind": "validation_error", "status": 400}),
                )
            }),
    }
}

pub fn parse_history_page_size(
    query: &std::collections::HashMap<String, String>,
) -> std::result::Result<usize, Response> {
    const SUPPORTED: [usize; 3] = [20, 50, 100];
    let requested = match query
        .get("page_size")
        .map(|value| value.trim())
        .filter(|v| !v.is_empty())
        .or_else(|| {
            query
                .get("limit")
                .map(|value| value.trim())
                .filter(|v| !v.is_empty())
        }) {
        None => return Ok(20),
        Some(value) => value
            .parse::<usize>()
            .ok()
            .filter(|v| *v > 0)
            .ok_or_else(|| {
                Response::error_with_data(
                    "invalid page_size",
                    serde_json::json!({"kind": "validation_error", "status": 400}),
                )
            })?,
    };
    Ok(*SUPPORTED
        .iter()
        .min_by_key(|size| (requested.abs_diff(**size), **size))
        .expect("supported page sizes"))
}

pub fn casefold_search_text(value: &str) -> String {
    value.case_fold().collect()
}

pub const AUDIT_PAYLOAD_LIMIT_BYTES: usize = 64 * 1024;
pub const REDACTED_VALUE: &str = "[REDACTED]";

fn sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .map(|ch| if ch == '-' { '_' } else { ch })
        .collect::<String>()
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "token"
            | "password"
            | "secret"
            | "api_key"
            | "apikey"
            | "authorization"
            | "cookie"
            | "credential"
            | "credentials"
            | "access_key"
            | "accesskey"
            | "private_key"
            | "privatekey"
            | "enrollment_token"
    ) || normalized.ends_with("_token")
        || normalized.ends_with("_secret")
        || normalized.ends_with("_password")
        || normalized.ends_with("_key")
}

pub fn redact_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, nested)| {
                    let redacted = if sensitive_key(key) {
                        Value::String(REDACTED_VALUE.to_string())
                    } else {
                        redact_json(nested)
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact_json).collect()),
        other => other.clone(),
    }
}

pub fn truncate_json(value: Value, limit: usize) -> Value {
    let serialized = serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string());
    if serialized.len() <= limit {
        return value;
    }
    serde_json::json!({
        "truncated": true,
        "original_bytes": serialized.len(),
        "preview": serialized.chars().take(256).collect::<String>(),
    })
}

pub fn sanitize_audit_value(value: &Value) -> Value {
    truncate_json(redact_json(value), AUDIT_PAYLOAD_LIMIT_BYTES)
}
