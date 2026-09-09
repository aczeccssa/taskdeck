//! Leader-side cluster state and agent socket.

use super::messages::{AgentMessage, RemoteRequest, parse_axum_message};
use super::worker::current_timestamp_ms;
use super::{AGENT_PROTOCOL_VERSION, COMMAND_TIMEOUT};
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

#[derive(Clone)]
pub struct LeaderCluster {
    pub(super) inner: Arc<Mutex<LeaderClusterInner>>,
    pub(super) enrollment_token: Option<String>,
    pub(super) store: Arc<StateStore>,
    pub(super) node_metrics: Arc<crate::daemon::NodeMetricsStore>,
}

struct LeaderClusterInner {
    pub(super) workers: BTreeMap<String, WorkerState>,
    pub(super) pending: HashMap<String, oneshot::Sender<Response>>,
}

struct WorkerState {
    pub(super) name: String,
    pub(super) online: bool,
    pub(super) last_seen_ms: u64,
    pub(super) inventory: Vec<SessionSnapshot>,
    pub(super) sender: Option<mpsc::Sender<AgentMessage>>,
    pub(super) connection_id: Option<String>,
}

impl LeaderCluster {
    pub fn new(store: Arc<StateStore>, enrollment_token: Option<String>) -> Result<Self> {
        let mut workers = BTreeMap::new();
        for worker in store.known_workers()? {
            let inventory = serde_json::from_str(&worker.inventory_json).unwrap_or_default();
            workers.insert(
                worker.node_id,
                WorkerState {
                    name: worker.name,
                    online: false,
                    last_seen_ms: worker.last_seen_ms,
                    inventory,
                    sender: None,
                    connection_id: None,
                },
            );
        }
        Ok(Self {
            inner: Arc::new(Mutex::new(LeaderClusterInner {
                workers,
                pending: HashMap::new(),
            })),
            enrollment_token,
            store,
            node_metrics: Arc::new(crate::daemon::NodeMetricsStore::default()),
        })
    }

    pub fn node_metrics(&self) -> Arc<crate::daemon::NodeMetricsStore> {
        self.node_metrics.clone()
    }

    pub(super) fn validate_hello(&self, hello: &AgentMessage) -> Result<(String, String)> {
        let AgentMessage::Hello {
            protocol,
            node_id,
            name,
            token,
            ..
        } = hello
        else {
            bail!("first agent message must be hello");
        };
        if *protocol != AGENT_PROTOCOL_VERSION {
            bail!("unsupported agent protocol {protocol}; expected {AGENT_PROTOCOL_VERSION}");
        }
        if node_id.trim().is_empty() || name.trim().is_empty() {
            bail!("worker identity and name are required");
        }
        if let Some(expected) = &self.enrollment_token {
            if token.as_deref() != Some(expected.as_str()) {
                bail!("invalid worker enrollment token");
            }
        }
        Ok((node_id.clone(), name.clone()))
    }

    pub(super) fn connect_worker(
        &self,
        hello: &AgentMessage,
    ) -> Result<(String, String, mpsc::Receiver<AgentMessage>)> {
        let (node_id, name) = self.validate_hello(hello)?;
        let (sender, receiver) = mpsc::channel(128);
        let connection_id = Uuid::new_v4().to_string();
        let mut inner = self.inner.lock().expect("leader cluster lock");
        if inner
            .workers
            .get(&node_id)
            .is_some_and(|worker| worker.online)
        {
            bail!("worker '{node_id}' is already connected");
        }
        let last_seen_ms = current_timestamp_ms();
        let inventory = inner
            .workers
            .get(&node_id)
            .map(|worker| worker.inventory.clone())
            .unwrap_or_default();
        inner.workers.insert(
            node_id.clone(),
            WorkerState {
                name,
                online: true,
                last_seen_ms,
                inventory,
                sender: Some(sender),
                connection_id: Some(connection_id.clone()),
            },
        );
        Ok((node_id, connection_id, receiver))
    }

    pub(super) fn disconnect_worker(&self, node_id: &str, connection_id: &str) {
        let mut inner = self.inner.lock().expect("leader cluster lock");
        if let Some(worker) = inner.workers.get_mut(node_id) {
            if worker.connection_id.as_deref() == Some(connection_id) {
                worker.online = false;
                worker.sender = None;
                worker.connection_id = None;
            }
        }
    }

