//! Leader/worker cluster over WebSocket.

mod leader;
mod messages;
mod worker;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use leader::{LeaderCluster, serve_agent_socket};
#[allow(unused_imports)]
pub use messages::{AgentMessage, RemoteRequest};
#[allow(unused_imports)]
pub use worker::spawn_worker_client;

pub const AGENT_PROTOCOL_VERSION: u32 = 2;
const MAX_AGENT_MESSAGE_BYTES: usize = 2 * 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const COMMAND_CACHE_SIZE: usize = 256;

use std::time::Duration;
