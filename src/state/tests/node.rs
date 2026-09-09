//! node domain state tests.

use std::path::Path;

use super::super::*;
use crate::protocol::*;

#[test]
fn node_settings_patch_handles_tokens_and_restart_flags() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::open(dir.path()).unwrap();
    let result = store
        .configure_patch(crate::protocol::NodeSettingsPatch {
            name: Some(" laptop ".to_string()),
            bind_host: Some("127.0.0.1".to_string()),
            web_port: Some(9937),
            enrollment_token: Some(crate::protocol::EnrollmentTokenUpdate::Set {
                value: "secret".to_string(),
            }),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(result.settings.name, "laptop");
    assert!(result.restart_required);
    assert!(result.settings.has_enrollment_token);
    let serialized = serde_json::to_string(&result).unwrap();
    assert!(!serialized.contains("secret"));
    let result = store
        .configure_patch(crate::protocol::NodeSettingsPatch {
            enrollment_token: Some(crate::protocol::EnrollmentTokenUpdate::Clear),
            ..Default::default()
        })
        .unwrap();
    assert!(!result.settings.has_enrollment_token);
    assert!(result.restart_required);
    assert!(
        store
            .configure_patch(crate::protocol::NodeSettingsPatch {
                role: Some("master".to_string()),
                ..Default::default()
            })
            .is_err()
    );
}

#[test]
fn pure_master_requires_empty_local_registry() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::open(dir.path()).unwrap();
    store
        .upsert_registration("api", Path::new("/tmp/api"))
        .unwrap();
    let error = store
        .configure(NodeSettingsUpdate {
            role: Some(NodeRole::Leader),
            leader_mode: Some(LeaderMode::PureMaster),
            ..NodeSettingsUpdate::default()
        })
        .unwrap_err();
    assert!(error.to_string().contains("local registration"));
}

#[test]
fn public_settings_redact_token() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::open(dir.path()).unwrap();
    let settings = store
        .configure(NodeSettingsUpdate {
            enrollment_token: Some(Some("secret".to_string())),
            ..NodeSettingsUpdate::default()
        })
        .unwrap();
    let serialized = serde_json::to_string(&settings.public()).unwrap();
    assert!(!serialized.contains("secret"));
    assert!(settings.public().has_enrollment_token);
}
