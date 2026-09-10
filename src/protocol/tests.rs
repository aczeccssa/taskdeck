use super::{
    AuditSource, AuditTransport, Envelope, Request, casefold_search_text, redact_json,
    sanitize_audit_value,
};
use serde_json::json;

#[test]
fn casefold_search_text_matches_expanding_equivalents() {
    assert_eq!(
        casefold_search_text("Straße"),
        casefold_search_text("STRASSE")
    );
}

#[test]
fn casefold_search_text_matches_sigma_and_final_sigma() {
    assert_eq!(casefold_search_text("οσ"), casefold_search_text("ος"));
}

#[test]
fn envelope_parses_bare_request_and_wrapped_request() {
    let bare = r#"{"type":"ping"}"#;
    let parsed = Envelope::parse_line(bare).unwrap();
    assert!(matches!(parsed.request, Request::Ping));
    assert!(parsed.audit.is_none());

    let wrapped = serde_json::to_string(&Envelope::new(
        Request::ListSessions,
        super::AuditContext::new(AuditSource::Cli, AuditTransport::Ipc),
    ))
    .unwrap();
    let parsed = Envelope::parse_line(&wrapped).unwrap();
    assert!(matches!(parsed.request, Request::ListSessions));
    assert_eq!(parsed.audit.unwrap().source, AuditSource::Cli);
}

#[test]
fn redacts_nested_sensitive_fields_and_truncates_large_payloads() {
    let value = json!({
        "token": "secret-value",
        "nested": {"api_key": "abc", "ok": true},
        "items": [{"password": "p", "name": "keep"}]
    });
    let redacted = redact_json(&value);
    assert_eq!(redacted["token"], "[REDACTED]");
    assert_eq!(redacted["nested"]["api_key"], "[REDACTED]");
    assert_eq!(redacted["items"][0]["password"], "[REDACTED]");
    assert_eq!(redacted["items"][0]["name"], "keep");

    let huge = json!({"blob": "x".repeat(70_000)});
    let sanitized = sanitize_audit_value(&huge);
    assert_eq!(sanitized["truncated"], true);
    assert!(sanitized["original_bytes"].as_u64().unwrap() > 64_000);
}
