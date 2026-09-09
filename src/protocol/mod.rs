//! Wire protocol types shared by CLI, TUI, Web, daemon and cluster.
//!
//! Types are grouped by business domain; `mod.rs` re-exports everything so
//! external paths (`crate::protocol::X`) stay stable.

mod audit;
mod auth;
mod base;
mod board;
mod dependencies;
mod editable_task;
mod history;
mod metrics;
mod quota;
mod scaling;
mod service_obs;
mod session;
mod util;
mod workflow;

pub use audit::*;
pub use auth::*;
pub use base::*;
pub use board::*;
pub use dependencies::*;
pub use editable_task::*;
pub use history::*;
pub use metrics::*;
pub use quota::*;
pub use scaling::*;
pub use service_obs::*;
pub use session::*;
pub use workflow::*;
// Kept re-exported for interface compatibility with the pre-split monolith;
// the crate currently consumes these via `super::util` inside the module.
#[allow(unused_imports)]
pub use util::{
    casefold_search_text, parse_history_page_size, parse_positive_usize, redact_json,
    sanitize_audit_value, truncate_json, AUDIT_PAYLOAD_LIMIT_BYTES, REDACTED_VALUE,
};

#[cfg(test)]
mod tests;
