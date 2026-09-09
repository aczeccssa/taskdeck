//! Terminal UI application state and event loop.

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::prelude::{Color, Line, Modifier, Span, Style, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Tabs, Wrap};
use ratatui::{Frame, Terminal};
use tokio::sync::mpsc;

use crate::daemon;
use crate::protocol::{
    Action, AuditSource, LogLine, Request, Response, SessionSnapshot, TaskLogsSnapshot, TaskStatus,
    WorkspaceSummary,
};

const LOG_CACHE_LIMIT: usize = 1_000;
const MAX_RENDER_CHARS: usize = 16_384;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) struct App {
    pub(crate) snapshot: SessionSnapshot,
    pub(crate) labels: Vec<String>,
    pub(crate) selected: usize,
    pub(crate) scroll_from_end: usize,
    pub(crate) follow: bool,
    pub(crate) message: String,
    pub(crate) logs: Vec<LogLine>,
    pub(crate) log_generation: Option<u64>,
    pub(crate) last_log_seq: Option<u64>,
}

impl App {
    pub(crate) fn new(snapshot: SessionSnapshot) -> Self {
        let labels = ordered_labels(&snapshot);
        let logs = labels
            .first()
            .and_then(|label| snapshot.tasks.get(label))
            .map(|task| task.logs.clone())
            .unwrap_or_default();
        let last_log_seq = logs.last().map(|line| line.seq);
        Self {
            snapshot,
            labels,
            selected: 0,
            scroll_from_end: 0,
            follow: true,
            message: "connected".to_string(),
            logs,
            log_generation: None,
            last_log_seq,
        }
    }

    pub(crate) fn display_name(&self) -> &str {
        self.snapshot
            .alias
            .as_deref()
            .unwrap_or(&self.snapshot.name)
    }

    pub(crate) fn selected_label(&self) -> Option<&str> {
        self.labels.get(self.selected).map(String::as_str)
    }

    pub(crate) fn update(&mut self, snapshot: SessionSnapshot) {
        let selected = self.selected_label().map(str::to_owned);
        self.labels = ordered_labels(&snapshot);
        self.selected = selected
            .as_ref()
            .and_then(|label| self.labels.iter().position(|item| item == label))
            .unwrap_or(0);
        if selected.as_deref() != self.selected_label() {
            self.reset_logs();
        }
        self.snapshot = snapshot;
    }

    pub(crate) fn select(&mut self, selected: usize) {
        if selected == self.selected || selected >= self.labels.len() {
            return;
        }
        self.selected = selected;
        self.reset_logs();
    }

    pub(crate) fn next(&mut self) {
        if !self.labels.is_empty() {
            self.select((self.selected + 1) % self.labels.len());
        }
    }

    pub(crate) fn previous(&mut self) {
        if !self.labels.is_empty() {
            self.select(
                self.selected
                    .checked_sub(1)
                    .unwrap_or(self.labels.len() - 1),
            );
        }
    }

    pub(crate) fn reset_logs(&mut self) {
        self.logs.clear();
        self.log_generation = None;
        self.last_log_seq = None;
        self.scroll_from_end = 0;
        self.follow = true;
    }

    pub(crate) fn merge_logs(&mut self, payload: TaskLogsSnapshot) {
        let generation_changed = self
            .log_generation
            .is_some_and(|generation| generation != payload.generation);
        if payload.reset || generation_changed || self.last_log_seq.is_none() {
            self.logs = payload.lines;
        } else {
            self.logs.extend(payload.lines);
        }
        if self.logs.len() > LOG_CACHE_LIMIT {
            let remove = self.logs.len() - LOG_CACHE_LIMIT;
            self.logs.drain(..remove);
        }
        self.log_generation = Some(payload.generation);
        self.last_log_seq = self.logs.last().map(|line| line.seq);
        if self.follow {
            self.scroll_from_end = 0;
        }
    }
}

