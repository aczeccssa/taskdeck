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

use crate::daemon::{DaemonState, dispatch_async};
use crate::protocol::{Action, EditableTaskInput, NodeSummary, Request, Response, SessionSnapshot};
use crate::state::{NodeSettings, StateStore};

pub const AGENT_PROTOCOL_VERSION: u32 = 1;
const MAX_AGENT_MESSAGE_BYTES: usize = 2 * 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const COMMAND_CACHE_SIZE: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMessage {
    Hello {
        protocol: u32,
        node_id: String,
        name: String,
        version: String,
        token: Option<String>,
    },
    Welcome {
        protocol: u32,
    },
    Inventory {
        sessions: Vec<SessionSnapshot>,
    },
    Heartbeat {
        timestamp_ms: u64,
    },
    Command {
        id: String,
        request: RemoteRequest,
    },
    CommandResult {
        id: String,
        response: Response,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RemoteRequest {
    ListSessions,
    Snapshot {
        session: String,
        tail: Option<usize>,
    },
    TaskLogs {
        session: String,
        task: String,
        after: Option<u64>,
        limit: usize,
    },
    TaskMetrics {
        session: String,
        task: String,
        window_seconds: usize,
    },
    ClearTaskHistory {
        session: String,
        task: String,
    },
    GetSessionConfig {
        session: String,
    },
    PutSessionConfig {
        session: String,
        revision: String,
        tasks: Vec<EditableTaskInput>,
    },
    Action {
        session: String,
        task: Option<String>,
        action: Action,
    },
    RemoveSession {
        session: String,
    },
}

impl RemoteRequest {
    pub fn into_local(self) -> Request {
        match self {
            Self::ListSessions => Request::ListSessions,
            Self::Snapshot { session, tail } => Request::Snapshot { session, tail },
            Self::TaskLogs {
                session,
                task,
                after,
                limit,
            } => Request::TaskLogs {
                session,
                task,
                after,
                limit,
            },
            Self::TaskMetrics {
                session,
                task,
                window_seconds,
            } => Request::TaskMetrics {
                session,
                task,
                window_seconds,
            },
            Self::ClearTaskHistory { session, task } => Request::ClearTaskHistory { session, task },
            Self::GetSessionConfig { session } => Request::GetSessionConfig { session },
            Self::PutSessionConfig {
                session,
                revision,
                tasks,
            } => Request::PutSessionConfig {
                session,
                revision,
                tasks,
            },
            Self::Action {
                session,
                task,
                action,
            } => Request::Action {
                session,
                task,
                action,
            },
            Self::RemoveSession { session } => Request::RemoveSession { session },
        }
    }
}

#[derive(Clone)]
pub struct LeaderCluster {
    inner: Arc<Mutex<LeaderClusterInner>>,
    enrollment_token: Option<String>,
    store: Arc<StateStore>,
}

struct LeaderClusterInner {
    workers: BTreeMap<String, WorkerState>,
    pending: HashMap<String, oneshot::Sender<Response>>,
}

struct WorkerState {
    name: String,
    online: bool,
    last_seen_ms: u64,
    inventory: Vec<SessionSnapshot>,
    sender: Option<mpsc::Sender<AgentMessage>>,
    connection_id: Option<String>,
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
        })
    }

    fn validate_hello(&self, hello: &AgentMessage) -> Result<(String, String)> {
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

    fn connect_worker(
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

    fn disconnect_worker(&self, node_id: &str, connection_id: &str) {
        let mut inner = self.inner.lock().expect("leader cluster lock");
        if let Some(worker) = inner.workers.get_mut(node_id) {
            if worker.connection_id.as_deref() == Some(connection_id) {
                worker.online = false;
                worker.sender = None;
                worker.connection_id = None;
            }
        }
    }

    fn update_inventory(&self, node_id: &str, sessions: Vec<SessionSnapshot>) -> Result<()> {
        let now = current_timestamp_ms();
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

    fn heartbeat(&self, node_id: &str, timestamp_ms: u64) {
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

    fn resolve_result(&self, id: &str, response: Response) {
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

    pub async fn request(&self, node_id: &str, request: RemoteRequest) -> Response {
        let (sender, command_id) = {
            let inner = self.inner.lock().expect("leader cluster lock");
            let Some(worker) = inner.workers.get(node_id) else {
                return Response::error(format!("worker '{node_id}' not found"));
            };
            if !worker.online {
                return Response::error(format!("worker '{node_id}' is offline"));
            }
            let Some(sender) = worker.sender.clone() else {
                return Response::error(format!("worker '{node_id}' has no active connection"));
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
            })
            .await
            .is_err()
        {
            self.inner
                .lock()
                .expect("leader cluster lock")
                .pending
                .remove(&command_id);
            return Response::error(format!("worker '{node_id}' disconnected"));
        }
        match tokio::time::timeout(COMMAND_TIMEOUT, result_receiver).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => Response::error(format!("worker '{node_id}' command was cancelled")),
            Err(_) => {
                self.inner
                    .lock()
                    .expect("leader cluster lock")
                    .pending
                    .remove(&command_id);
                Response::error(format!("worker '{node_id}' command timed out"))
            }
        }
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
            AgentMessage::Inventory { sessions } => {
                if cluster.update_inventory(&node_id, sessions).is_err() {
                    break;
                }
            }
            AgentMessage::Heartbeat { timestamp_ms } => {
                cluster.heartbeat(&node_id, timestamp_ms);
            }
            AgentMessage::CommandResult { id, response } => {
                cluster.resolve_result(&id, response);
            }
            _ => break,
        }
    }
    writer_task.abort();
    cluster.disconnect_worker(&node_id, &connection_id);
}

fn parse_axum_message(message: AxumMessage) -> Result<AgentMessage> {
    let text = match message {
        AxumMessage::Text(text) => text,
        _ => bail!("agent messages must be text JSON"),
    };
    if text.len() > MAX_AGENT_MESSAGE_BYTES {
        bail!("agent message exceeds size limit");
    }
    serde_json::from_str(&text).context("invalid agent message")
}

async fn send_axum(socket: &mut WebSocket, message: &AgentMessage) -> Result<()> {
    socket
        .send(AxumMessage::Text(serde_json::to_string(message)?.into()))
        .await
        .context("failed to send agent message")
}

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

async fn run_worker_connection(
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

    let mut interval = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                send_worker(&mut writer, &AgentMessage::Inventory {
                    sessions: state.local_inventory(),
                }).await?;
                send_worker(&mut writer, &AgentMessage::Heartbeat {
                    timestamp_ms: current_timestamp_ms(),
                }).await?;
            }
            message = reader.next() => {
                let Some(message) = message else { bail!("leader connection closed"); };
                let message = parse_worker_message(message?)?;
                let AgentMessage::Command { id, request } = message else {
                    bail!("unexpected leader message");
                };
                let cached = cache.lock().expect("command cache lock").get(&id);
                let response = if let Some(response) = cached {
                    response
                } else {
                    let response = dispatch_async(state.clone(), request.into_local()).await;
                    cache.lock().expect("command cache lock").insert(id.clone(), response.clone());
                    response
                };
                send_worker(&mut writer, &AgentMessage::CommandResult { id, response }).await?;
            }
        }
    }
}

