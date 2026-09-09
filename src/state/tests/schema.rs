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
}

#[test]
fn schema_one_migrates_the_old_default_bind_host() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::open(dir.path()).unwrap();
    {
        let connection = store.connection.lock().unwrap();
        set_metadata(&connection, "schema_version", "1").unwrap();
        set_metadata(&connection, "bind_host", "127.0.0.1").unwrap();
    }
    drop(store);

    let migrated = StateStore::open(dir.path()).unwrap();
    assert_eq!(migrated.node_settings().unwrap().bind_host, "0.0.0.0");
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
    {
        let connection = store.connection.lock().unwrap();
        set_metadata(&connection, "schema_version", "1").unwrap();
        set_metadata(&connection, "bind_host", "192.168.1.20").unwrap();
    }
    drop(store);

    let migrated = StateStore::open(dir.path()).unwrap();
    assert_eq!(migrated.node_settings().unwrap().bind_host, "192.168.1.20");
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