pub(crate) fn ordered_labels(snapshot: &SessionSnapshot) -> Vec<String> {
    let mut labels = snapshot
        .task_order
        .iter()
        .filter(|label| snapshot.tasks.contains_key(*label))
        .cloned()
        .collect::<Vec<_>>();
    for label in snapshot.tasks.keys() {
        if !labels.contains(label) {
            labels.push(label.clone());
        }
    }
    labels
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

struct InputGuard {
    pub(crate) running: Arc<AtomicBool>,
}

impl Drop for InputGuard {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

enum RefreshResult {
    Snapshot(Result<Response>),
    Logs {
        task: String,
        result: Result<Response>,
    },
    Action(Result<Response>),
}

pub async fn run(project: &Path, requested_session: Option<String>) -> Result<()> {
    let register = daemon::request_from(
        &Request::Register {
            project: project.to_path_buf(),
            session: requested_session,
        },
        AuditSource::Tui,
    )
    .await?;
    if !register.ok {
        bail!(register.message);
    }
    let snapshot: SessionSnapshot = serde_json::from_value(
        register
            .data
            .context("register response did not include a snapshot")?,
    )?;
    let session = snapshot.name.clone();
    let mut app = App::new(snapshot);
    if let Ok(response) = daemon::request(&Request::ListWorkspaces).await {
        if response.ok {
            if let Some(data) = response.data {
                if let Ok(workspaces) = serde_json::from_value::<Vec<WorkspaceSummary>>(data) {
                    if let Some(alias) = workspaces
                        .iter()
                        .find(|workspace| workspace.session == session)
                        .and_then(|workspace| workspace.alias.clone())
                    {
                        app.snapshot.alias = Some(alias);
                    }
                }
            }
        }
    }

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let _terminal_guard = TerminalGuard;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let (key_tx, mut key_rx) = mpsc::unbounded_channel();
    let input_running = Arc::new(AtomicBool::new(true));
    let _input_guard = InputGuard {
        running: input_running.clone(),
    };
    std::thread::spawn(move || {
        while input_running.load(Ordering::SeqCst) {
            if event::poll(Duration::from_millis(50)).unwrap_or(false) {
                match event::read() {
                    Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                        if key_tx.send(key).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        }
    });

    let (refresh_tx, mut refresh_rx) = mpsc::unbounded_channel();
    let mut status_tick = tokio::time::interval(Duration::from_secs(1));
    let mut log_tick = tokio::time::interval(Duration::from_millis(250));
    status_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    log_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut status_inflight = false;
    let mut logs_inflight = false;
    let mut action_inflight = false;
    let mut dirty = true;

    loop {
        if dirty {
            terminal.draw(|frame| render(frame, &app))?;
            dirty = false;
        }
        tokio::select! {
            key = key_rx.recv() => {
                let Some(key) = key else { break };
                if handle_key(&mut app, key, &session, &refresh_tx, &mut action_inflight) {
                    break;
                }
                dirty = true;
            }
            _ = status_tick.tick(), if !status_inflight => {
                status_inflight = true;
                let tx = refresh_tx.clone();
                let session = session.clone();
                tokio::spawn(async move {
                    let result = timed_request(Request::Snapshot { session, tail: Some(0) }).await;
                    let _ = tx.send(RefreshResult::Snapshot(result));
                });
            }
            _ = log_tick.tick(), if !logs_inflight && app.selected_label().is_some() => {
                logs_inflight = true;
                let tx = refresh_tx.clone();
                let session = session.clone();
                let task = app.selected_label().expect("selected task").to_string();
                let after = app.last_log_seq;
                tokio::spawn(async move {
                    let result = timed_request(Request::TaskLogs {
                        session,
                        task: task.clone(),
                        after,
                        limit: LOG_CACHE_LIMIT,
                    }).await;
                    let _ = tx.send(RefreshResult::Logs { task, result });
                });
            }
            result = refresh_rx.recv() => {
                let Some(result) = result else { break };
                match result {
                    RefreshResult::Snapshot(result) => {
                        status_inflight = false;
                        apply_snapshot_result(&mut app, result);
                    }
                    RefreshResult::Logs { task, result } => {
                        logs_inflight = false;
                        if app.selected_label() == Some(task.as_str()) {
                            apply_logs_result(&mut app, result);
                        }
                    }
                    RefreshResult::Action(result) => {
                        action_inflight = false;
                        apply_action_result(&mut app, result);
                    }
                }
                dirty = true;
            }
        }
    }
    Ok(())
}

pub(crate) async fn timed_request(request: Request) -> Result<Response> {
    tokio::time::timeout(
        REQUEST_TIMEOUT,
        daemon::request_from(&request, AuditSource::Tui),
    )
    .await
    .context("daemon request timed out")?
}

mod input;
use input::{apply_action_result, apply_logs_result, apply_snapshot_result, handle_key};
use render::render;
mod render;

#[cfg(test)]
mod tests;
