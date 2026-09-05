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
};

const LOG_CACHE_LIMIT: usize = 1_000;
const MAX_RENDER_CHARS: usize = 16_384;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

struct App {
    snapshot: SessionSnapshot,
    labels: Vec<String>,
    selected: usize,
    scroll_from_end: usize,
    follow: bool,
    message: String,
    logs: Vec<LogLine>,
    log_generation: Option<u64>,
    last_log_seq: Option<u64>,
}

impl App {
    fn new(snapshot: SessionSnapshot) -> Self {
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

    fn selected_label(&self) -> Option<&str> {
        self.labels.get(self.selected).map(String::as_str)
    }

    fn update(&mut self, snapshot: SessionSnapshot) {
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

    fn select(&mut self, selected: usize) {
        if selected == self.selected || selected >= self.labels.len() {
            return;
        }
        self.selected = selected;
        self.reset_logs();
    }

    fn next(&mut self) {
        if !self.labels.is_empty() {
            self.select((self.selected + 1) % self.labels.len());
        }
    }

    fn previous(&mut self) {
        if !self.labels.is_empty() {
            self.select(
                self.selected
                    .checked_sub(1)
                    .unwrap_or(self.labels.len() - 1),
            );
        }
    }

    fn reset_logs(&mut self) {
        self.logs.clear();
        self.log_generation = None;
        self.last_log_seq = None;
        self.scroll_from_end = 0;
        self.follow = true;
    }

    fn merge_logs(&mut self, payload: TaskLogsSnapshot) {
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

fn ordered_labels(snapshot: &SessionSnapshot) -> Vec<String> {
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
    running: Arc<AtomicBool>,
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

async fn timed_request(request: Request) -> Result<Response> {
    tokio::time::timeout(
        REQUEST_TIMEOUT,
        daemon::request_from(&request, AuditSource::Tui),
    )
    .await
    .context("daemon request timed out")?
}

fn handle_key(
    app: &mut App,
    key: KeyEvent,
    session: &str,
    refresh_tx: &mpsc::UnboundedSender<RefreshResult>,
    action_inflight: &mut bool,
) -> bool {
    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            return true;
        }
        (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => return true,
        (KeyCode::Tab, _) | (KeyCode::Right, _) => app.next(),
        (KeyCode::BackTab, _) | (KeyCode::Left, _) => app.previous(),
        (KeyCode::Up, _) => {
            app.follow = false;
            app.scroll_from_end = (app.scroll_from_end + 1).min(app.logs.len());
        }
        (KeyCode::Down, _) => {
            app.scroll_from_end = app.scroll_from_end.saturating_sub(1);
            app.follow = app.scroll_from_end == 0;
        }
        (KeyCode::PageUp, _) => {
            app.follow = false;
            app.scroll_from_end = (app.scroll_from_end + 10).min(app.logs.len());
        }
        (KeyCode::PageDown, _) => {
            app.scroll_from_end = app.scroll_from_end.saturating_sub(10);
            app.follow = app.scroll_from_end == 0;
        }
        (KeyCode::End, _) => {
            app.follow = true;
            app.scroll_from_end = 0;
        }
        (KeyCode::Char('s'), _) => {
            spawn_action(app, session, Action::Start, refresh_tx, action_inflight)
        }
        (KeyCode::Char('r'), _) => {
            spawn_action(app, session, Action::Restart, refresh_tx, action_inflight)
        }
        (KeyCode::Char('x'), _) => {
            spawn_action(app, session, Action::Stop, refresh_tx, action_inflight)
        }
        (KeyCode::Char(' '), _) | (KeyCode::Char('p'), _) => {
            let action = app
                .selected_label()
                .and_then(|label| app.snapshot.tasks.get(label))
                .map(|task| {
                    if task.status == TaskStatus::Paused {
                        Action::Resume
                    } else {
                        Action::Pause
                    }
                });
            if let Some(action) = action {
                spawn_action(app, session, action, refresh_tx, action_inflight);
            }
        }
        _ => {}
    }
    false
}

fn spawn_action(
    app: &mut App,
    session: &str,
    action: Action,
    refresh_tx: &mpsc::UnboundedSender<RefreshResult>,
    action_inflight: &mut bool,
) {
    if *action_inflight {
        app.message = "another action is still running".to_string();
        return;
    }
    let Some(task) = app.selected_label().map(str::to_owned) else {
        return;
    };
    *action_inflight = true;
    app.message = format!("{action:?} requested").to_lowercase();
    let tx = refresh_tx.clone();
    let session = session.to_string();
    tokio::spawn(async move {
        let result = timed_request(Request::Action {
            session,
            task: Some(task),
            action,
        })
        .await;
        let _ = tx.send(RefreshResult::Action(result));
    });
}

fn apply_snapshot_result(app: &mut App, result: Result<Response>) {
    match result {
        Ok(response) if response.ok => {
            if let Some(data) = response.data {
                if let Ok(snapshot) = serde_json::from_value(data) {
                    app.update(snapshot);
                }
            }
        }
        Ok(response) => app.message = response.message,
        Err(error) => app.message = error.to_string(),
    }
}

fn apply_logs_result(app: &mut App, result: Result<Response>) {
    match result {
        Ok(response) if response.ok => {
            if let Some(data) = response.data {
                match serde_json::from_value(data) {
                    Ok(payload) => app.merge_logs(payload),
                    Err(error) => app.message = error.to_string(),
                }
            }
        }
        Ok(response) => app.message = response.message,
        Err(error) => app.message = error.to_string(),
    }
}

fn apply_action_result(app: &mut App, result: Result<Response>) {
    match result {
        Ok(response) => {
            app.message = response.message;
            if response.ok {
                if let Some(data) = response.data {
                    if let Ok(snapshot) = serde_json::from_value(data) {
                        app.update(snapshot);
                    }
                }
            }
        }
        Err(error) => app.message = error.to_string(),
    }
}

fn render(frame: &mut Frame, app: &App) {
    let sections = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(4),
        Constraint::Min(6),
        Constraint::Length(2),
    ])
    .split(frame.size());
    let titles = app
        .labels
        .iter()
        .map(|label| Line::from(format!(" {label} ")))
        .collect::<Vec<_>>();
    let tabs = Tabs::new(titles)
        .select(app.selected)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Taskdeck / {} ", app.snapshot.name)),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .divider(Span::raw("|"));
    frame.render_widget(tabs, sections[0]);

    let task = app
        .selected_label()
        .and_then(|label| app.snapshot.tasks.get(label));
    let (status, status_style, pid, command, cwd, schedule, last_exit) = if let Some(task) = task {
        let style = match task.status {
            TaskStatus::Running => Style::default().fg(Color::Green),
            TaskStatus::Paused => Style::default().fg(Color::Yellow),
            TaskStatus::Failed => Style::default().fg(Color::Red),
            _ => Style::default().fg(Color::DarkGray),
        };
        (
            format!("{:?}", task.status).to_uppercase(),
            style,
            task.pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "-".into()),
            task.command.as_str(),
            task.cwd.to_string_lossy(),
            task.schedule.as_deref().unwrap_or(""),
            task.last_exit.as_deref().unwrap_or(""),
        )
    } else {
        (
            "NO TASKS".to_string(),
            Style::default(),
            "-".into(),
            "",
            "".into(),
            "",
            "",
        )
    };
    let mut info_lines = vec![
        Line::from(vec![
            Span::styled(status, status_style.add_modifier(Modifier::BOLD)),
            Span::raw(format!("  PID {pid}  ")),
            Span::styled(command.to_string(), Style::default().fg(Color::White)),
        ]),
        Line::from(Span::styled(
            format!("cwd: {cwd}"),
            Style::default().fg(Color::DarkGray),
        )),
    ];
    if !schedule.is_empty() {
        info_lines.push(Line::from(Span::styled(
            format!("cron: {schedule}"),
            Style::default().fg(Color::DarkGray),
        )));
    }
    if !last_exit.is_empty() {
        info_lines.push(Line::from(Span::styled(
            format!("last exit: {last_exit}"),
            Style::default().fg(Color::DarkGray),
        )));
    }
    let info =
        Paragraph::new(info_lines).block(Block::default().borders(Borders::LEFT | Borders::RIGHT));
    frame.render_widget(info, sections[1]);

    let inner_height = sections[2].height.saturating_sub(2) as usize;
    let render_limit = inner_height.saturating_mul(3).max(1);
    let end = app
        .logs
        .len()
        .saturating_sub(app.scroll_from_end.min(app.logs.len()));
    let start = end.saturating_sub(render_limit);
    let lines = app.logs[start..end]
        .iter()
        .map(|line| {
            let style = match line.stream.as_str() {
                "stderr" => Style::default().fg(Color::LightRed),
                "system" => Style::default().fg(Color::Cyan),
                _ => Style::default().fg(Color::Gray),
            };
            let mut text = line.text.chars().take(MAX_RENDER_CHARS).collect::<String>();
            if line.text.chars().count() > MAX_RENDER_CHARS {
                text.push_str(" ... [truncated]");
            }
            ansi_log_line(&text, style)
        })
        .collect::<Vec<_>>();
    let output = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title(" Output "))
        .wrap(Wrap { trim: false });
    frame.render_widget(output, sections[2]);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            "Tab switch | s start | Space pause/resume | r restart | x stop | q detach",
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("  |  "),
        Span::styled(&app.message, Style::default().fg(Color::Cyan)),
    ]));
    frame.render_widget(footer, sections[3]);
}