async fn send_worker<S>(writer: &mut S, message: &AgentMessage) -> Result<()>
where
    S: futures_util::Sink<WsMessage> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    writer
        .send(WsMessage::Text(serde_json::to_string(message)?.into()))
        .await
        .context("failed to send worker message")
}

fn parse_worker_message(message: WsMessage) -> Result<AgentMessage> {
    let text = match message {
        WsMessage::Text(text) => text,
        _ => bail!("leader messages must be text JSON"),
    };
    if text.len() > MAX_AGENT_MESSAGE_BYTES {
        bail!("leader message exceeds size limit");
    }
    serde_json::from_str(&text).context("invalid leader message")
}

fn agent_url(leader_url: &str) -> Result<String> {
    let mut url = leader_url.trim_end_matches('/').to_string();
    if let Some(rest) = url.strip_prefix("http://") {
        url = format!("ws://{rest}");
    } else if let Some(rest) = url.strip_prefix("https://") {
        url = format!("wss://{rest}");
    } else if !url.starts_with("ws://") && !url.starts_with("wss://") {
        bail!("leader URL must use http, https, ws, or wss");
    }
    Ok(format!("{url}/api/agent/connect"))
}

struct CommandResultCache {
    capacity: usize,
    entries: VecDeque<(String, Response)>,
}

impl CommandResultCache {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: VecDeque::new(),
        }
    }

    fn get(&self, id: &str) -> Option<Response> {
        self.entries
            .iter()
            .find(|(entry_id, _)| entry_id == id)
            .map(|(_, response)| response.clone())
    }

    fn insert(&mut self, id: String, response: Response) {
        self.entries.push_back((id, response));
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }
}

fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_command_returns_cached_result() {
        let mut cache = CommandResultCache::new(2);
        cache.insert("cmd-1".to_string(), Response::empty("done"));
        assert_eq!(cache.get("cmd-1").unwrap().message, "done");
    }

    #[test]
    fn rejects_wrong_enrollment_token_and_protocol() {
        let store = Arc::new(StateStore::open_in_memory().unwrap());
        let cluster = LeaderCluster::new(store, Some("secret".to_string())).unwrap();
        let wrong_token = AgentMessage::Hello {
            protocol: AGENT_PROTOCOL_VERSION,
            node_id: "worker-1".to_string(),
            name: "worker".to_string(),
            version: "test".to_string(),
            token: Some("wrong".to_string()),
        };
        assert!(cluster.validate_hello(&wrong_token).is_err());

        let wrong_protocol = AgentMessage::Hello {
            protocol: 99,
            node_id: "worker-1".to_string(),
            name: "worker".to_string(),
            version: "test".to_string(),
            token: Some("secret".to_string()),
        };
        assert!(cluster.validate_hello(&wrong_protocol).is_err());
    }

    #[test]
    fn builds_agent_urls_for_http_and_websocket_leaders() {
        assert_eq!(
            agent_url("http://leader:9837").unwrap(),
            "ws://leader:9837/api/agent/connect"
        );
        assert_eq!(
            agent_url("wss://leader.example/").unwrap(),
            "wss://leader.example/api/agent/connect"
        );
    }
}
