//! Ratatui rendering and ANSI handling.

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

use super::App;
pub(crate) fn render(frame: &mut Frame, app: &App) {
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
                .title(format!(" Taskdeck / {} ", app.display_name())),
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
            format!("session: {}  cwd: {cwd}", app.snapshot.name),
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

pub(crate) fn ansi_log_line(text: &str, base_style: Style) -> Line<'static> {
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

pub(crate) fn apply_ansi_sgr(style: &mut Style, base_style: Style, sequence: &str) {
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

pub(crate) fn ansi_color(code: u8) -> Color {
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
