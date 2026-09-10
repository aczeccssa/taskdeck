//! schema domain state tests.

use std::path::Path;

use super::super::schema::SCHEMA_VERSION;
use super::super::util::*;
use super::super::*;
use crate::protocol::*;

#[test]
fn new_store_defaults_to_unlinked_worker_and_keeps_identity() {
    let dir = tempfile::tempdir().unwrap();
    let first = StateStore::open(dir.path())
        .unwrap()
        .node_settings()
        .unwrap();
    let second = StateStore::open(dir.path())
        .unwrap()
        .node_settings()
        .unwrap();
    assert_eq!(first.role, NodeRole::Worker);
    assert_eq!(first.leader_mode, LeaderMode::Standard);
    assert_eq!(first.node_id, second.node_id);
    assert!(first.leader_url.is_none());
    assert_eq!(first.bind_host, DEFAULT_BIND_HOST);
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("taskdeck.json")).unwrap())
            .unwrap();
    assert_eq!(config["version"], 1);
    assert_eq!(config["bind_host"], "127.0.0.1");
    assert_eq!(config["web_port"], 9837);
    let reopened = StateStore::open(dir.path()).unwrap();
    let connection = reopened.connection.lock().unwrap();
    let user_version: u32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(user_version, SCHEMA_VERSION.parse::<u32>().unwrap());
}

#[test]
fn schema_one_preserves_legacy_loopback_bind_host() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::open(dir.path()).unwrap();
    store.node_settings().unwrap();
    {
        let connection = store.connection.lock().unwrap();
        set_metadata(&connection, "schema_version", "1").unwrap();
        set_metadata(&connection, "bind_host", "127.0.0.1").unwrap();
        connection.execute_batch("PRAGMA user_version=0").unwrap();
    }
    std::fs::remove_file(dir.path().join("taskdeck.json")).unwrap();
    drop(store);

    let migrated = StateStore::open(dir.path()).unwrap();
    assert_eq!(migrated.node_settings().unwrap().bind_host, "127.0.0.1");
    let connection = migrated.connection.lock().unwrap();
    assert_eq!(
        get_metadata(&connection, "schema_version")
            .unwrap()
            .as_deref(),
        Some(SCHEMA_VERSION)
    );
}

#[test]
fn schema_one_preserves_a_custom_bind_host() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::open(dir.path()).unwrap();
    store.node_settings().unwrap();
    {
        let connection = store.connection.lock().unwrap();
        set_metadata(&connection, "schema_version", "1").unwrap();
        set_metadata(&connection, "bind_host", "192.168.1.20").unwrap();
        connection.execute_batch("PRAGMA user_version=0").unwrap();
    }
    std::fs::remove_file(dir.path().join("taskdeck.json")).unwrap();
    drop(store);

    let migrated = StateStore::open(dir.path()).unwrap();
    assert_eq!(migrated.node_settings().unwrap().bind_host, "192.168.1.20");
}

#[test]
fn existing_user_config_is_authoritative_over_legacy_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::open(dir.path()).unwrap();
    {
        let connection = store.connection.lock().unwrap();
        set_metadata(&connection, "bind_host", "0.0.0.0").unwrap();
        set_metadata(&connection, "web_port", "9837").unwrap();
    }
    std::fs::write(
        dir.path().join("taskdeck.json"),
        r#"{"version":1,"bind_host":"127.0.0.2","web_port":9940,"future_field":"kept"}"#,
    )
    .unwrap();
    assert_eq!(store.node_settings().unwrap().bind_host, "127.0.0.2");
    assert_eq!(store.node_settings().unwrap().web_port, 9940);
    store
        .configure(NodeSettingsUpdate {
            web_port: Some(9941),
            ..NodeSettingsUpdate::default()
        })
        .unwrap();
    let rewritten: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("taskdeck.json")).unwrap())
            .unwrap();
    assert_eq!(rewritten["future_field"], "kept");
    assert_eq!(rewritten["web_port"], 9941);
}

#[test]
fn invalid_user_config_does_not_fall_back_to_legacy_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::open(dir.path()).unwrap();
    std::fs::write(dir.path().join("taskdeck.json"), "{not-json").unwrap();
    let error = store.node_settings().unwrap_err();
    assert!(error.to_string().contains("failed to parse"));
}

#[test]
fn future_database_is_rejected_before_schema_is_modified() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("state.db");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
             INSERT INTO metadata VALUES ('schema_version','9');",
        )
        .unwrap();
    drop(connection);

    let before: i64 = rusqlite::Connection::open(&database)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let error = match StateStore::open(dir.path()) {
        Ok(_) => panic!("future database unexpectedly opened"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("newer than supported"));
    let after_connection = rusqlite::Connection::open(&database).unwrap();
    let after: i64 = after_connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(before, after);
    let notifications: i64 = after_connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='notifications'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(notifications, 0);
}

