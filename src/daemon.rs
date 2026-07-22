use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::config;
use crate::protocol::{McpCallRecord, Request, Response};
use crate::runtime::{SessionRuntime, Sessions};
use crate::web;

pub const DEFAULT_WEB_HOST: &str = "0.0.0.0";
pub const DEFAULT_WEB_PORT: u16 = 9837;
pub const MAX_MCP_CALLS: usize = 500;

#[derive(Clone)]
pub struct DaemonState {
    pub sessions: Arc<Mutex<Sessions>>,
    pub mcp_calls: Arc<Mutex<VecDeque<McpCallRecord>>>,
    pub next_mcp_call_id: Arc<AtomicU64>,
    pub shutdown: Arc<AtomicBool>,
}

impl DaemonState {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(Sessions::new())),
            mcp_calls: Arc::new(Mutex::new(VecDeque::new())),
            next_mcp_call_id: Arc::new(AtomicU64::new(1)),
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn record_mcp_call(&self, mut record: McpCallRecord) {
        record.id = self.next_mcp_call_id.fetch_add(1, Ordering::Relaxed);
        let mut calls = self.mcp_calls.lock().expect("MCP calls lock");
        calls.push_front(record);
        calls.truncate(MAX_MCP_CALLS);
    }

    pub fn recent_mcp_calls(&self, limit: usize) -> Vec<McpCallRecord> {
        self.mcp_calls
            .lock()
            .expect("MCP calls lock")
            .iter()
            .take(limit.min(MAX_MCP_CALLS))
            .cloned()
            .collect()
    }

    pub fn mcp_call(&self, id: u64) -> Option<McpCallRecord> {
        self.mcp_calls
            .lock()
            .expect("MCP calls lock")
            .iter()
            .find(|call| call.id == id)
            .cloned()
    }
}

pub struct GlobalPaths {
    pub root: PathBuf,
    pub socket: PathBuf,
    pub lock: PathBuf,
    pub log: PathBuf,
}

impl GlobalPaths {
    pub fn discover() -> Result<Self> {
        let root = if let Some(path) = std::env::var_os("TASKDECK_HOME") {
            PathBuf::from(path)
        } else {
            let home = std::env::var_os("HOME").context("HOME is not set")?;
            PathBuf::from(home).join(".taskdeck")
        };
        Ok(Self {
            socket: root.join("taskdeck.sock"),
            lock: root.join("daemon.lock"),
            log: root.join("daemon.log"),
            root,
        })
    }

    pub fn prepare(&self) -> Result<()> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("failed to create {}", self.root.display()))
    }
}

pub async fn run(web_port: u16) -> Result<()> {
    let paths = GlobalPaths::discover()?;
    paths.prepare()?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&paths.lock)
        .context("failed to open daemon lock")?;
    lock.try_lock_exclusive()
        .map_err(|_| anyhow::anyhow!("taskdeck daemon is already running"))?;

    if paths.socket.exists() {
        let _ = fs::remove_file(&paths.socket);
    }
    let listener = UnixListener::bind(&paths.socket)
        .with_context(|| format!("failed to bind {}", paths.socket.display()))?;
    let state = DaemonState::new();
    let web_listener = tokio::net::TcpListener::bind((DEFAULT_WEB_HOST, web_port))
        .await
        .with_context(|| format!("failed to bind Web UI to {DEFAULT_WEB_HOST}:{web_port}"))?;
    let web_state = state.clone();
    let web_task = tokio::spawn(async move { web::serve(web_state, web_listener).await });

    while !state.shutdown.load(Ordering::SeqCst) {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let connection_state = state.clone();
                        tokio::spawn(async move {
                            let _ = serve_connection(connection_state, stream).await;
                        });
                    }
                    Err(error) => eprintln!("IPC accept error: {error}"),
                }
            }
            _ = tokio::signal::ctrl_c() => {
                state.shutdown.store(true, Ordering::SeqCst);
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }

    web_task.abort();
    stop_all(&state);
    let _ = fs::remove_file(&paths.socket);
    drop(lock);
    Ok(())
}