fn ansi_log_line(text: &str, base_style: Style) -> Line<'static> {
    let mut spans = Vec::new();
    let mut remaining = text;
    let mut style = base_style;
    while let Some(start) = remaining.find("\u{1b}[") {
        if start > 0 {
            spans.push(Span::styled(remaining[..start].to_string(), style));
        }
        let sequence = &remaining[start + 2..];
        let Some(end) = sequence.find('m') else {
            spans.push(Span::styled(remaining[start..].to_string(), style));
            return Line::from(spans);
        };
        apply_ansi_sgr(&mut style, base_style, &sequence[..end]);
        remaining = &sequence[end + 1..];
    }
    if !remaining.is_empty() || spans.is_empty() {
        spans.push(Span::styled(remaining.to_string(), style));
    }
    Line::from(spans)
}

fn apply_ansi_sgr(style: &mut Style, base_style: Style, sequence: &str) {
    for code in sequence
        .split(';')
        .map(|code| code.parse::<u8>().unwrap_or(0))
    {
        match code {
            0 => *style = base_style,
            1 => *style = style.add_modifier(Modifier::BOLD),
            2 => *style = style.add_modifier(Modifier::DIM),
            22 => *style = style.remove_modifier(Modifier::BOLD | Modifier::DIM),
            30..=37 | 90..=97 => *style = style.fg(ansi_color(code)),
            39 => *style = style.fg(base_style.fg.unwrap_or(Color::Reset)),
            40..=47 | 100..=107 => *style = style.bg(ansi_color(code - 10)),
            49 => *style = style.bg(base_style.bg.unwrap_or(Color::Reset)),
            _ => {}
        }
    }
}