    pub(super) fn update_inventory(
        &self,
        node_id: &str,
        sessions: Vec<SessionSnapshot>,
        node_metrics: Option<NodeMetricsSample>,
    ) -> Result<()> {
        let now = current_timestamp_ms();
        if let Some(sample) = node_metrics {
            self.node_metrics.push(node_id, sample);
        }
        let (name, inventory_json) = {
            let mut inner = self.inner.lock().expect("leader cluster lock");
            let worker = inner
                .workers
                .get_mut(node_id)
                .with_context(|| format!("worker '{node_id}' is not connected"))?;
            worker.last_seen_ms = now;
            worker.inventory = sessions;
            (
                worker.name.clone(),
                serde_json::to_string(&worker.inventory)?,
            )
        };
        self.store
            .upsert_worker(node_id, &name, now, &inventory_json)
    }

    pub(super) fn heartbeat(&self, node_id: &str, timestamp_ms: u64) {
        if let Some(worker) = self
            .inner
            .lock()
            .expect("leader cluster lock")
            .workers
            .get_mut(node_id)
        {
            worker.last_seen_ms = timestamp_ms.max(current_timestamp_ms());
        }
    }

    pub(super) fn resolve_result(&self, id: &str, response: Response) {
        if let Some(sender) = self
            .inner
            .lock()
            .expect("leader cluster lock")
            .pending
            .remove(id)
        {
            let _ = sender.send(response);
        }
    }

    pub fn remote_nodes(&self) -> Vec<NodeSummary> {
        self.inner
            .lock()
            .expect("leader cluster lock")
            .workers
            .iter()
            .map(|(id, worker)| NodeSummary {
                id: id.clone(),
                name: worker.name.clone(),
                role: "worker".to_string(),
                mode: "local_executor".to_string(),
                online: worker.online,
                is_self: false,
                last_seen_ms: Some(worker.last_seen_ms),
                sessions: worker
                    .inventory
                    .iter()
                    .map(|session| session.name.clone())
                    .collect(),
            })
            .collect()
    }

    pub fn cached_inventory(&self, node_id: &str) -> Option<Vec<SessionSnapshot>> {
        self.inner
            .lock()
            .expect("leader cluster lock")
            .workers
            .get(node_id)
            .map(|worker| worker.inventory.clone())
    }

    pub async fn request_with_audit(
        &self,
        node_id: &str,
        request: RemoteRequest,
        audit: Option<AuditContext>,
    ) -> (Response, AuditStatus) {
        let (sender, command_id) = {
            let inner = self.inner.lock().expect("leader cluster lock");
            let Some(worker) = inner.workers.get(node_id) else {
                return (
                    Response::error(format!("worker '{node_id}' not found")),
                    AuditStatus::Error,
                );
            };
            if !worker.online {
                return (
                    Response::error(format!("worker '{node_id}' is offline")),
                    AuditStatus::Error,
                );
            }
            let Some(sender) = worker.sender.clone() else {
                return (
                    Response::error(format!("worker '{node_id}' has no active connection")),
                    AuditStatus::Error,
                );
            };
            (sender, Uuid::new_v4().to_string())
        };
        let (result_sender, result_receiver) = oneshot::channel();
        self.inner
            .lock()
            .expect("leader cluster lock")
            .pending
            .insert(command_id.clone(), result_sender);
        if sender
            .send(AgentMessage::Command {
                id: command_id.clone(),
                request,
                audit,
            })
            .await
            .is_err()
        {
            self.inner
                .lock()
                .expect("leader cluster lock")
                .pending
                .remove(&command_id);
            return (
                Response::error(format!("worker '{node_id}' disconnected")),
                AuditStatus::Error,
            );
        }
        match tokio::time::timeout(COMMAND_TIMEOUT, result_receiver).await {
            Ok(Ok(response)) => {
                let status = AuditStatus::from_ok(response.ok);
                (response, status)
            }
            Ok(Err(_)) => (
                Response::error(format!("worker '{node_id}' command was cancelled")),
                AuditStatus::Error,
            ),
            Err(_) => {
                self.inner
                    .lock()
                    .expect("leader cluster lock")
                    .pending
                    .remove(&command_id);
                (
                    Response::error(format!("worker '{node_id}' command timed out")),
                    AuditStatus::Timeout,
                )
            }
        }
    }