fn stop_all(state: &DaemonState) {
    let mut sessions = state.sessions.lock().expect("sessions lock");
    for session in sessions.values_mut() {
        session.stop_all();
    }
}

async fn serve_connection(state: DaemonState, stream: UnixStream) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => dispatch_async(state.clone(), request).await,
            Err(error) => Response::error(format!("invalid request: {error}")),
        };
        let mut payload = serde_json::to_vec(&response)?;
        payload.push(b'\n');
        writer.write_all(&payload).await?;
    }
    Ok(())
}

pub async fn dispatch_async(state: DaemonState, request: Request) -> Response {
    match tokio::task::spawn_blocking(move || dispatch(&state, request)).await {
        Ok(response) => response,
        Err(error) => Response::error(format!("request worker failed: {error}")),
    }
}

fn dispatch(state: &DaemonState, request: Request) -> Response {
    let result = handle(state, request);
    match result {
        Ok(response) => response,
        Err(error) => Response::error(format!("{error:#}")),
    }
}

fn handle(state: &DaemonState, request: Request) -> Result<Response> {
    match request {
        Request::Ping => Ok(Response::empty("pong")),
        Request::Register { project, session } => {
            let definition = config::discover(&project, session.as_deref())?;
            let name = definition.session.clone();
            let project = definition.project.clone();
            let mut sessions = state.sessions.lock().expect("sessions lock");
            if let Some(existing) = sessions.get_mut(&name) {
                if !existing.same_project(&project) {
                    bail!(
                        "session '{name}' already belongs to another project; choose a different --session"
                    );
                }
                return Ok(Response::ok(
                    format!(
                        "session '{name}' already registered; use update to reload configuration"
                    ),
                    existing.snapshot(200)?,
                ));
            }
            let mut runtime = SessionRuntime::new(definition);
            runtime.auto_start();
            let snapshot = runtime.snapshot(200)?;
            sessions.insert(name.clone(), runtime);
            Ok(Response::ok(
                format!("registered session '{name}'"),
                snapshot,
            ))
        }
        Request::Update { project, session } => {
            let mut definition = config::discover(&project, session.as_deref())?;
            let configured_name = definition.session.clone();
            let project = definition.project.clone();
            let mut sessions = state.sessions.lock().expect("sessions lock");
            let name = if session.is_some() {
                configured_name
            } else {
                let matches = sessions
                    .iter()
                    .filter(|(_, runtime)| runtime.same_project(&project))
                    .map(|(name, _)| name.clone())
                    .collect::<Vec<_>>();
                if matches.iter().any(|name| name == &configured_name) {
                    configured_name
                } else {
                    match matches.as_slice() {
                        [name] => name.clone(),
                        [] => configured_name,
                        _ => bail!(
                            "multiple sessions are registered for project {}; specify --session",
                            project.display()
                        ),
                    }
                }
            };
            let runtime = sessions
                .get_mut(&name)
                .with_context(|| format!("session '{name}' is not registered"))?;
            if !runtime.same_project(&project) {
                bail!("session '{name}' belongs to another project");
            }
            definition.session = name.clone();
            runtime.update(definition)?;
            let snapshot = runtime.snapshot(200)?;
            Ok(Response::ok(format!("updated session '{name}'"), snapshot))
        }
        Request::ListSessions => {
            let sessions = state.sessions.lock().expect("sessions lock");
            let names = sessions.keys().cloned().collect::<Vec<_>>();
            Ok(Response::ok("sessions", names))
        }
        Request::Snapshot { session, tail } => {
            let mut sessions = state.sessions.lock().expect("sessions lock");
            let runtime = sessions
                .get_mut(&session)
                .with_context(|| format!("session '{session}' not found"))?;
            Ok(Response::ok(
                "snapshot",
                runtime.snapshot(tail.unwrap_or(500))?,
            ))
        }
        Request::Action {
            session,
            task,
            action,
        } => {
            let mut sessions = state.sessions.lock().expect("sessions lock");
            let runtime = sessions
                .get_mut(&session)
                .with_context(|| format!("session '{session}' not found"))?;
            runtime.apply(task.as_deref(), action.clone())?;
            Ok(Response::ok(
                format!("{action:?} completed"),
                runtime.snapshot(50)?,
            ))
        }
        Request::RemoveSession { session } => {
            let mut sessions = state.sessions.lock().expect("sessions lock");
            let mut runtime = sessions
                .remove(&session)
                .with_context(|| format!("session '{session}' not found"))?;
            runtime.stop_all();
            Ok(Response::empty(format!("removed session '{session}'")))
        }
        Request::Shutdown => {
            state.shutdown.store(true, Ordering::SeqCst);
            Ok(Response::empty("daemon shutdown requested"))
        }
    }
}