fn ansi_color(code: u8) -> Color {
    match code {
        30 | 40 => Color::Black,
        31 | 41 => Color::Red,
        32 | 42 => Color::Green,
        33 | 43 => Color::Yellow,
        34 | 44 => Color::Blue,
        35 | 45 => Color::Magenta,
        36 | 46 => Color::Cyan,
        37 | 47 => Color::Gray,
        90 | 100 => Color::DarkGray,
        91 | 101 => Color::LightRed,
        92 | 102 => Color::LightGreen,
        93 | 103 => Color::LightYellow,
        94 | 104 => Color::LightBlue,
        95 | 105 => Color::LightMagenta,
        96 | 106 => Color::LightCyan,
        _ => Color::White,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_logs_is_incremental_bounded_and_resets_on_generation_change() {
        let mut app = App::new(SessionSnapshot {
            name: "demo".to_string(),
            project: "/tmp".into(),
            source: "test".to_string(),
            tasks: Default::default(),
            task_order: Vec::new(),
        });
        app.merge_logs(TaskLogsSnapshot {
            generation: 1,
            reset: true,
            lines: (1..=LOG_CACHE_LIMIT as u64).map(log_line).collect(),
        });
        app.merge_logs(TaskLogsSnapshot {
            generation: 1,
            reset: false,
            lines: vec![log_line(1_001)],
        });
        assert_eq!(app.logs.len(), LOG_CACHE_LIMIT);
        assert_eq!(app.logs[0].seq, 2);

        app.merge_logs(TaskLogsSnapshot {
            generation: 2,
            reset: false,
            lines: vec![log_line(1)],
        });
        assert_eq!(
            app.logs.iter().map(|line| line.seq).collect::<Vec<_>>(),
            [1]
        );
    }

    fn log_line(seq: u64) -> LogLine {
        LogLine {
            seq,
            stream: "stdout".to_string(),
            text: format!("line {seq}"),
        }
    }

    #[test]
    fn ansi_log_lines_render_text_without_escape_sequences() {
        let line = ansi_log_line("\u{1b}[32mVITE\u{1b}[0m ready", Style::default());
        assert_eq!(
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "VITE ready"
        );
        assert_eq!(line.spans[0].style.fg, Some(Color::Green));
    }
}