    pub fn ingest_worker_audits(&self, node_id: &str, records: Vec<AuditRecord>) -> Vec<String> {
        let now = current_timestamp_ms();
        let mut accepted = Vec::new();
        for mut record in records {
            if record.audit_id.trim().is_empty() {
                continue;
            }
            if record
                .executor_node_id
                .as_deref()
                .is_some_and(|value| value != node_id)
            {
                record.details = merge_detail(
                    record.details,
                    serde_json::json!({"reported_executor_node_id": record.executor_node_id}),
                );
            }
            if record.origin_node_id.is_none() {
                record.origin_node_id = Some(node_id.to_string());
            }
            record.executor_node_id = Some(node_id.to_string());
            record.replicated_at_ms = Some(now);
            let audit_id = record.audit_id.clone();
            if self.store.ingest_replicated_audit(record).is_ok()
                && !accepted.iter().any(|accepted_id| accepted_id == &audit_id)
            {
                accepted.push(audit_id);
            }
        }
        accepted
    }

    pub async fn send_to_worker(&self, node_id: &str, message: AgentMessage) -> Result<()> {
        let sender = self
            .inner
            .lock()
            .expect("leader cluster lock")
            .workers
            .get(node_id)
            .and_then(|worker| worker.sender.clone())
            .context("worker has no active sender")?;
        sender
            .send(message)
            .await
            .context("failed to send agent message")
    }
}

pub(super) fn merge_detail(
    mut existing: serde_json::Value,
    patch: serde_json::Value,
) -> serde_json::Value {
    match (existing.as_object_mut(), patch) {
        (Some(existing), serde_json::Value::Object(patch)) => {
            for (key, value) in patch {
                existing.insert(key, value);
            }
            serde_json::Value::Object(existing.clone())
        }
        (_, patch) => serde_json::json!({"existing": existing, "patch": patch}),
    }
}

pub async fn serve_agent_socket(cluster: LeaderCluster, mut socket: WebSocket) {
    let Some(Ok(first)) = socket.recv().await else {
        return;
    };
    let hello = match parse_axum_message(first) {
        Ok(message) => message,
        Err(error) => {
            let _ = send_axum(
                &mut socket,
                &AgentMessage::Error {
                    message: error.to_string(),
                },
            )
            .await;
            return;
        }
    };
    let (node_id, connection_id, mut outgoing) = match cluster.connect_worker(&hello) {
        Ok(connection) => connection,
        Err(error) => {
            let _ = send_axum(
                &mut socket,
                &AgentMessage::Error {
                    message: error.to_string(),
                },
            )
            .await;
            return;
        }
    };
    if send_axum(
        &mut socket,
        &AgentMessage::Welcome {
            protocol: AGENT_PROTOCOL_VERSION,
        },
    )
    .await
    .is_err()
    {
        cluster.disconnect_worker(&node_id, &connection_id);
        return;
    }

    let (mut writer, mut reader) = socket.split();
    let writer_task = tokio::spawn(async move {
        while let Some(message) = outgoing.recv().await {
            let payload = serde_json::to_string(&message)?;
            writer.send(AxumMessage::Text(payload.into())).await?;
        }
        Result::<()>::Ok(())
    });
    while let Some(message) = reader.next().await {
        let message = match message {
            Ok(message) => message,
            Err(_) => break,
        };
        let message = match parse_axum_message(message) {
            Ok(message) => message,
            Err(_) => break,
        };
        match message {
            AgentMessage::Inventory {
                sessions,
                node_metrics,
            } => {
                if cluster
                    .update_inventory(&node_id, sessions, node_metrics)
                    .is_err()
                {
                    break;
                }
            }
            AgentMessage::Heartbeat { timestamp_ms } => {
                cluster.heartbeat(&node_id, timestamp_ms);
            }
            AgentMessage::CommandResult { id, response } => {
                cluster.resolve_result(&id, response);
            }
            AgentMessage::AuditBatch { records } => {
                let audit_ids = cluster.ingest_worker_audits(&node_id, records);
                if !audit_ids.is_empty() {
                    let _ = cluster
                        .send_to_worker(&node_id, AgentMessage::AuditAck { audit_ids })
                        .await;
                }
            }
            _ => break,
        }
    }
    writer_task.abort();
    cluster.disconnect_worker(&node_id, &connection_id);
}
pub(super) async fn send_axum(socket: &mut WebSocket, message: &AgentMessage) -> Result<()> {
    socket
        .send(AxumMessage::Text(serde_json::to_string(message)?.into()))
        .await
        .context("failed to send agent message")
}
