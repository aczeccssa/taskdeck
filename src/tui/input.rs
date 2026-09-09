//! Keyboard handling and action dispatch.

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

use super::{timed_request, App, RefreshResult};
pub(crate) fn handle_key(

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



pub(crate) fn spawn_action(

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



pub(crate) fn apply_snapshot_result(app: &mut App, result: Result<Response>) {

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



pub(crate) fn apply_logs_result(app: &mut App, result: Result<Response>) {

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



pub(crate) fn apply_action_result(app: &mut App, result: Result<Response>) {

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