pub async fn request(request: &Request) -> Result<Response> {
    let paths = GlobalPaths::discover()?;
    let stream = UnixStream::connect(&paths.socket)
        .await
        .with_context(|| format!("cannot connect to daemon at {}", paths.socket.display()))?;
    let (reader, mut writer) = stream.into_split();
    let mut payload = serde_json::to_vec(request)?;
    payload.push(b'\n');
    writer.write_all(&payload).await?;
    let mut lines = BufReader::new(reader).lines();
    let line = lines
        .next_line()
        .await?
        .context("daemon closed connection without a response")?;
    serde_json::from_str(&line).context("invalid daemon response")
}

pub async fn is_running() -> bool {
    matches!(request(&Request::Ping).await, Ok(response) if response.ok)
}

pub fn open_daemon_log() -> Result<File> {
    let paths = GlobalPaths::discover()?;
    paths.prepare()?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.log)
        .context("failed to open daemon log")
}

pub fn socket_path() -> Result<PathBuf> {
    Ok(GlobalPaths::discover()?.socket)
}

pub fn root_path() -> Result<PathBuf> {
    Ok(GlobalPaths::discover()?.root)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn call_record() -> McpCallRecord {
        McpCallRecord {
            id: 0,
            tool: "taskdeck_control".to_string(),
            operation: Some("sessions".to_string()),
            started_at_ms: 1,
            duration_ms: 2,
            success: true,
            request: json!({"method": "tools/call"}),
            response: json!({"result": {"isError": false}}),
        }
    }

    #[test]
    fn mcp_call_history_is_newest_first_and_bounded() {
        let state = DaemonState::new();
        for _ in 0..MAX_MCP_CALLS + 2 {
            state.record_mcp_call(call_record());
        }

        let calls = state.recent_mcp_calls(MAX_MCP_CALLS + 20);
        assert_eq!(calls.len(), MAX_MCP_CALLS);
        assert_eq!(calls.first().map(|call| call.id), Some(502));
        assert_eq!(calls.last().map(|call| call.id), Some(3));
        assert!(state.mcp_call(1).is_none());
        assert_eq!(
            state.mcp_call(502).map(|call| call.tool),
            Some("taskdeck_control".to_string())
        );
    }

    #[test]
    fn update_reloads_a_registered_project_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(config::PROJECT_CONFIG);
        fs::write(
            &config_path,
            "version: 1\nsession: demo\ntasks:\n  api:\n    command: echo old\n",
        )
        .unwrap();
        let state = DaemonState::new();

        let registered = handle(
            &state,
            Request::Register {
                project: dir.path().to_path_buf(),
                session: Some("custom".to_string()),
            },
        )
        .unwrap();
        assert!(registered.ok);

        fs::write(
            &config_path,
            "version: 1\nsession: demo\ntasks:\n  api:\n    command: echo new\n  worker:\n    command: echo worker\n",
        )
        .unwrap();
        let updated = handle(
            &state,
            Request::Update {
                project: dir.path().to_path_buf(),
                session: None,
            },
        )
        .unwrap();
        let snapshot: crate::protocol::SessionSnapshot =
            serde_json::from_value(updated.data.unwrap()).unwrap();

        assert_eq!(updated.message, "updated session 'custom'");
        assert_eq!(snapshot.name, "custom");
        assert_eq!(snapshot.tasks["api"].command, "echo new");
        assert!(snapshot.tasks.contains_key("worker"));
    }
}