#[test]
fn schema_four_migrates_and_preserves_workspace_registrations() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("state.db");
    {
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
                .execute_batch(
                    "PRAGMA journal_mode=WAL;
                     CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
                     CREATE TABLE registrations(session TEXT PRIMARY KEY,project TEXT NOT NULL,registered_at_ms INTEGER NOT NULL);
                     INSERT INTO metadata VALUES ('schema_version','4');
                     INSERT INTO metadata VALUES ('node_id','legacy-id');
                     INSERT INTO metadata VALUES ('node_name','legacy');
                     INSERT INTO metadata VALUES ('role','worker');
                     INSERT INTO metadata VALUES ('leader_mode','standard');
                     INSERT INTO metadata VALUES ('bind_host','0.0.0.0');
                     INSERT INTO metadata VALUES ('web_port','9837');
                     INSERT INTO registrations VALUES ('api','/tmp/api',7);",
                )
                .unwrap();
    }
    let store = StateStore::open(dir.path()).unwrap();
    let backup = dir.path().join("state.db.bak-v4.sqlite");
    assert!(backup.exists());
    let backup_connection = rusqlite::Connection::open(&backup).unwrap();
    let integrity: String = backup_connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    let registration = &store.registrations().unwrap()[0];
    assert_eq!(registration.session, "api");
    assert_eq!(registration.alias, None);
    assert_eq!(registration.project, Path::new("/tmp/api"));
    assert_eq!(store.node_settings().unwrap().node_id, "legacy-id");
}

#[test]
fn schema_five_migrates_workflow_group_tables_without_losing_workspaces() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("state.db");
    {
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
                .execute_batch(
                    "PRAGMA journal_mode=WAL;
                     CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
                     CREATE TABLE registrations(session TEXT PRIMARY KEY,alias TEXT,project TEXT NOT NULL,registered_at_ms INTEGER NOT NULL);
                     INSERT INTO metadata VALUES ('schema_version','5');
                     INSERT INTO metadata VALUES ('node_id','legacy-id');
                     INSERT INTO metadata VALUES ('node_name','legacy');
                     INSERT INTO metadata VALUES ('role','leader');
                     INSERT INTO metadata VALUES ('leader_mode','standard');
                     INSERT INTO metadata VALUES ('bind_host','0.0.0.0');
                     INSERT INTO metadata VALUES ('web_port','9837');
                     INSERT INTO registrations VALUES ('api','Backend API','/tmp/api',7);",
                )
                .unwrap();
    }

    let store = StateStore::open(dir.path()).unwrap();
    let workspaces = store.workspace_summaries().unwrap();
    assert_eq!(workspaces[0].display_name, "Backend API");
    let group = store
        .create_workflow_group(crate::protocol::WorkflowGroupInput {
            name: "Backend".to_string(),
            members: vec![crate::protocol::WorkflowGroupMember {
                node_id: "self".to_string(),
                session: "api".to_string(),
                task: "dev".to_string(),
            }],
            graph: crate::protocol::WorkflowGraph::default(),
        })
        .unwrap();
    assert_eq!(
        store
            .workflow_group(&group.id)
            .unwrap()
            .unwrap()
            .members
            .len(),
        1
    );
    let connection = store.connection.lock().unwrap();
    assert_eq!(
        get_metadata(&connection, "schema_version")
            .unwrap()
            .as_deref(),
        Some(SCHEMA_VERSION)
    );
}

#[test]
fn schema_two_migrates_and_preserves_registrations() {
    let dir = tempfile::tempdir().unwrap();
    {
        let connection = rusqlite::Connection::open(dir.path().join("state.db")).unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON;
             CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
             CREATE TABLE registrations(session TEXT PRIMARY KEY,project TEXT NOT NULL,registered_at_ms INTEGER NOT NULL);
             CREATE TABLE workers(node_id TEXT PRIMARY KEY,name TEXT NOT NULL,last_seen_ms INTEGER NOT NULL,inventory_json TEXT NOT NULL);").unwrap();
        connection
            .execute("INSERT INTO registrations VALUES ('api','/tmp/api',1)", [])
            .unwrap();
        connection
            .execute("INSERT INTO metadata VALUES ('schema_version','2')", [])
            .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        connection
            .execute("INSERT INTO metadata VALUES ('node_id',?1)", [&id])
            .unwrap();
    }
    let store = StateStore::open(dir.path()).unwrap();
    assert_eq!(store.registrations().unwrap()[0].session, "api");
    let ids = store
        .list_task_runs(&TaskRunFilter {
            session: None,
            task: None,
            status: None,
            trigger: None,
            page: 1,
            page_size: 20,
        })
        .unwrap()
        .total;
    assert_eq!(ids, 0);
}
