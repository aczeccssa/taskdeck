//! SQLite-backed persistent state store.
//!
//! `StateStore` is the single handle over `state.db`. Its `impl` blocks are
//! split across domain submodules (node, sessions, auth, workflow, boards,
//! quota, notification, dependencies, scaling, audit, events, mcp_calls);
//! this file defines the struct and re-exports domain types so external
//! paths (`crate::state::X`) stay stable.

mod audit;
mod auth;
mod boards;
mod dependencies;
mod events;
mod mcp_calls;
mod migrate;
mod node;
mod notification;
mod pagination;
mod quota;
mod scaling;
mod schema;
mod sessions;
mod user_config;
mod util;
mod workflow;

#[cfg(test)]
mod test_support;

// The re-exports below preserve the pre-split public interface; some items
// are consumed only by downstream modules or tests.
pub use crate::protocol::{LeaderMode, NodeRole, PublicNodeSettings};
#[allow(unused_imports)]
pub use auth::{AuthSettings, PublicAuthStatus, hash_access_key, verify_access_key};
#[allow(unused_imports)]
pub use node::{
    KnownWorker, NodeSettings, NodeSettingsUpdate, NodeSettingsWrite, environment_overrides,
};
#[allow(unused_imports)]
pub use scaling::{cron_next_after, validate_cron_expression};
#[allow(unused_imports)]
pub use sessions::Registration;

use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::Connection;

pub const DEFAULT_BIND_HOST: &str = "127.0.0.1";
pub const DEFAULT_WEB_PORT: u16 = 9837;
pub const AUTH_SESSION_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;
pub const AUDIT_RETENTION_LIMIT: usize = 10_000;
/// Keep the oldest pending records available for a future worker-to-leader sync.
pub const AUDIT_REPLICATION_QUEUE_LIMIT: usize = 10_000;
/// Keep the newest pending records visible in the local audit history while a
/// worker is offline; the middle of an over-capacity backlog is discarded.
pub const AUDIT_LOCAL_DISPLAY_LIMIT: usize = 10_000;
pub const WORKFLOW_REVISION_RETENTION_LIMIT: usize = 50;
pub const NOTIFICATION_RETENTION_LIMIT: usize = 1_000;

pub struct StateStore {
    pub(super) connection: Mutex<Connection>,
    pub(super) root: Option<PathBuf>,
}

impl StateStore {
    pub fn metadata(&self, key: &str) -> anyhow::Result<Option<String>> {
        let connection = self.connection.lock().expect("state database lock");
        util::get_metadata(&connection, key)
    }

    pub fn set_metadata(&self, key: &str, value: &str) -> anyhow::Result<()> {
        let connection = self.connection.lock().expect("state database lock");
        util::set_metadata(&connection, key, value)
    }
}

#[cfg(test)]
mod tests;
