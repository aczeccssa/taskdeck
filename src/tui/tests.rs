//! TUI tests.

use super::super::*;
use super::render::ansi_log_line;
use super::*;

#[test]
fn merge_logs_is_incremental_bounded_and_resets_on_generation_change() {
    let mut app = App::new(SessionSnapshot {
        name: "demo".to_string(),
        alias: None,
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
