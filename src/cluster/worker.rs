//! Worker-side connection client.

use super::messages::{AgentMessage, agent_url, parse_worker_message};
use super::{AGENT_PROTOCOL_VERSION, COMMAND_CACHE_SIZE, COMMAND_TIMEOUT};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use axum::extract::ws::{Message as AxumMessage, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use uuid::Uuid;

use crate::daemon::{DaemonState, dispatch_async_with_audit};
use crate::protocol::NodeMetricsSample;
use crate::protocol::{
    Action, AuditContext, AuditRecord, AuditStatus, EditableTaskInput, EventFilter,
    NodeSettingsPatch, NodeSummary, Request, Response, SessionSnapshot, TaskRunFilter,
};
use crate::state::{NodeSettings, StateStore};

pub fn spawn_worker_client(
    state: DaemonState,
    settings: NodeSettings,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let cache = Arc::new(Mutex::new(CommandResultCache::new(COMMAND_CACHE_SIZE)));
        let mut delay = Duration::from_secs(1);
        while !state.shutdown.load(std::sync::atomic::Ordering::SeqCst) {
            if run_worker_connection(state.clone(), &settings, cache.clone())
                .await
                .is_ok()
            {
                delay = Duration::from_secs(1);
            } else {
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(30));
            }
        }
    })
}

pub(super) async fn run_worker_connection(
    state: DaemonState,
    settings: &NodeSettings,
    cache: Arc<Mutex<CommandResultCache>>,
) -> Result<()> {
    let url = agent_url(
        settings
            .leader_url
            .as_deref()
            .context("worker leader URL is not configured")?,
    )?;
    let (socket, _) = tokio_tungstenite::connect_async(&url)
        .await
        .with_context(|| format!("failed to connect to leader at {url}"))?;
    let (mut writer, mut reader) = socket.split();
    send_worker(
        &mut writer,
        &AgentMessage::Hello {
            protocol: AGENT_PROTOCOL_VERSION,
            node_id: settings.node_id.clone(),
            name: settings.name.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            token: settings.enrollment_token.clone(),
        },
    )
    .await?;
    match reader.next().await {
        Some(Ok(message)) => match parse_worker_message(message)? {
            AgentMessage::Welcome { protocol } if protocol == AGENT_PROTOCOL_VERSION => {}
            AgentMessage::Error { message } => bail!("leader rejected worker: {message}"),
            _ => bail!("leader did not send a valid welcome"),
        },
        _ => bail!("leader closed before welcome"),
    }

    send_pending_audit_records(&mut writer, &state).await?;
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                send_worker(&mut writer, &AgentMessage::Inventory {
                    sessions: state.local_inventory(),
                    node_metrics: state.node_metrics.latest(&settings.node_id),
                }).await?;
                send_worker(&mut writer, &AgentMessage::Heartbeat {
                    timestamp_ms: current_timestamp_ms(),
                }).await?;
                send_pending_audit_records(&mut writer, &state).await?;
            }
            message = reader.next() => {
                let Some(message) = message else { bail!("leader connection closed"); };
                let message = parse_worker_message(message?)?;
                match message {
                    AgentMessage::Command { id, request, audit } => {
                        let cached = cache.lock().expect("command cache lock").get(&id);
                        let response = if let Some(response) = cached {
                            response
                        } else {
                            let response = dispatch_async_with_audit(state.clone(), request.into_local(), audit).await;
                            cache.lock().expect("command cache lock").insert(id.clone(), response.clone());
                            response
                        };
                        send_worker(&mut writer, &AgentMessage::CommandResult { id, response }).await?;
                        send_pending_audit_records(&mut writer, &state).await?;
                    }
                    AgentMessage::AuditAck { audit_ids } => {
                        let _ = state.store.mark_audit_replicated(&audit_ids, current_timestamp_ms());
                    }
                    _ => bail!("unexpected leader message"),
                }
            }
        }
    }
}

pub(super) async fn send_pending_audit_records<S>(writer: &mut S, state: &DaemonState) -> Result<()>
where
    S: futures_util::Sink<WsMessage> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    let records = state.store.unreplicated_audit_records(100)?;
    if !records.is_empty() {
        send_worker(writer, &AgentMessage::AuditBatch { records }).await?;
    }
    Ok(())
}

pub(super) async fn send_worker<S>(writer: &mut S, message: &AgentMessage) -> Result<()>
where
    S: futures_util::Sink<WsMessage> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    writer
        .send(WsMessage::Text(serde_json::to_string(message)?.into()))
        .await
        .context("failed to send worker message")
}

pub(super) struct CommandResultCache {
    pub(super) capacity: usize,
    pub(super) entries: VecDeque<(String, Response)>,
}

impl CommandResultCache {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: VecDeque::new(),
        }
    }

    pub(super) fn get(&self, id: &str) -> Option<Response> {
        self.entries
            .iter()
            .find(|(entry_id, _)| entry_id == id)
            .map(|(_, response)| response.clone())
    }

    pub(super) fn insert(&mut self, id: String, response: Response) {
        self.entries.push_back((id, response));
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }
}

pub(super) fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
