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

use std::sync::Mutex;

use rusqlite::Connection;

pub const DEFAULT_BIND_HOST: &str = "0.0.0.0";
pub const DEFAULT_WEB_PORT: u16 = 9837;
pub const AUTH_SESSION_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;
pub const AUDIT_RETENTION_LIMIT: usize = 10_000;
pub const WORKFLOW_REVISION_RETENTION_LIMIT: usize = 50;
pub const NOTIFICATION_RETENTION_LIMIT: usize = 1_000;

pub struct StateStore {
    pub(super) connection: Mutex<Connection>,
}

#[cfg(test)]
mod tests;
