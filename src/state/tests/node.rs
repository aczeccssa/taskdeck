//! node domain state tests.

use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use super::super::util::get_metadata;
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
    let user_config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("taskdeck.json")).unwrap())
            .unwrap();
    assert_eq!(user_config["bind_host"], "127.0.0.1");
    assert_eq!(user_config["web_port"], 9937);
    let connection = store.connection.lock().unwrap();
    assert_eq!(
        get_metadata(&connection, "bind_host").unwrap().as_deref(),
        Some("127.0.0.1")
    );
    assert_eq!(
        get_metadata(&connection, "web_port").unwrap().as_deref(),
        Some("9937")
    );
    drop(connection);
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

#[test]
fn default_network_config_is_loopback_and_secret_files_are_private() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::open(dir.path()).unwrap();
    let settings = store.node_settings().unwrap();

    assert_eq!(settings.bind_host, "127.0.0.1");
    assert!(!settings.public().allow_remote_bind);
    #[cfg(unix)]
    {
        assert_eq!(
            std::fs::metadata(dir.path().join("taskdeck.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(dir.path().join("state.db"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(dir.path().join("state-migration.lock"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn remote_network_binding_requires_explicit_opt_in() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::open(dir.path()).unwrap();
    let error = store
        .configure(NodeSettingsUpdate {
            bind_host: Some("0.0.0.0".to_string()),
            ..NodeSettingsUpdate::default()
        })
        .unwrap_err();
    assert!(error.to_string().contains("allow_remote_bind=true"));

    let settings = store
        .configure(NodeSettingsUpdate {
            bind_host: Some("0.0.0.0".to_string()),
            allow_remote_bind: Some(true),
            ..NodeSettingsUpdate::default()
        })
        .unwrap();
    assert_eq!(settings.bind_host, "0.0.0.0");
    assert!(settings.public().allow_remote_bind);
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("taskdeck.json")).unwrap())
            .unwrap();
    assert_eq!(config["allow_remote_bind"], true);
}
