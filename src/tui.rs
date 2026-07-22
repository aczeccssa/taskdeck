use std::io;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::prelude::{Color, Line, Modifier, Span, Style, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Tabs, Wrap};
use ratatui::{Frame, Terminal};

use crate::daemon;
use crate::protocol::{Action, Request, SessionSnapshot, TaskStatus};

struct App {
    snapshot: SessionSnapshot,
    labels: Vec<String>,
    selected: usize,
    scroll: u16,
    follow: bool,
    message: String,
}

impl App {
    fn new(snapshot: SessionSnapshot) -> Self {
        let labels = snapshot.tasks.keys().cloned().collect();
        Self {
            snapshot,
            labels,
            selected: 0,
            scroll: 0,
            follow: true,
            message: "connected".to_string(),
        }
    }

    fn selected_label(&self) -> Option<&str> {
        self.labels.get(self.selected).map(String::as_str)
    }

    fn update(&mut self, snapshot: SessionSnapshot) {
        let selected = self.selected_label().map(str::to_owned);
        self.labels = snapshot.tasks.keys().cloned().collect();
        self.selected = selected
            .and_then(|label| self.labels.iter().position(|item| item == &label))
            .unwrap_or(0);
        self.snapshot = snapshot;
    }

    fn next(&mut self) {
        if !self.labels.is_empty() {
            self.selected = (self.selected + 1) % self.labels.len();
            self.scroll = 0;
        }
    }

    fn previous(&mut self) {
        if !self.labels.is_empty() {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(self.labels.len() - 1);
            self.scroll = 0;
        }
    }
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

pub async fn run(project: &Path, requested_session: Option<String>) -> Result<()> {
    let register = daemon::request(&Request::Register {
        project: project.to_path_buf(),
        session: requested_session,
    })
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
    let _guard = TerminalGuard;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    loop {
        terminal.draw(|frame| render(frame, &app))?;
        if event::poll(Duration::from_millis(150))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match (key.code, key.modifiers) {
                    (KeyCode::Char('c'), modifiers)
                        if modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        break;
                    }
                    (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => break,
                    (KeyCode::Tab, _) | (KeyCode::Right, _) => app.next(),
                    (KeyCode::BackTab, _) | (KeyCode::Left, _) => app.previous(),
                    (KeyCode::Up, _) => {
                        app.follow = false;
                        app.scroll = app.scroll.saturating_sub(1);
                    }
                    (KeyCode::Down, _) => app.scroll = app.scroll.saturating_add(1),
                    (KeyCode::PageUp, _) => {
                        app.follow = false;
                        app.scroll = app.scroll.saturating_sub(10);
                    }
                    (KeyCode::PageDown, _) => app.scroll = app.scroll.saturating_add(10),
                    (KeyCode::End, _) => app.follow = true,
                    (KeyCode::Char('s'), _) => {
                        execute_action(&mut app, &session, Action::Start).await
                    }
                    (KeyCode::Char('r'), _) => {
                        execute_action(&mut app, &session, Action::Restart).await
                    }
                    (KeyCode::Char('x'), _) => {
                        execute_action(&mut app, &session, Action::Stop).await
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
                            execute_action(&mut app, &session, action).await;
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Ok(response) = daemon::request(&Request::Snapshot {
            session: session.clone(),
            tail: Some(1_000),
        })
        .await
        {
            if response.ok {
                if let Some(data) = response.data {
                    if let Ok(snapshot) = serde_json::from_value(data) {
                        app.update(snapshot);
                    }
                }
            } else {
                app.message = response.message;
            }
        }
    }
    Ok(())
}

async fn execute_action(app: &mut App, session: &str, action: Action) {
    let Some(task) = app.selected_label().map(str::to_owned) else {
        return;
    };
    match daemon::request(&Request::Action {
        session: session.to_string(),
        task: Some(task),
        action,
    })
    .await
    {
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
    let (status, status_style, pid, command, cwd) = if let Some(task) = task {
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
        )
    } else {
        (
            "NO TASKS".to_string(),
            Style::default(),
            "-".into(),
            "",
            "".into(),
        )
    };
    let info = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(status, status_style.add_modifier(Modifier::BOLD)),
            Span::raw(format!("  PID {pid}  ")),
            Span::styled(command.to_string(), Style::default().fg(Color::White)),
        ]),
        Line::from(Span::styled(
            format!("cwd: {cwd}"),
            Style::default().fg(Color::DarkGray),
        )),
    ])
    .block(Block::default().borders(Borders::LEFT | Borders::RIGHT));
    frame.render_widget(info, sections[1]);

    let logs = task.map(|task| task.logs.as_slice()).unwrap_or_default();
    let lines = logs
        .iter()
        .map(|line| {
            let style = match line.stream.as_str() {
                "stderr" => Style::default().fg(Color::LightRed),
                "system" => Style::default().fg(Color::Cyan),
                _ => Style::default().fg(Color::Gray),
            };
            Line::from(Span::styled(line.text.clone(), style))
        })
        .collect::<Vec<_>>();
    let inner_height = sections[2].height.saturating_sub(2) as usize;
    let tail_scroll = lines.len().saturating_sub(inner_height) as u16;
    let scroll = if app.follow {
        tail_scroll
    } else {
        app.scroll.min(tail_scroll)
    };
    let output = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title(" Output "))
        .scroll((scroll, 0))
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
