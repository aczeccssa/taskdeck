//! Ring-buffered task logs.

use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use windows_sys::Win32::Globalization::{CP_ACP, MultiByteToWideChar};

use anyhow::{Context, Result, bail};
use command_group::{CommandGroup, GroupChild};
#[cfg(unix)]
use nix::sys::signal::{Signal, killpg};
#[cfg(unix)]
use nix::unistd::Pid;
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32, Process32First, Process32Next,
            TH32CS_SNAPPROCESS, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        Threading::{OpenThread, ResumeThread, SuspendThread, THREAD_SUSPEND_RESUME},
    },
};

use crate::config::{ProjectDefinition, TaskSpec};
use crate::protocol::{
    Action, LogLine, ServiceEndpoint, ServiceInspectionState, ServiceObservation, SessionSnapshot,
    TaskLogsSnapshot, TaskSnapshot, TaskStatus,
};
use crate::service;


const MAX_LOG_LINES: usize = 5_000;
static NEXT_LOG_GENERATION: AtomicU64 = AtomicU64::new(0);

pub(super) fn next_log_generation() -> u64 {
    let counter = NEXT_LOG_GENERATION.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    now.wrapping_add(counter).max(1)
}

pub(super) struct LogBuffer {
    pub(super) generation: u64,
    pub(super) next_seq: u64,
    pub(super) lines: VecDeque<LogLine>,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self {
            generation: next_log_generation(),
            next_seq: 0,
            lines: VecDeque::new(),
        }
    }
}

impl LogBuffer {
pub(super)     fn push(&mut self, stream: &str, text: impl Into<String>) {
        self.next_seq += 1;
        if self.lines.len() >= MAX_LOG_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(LogLine {
            seq: self.next_seq,
            stream: stream.to_string(),
            text: text.into(),
        });
    }

pub(super)     fn snapshot(&self, after: Option<u64>, limit: usize) -> TaskLogsSnapshot {
        let limit = limit.clamp(1, MAX_LOG_LINES);
        let first_seq = self.lines.front().map(|line| line.seq);
        let mut reset = after.is_some_and(|after| after > self.next_seq)
            || after
                .zip(first_seq)
                .is_some_and(|(after, first)| after < first.saturating_sub(1));
        let candidates = if reset || after.is_none() {
            self.lines.iter().collect::<Vec<_>>()
        } else {
            let after = after.unwrap_or_default();
            self.lines
                .iter()
                .filter(|line| line.seq > after)
                .collect::<Vec<_>>()
        };
        if candidates.len() > limit {
            reset = after.is_some();
        }
        let skip = candidates.len().saturating_sub(limit);
        TaskLogsSnapshot {
            generation: self.generation,
            reset,
            lines: candidates.into_iter().skip(skip).cloned().collect(),
        }
    }

pub(super)     fn clear(&mut self) {
        *self = Self::default();
    }
}

pub(super) fn spawn_reader<R>(reader: R, stream: &'static str, logs: Arc<Mutex<LogBuffer>>)
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut bytes = Vec::new();
        loop {
            bytes.clear();
            match reader.read_until(b'\n', &mut bytes) {
                Ok(0) => break,
                Ok(_) => logs
                    .lock()
                    .expect("log lock")
                    .push(stream, decode_log_line(&bytes)),
                Err(error) => {
                    logs.lock()
                        .expect("log lock")
                        .push("system", format!("log read error: {error}"));
                    break;
                }
            }
        }
    });
}

pub(super) fn decode_log_line(bytes: &[u8]) -> String {
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    if let Ok(line) = std::str::from_utf8(bytes) {
        return line.to_string();
    }
    #[cfg(windows)]
    {
        if let Ok(length) = i32::try_from(bytes.len()) {
            let wide_length = unsafe {
                MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), length, std::ptr::null_mut(), 0)
            };
            if wide_length > 0 {
                let mut wide = vec![0; wide_length as usize];
                let written = unsafe {
                    MultiByteToWideChar(
                        CP_ACP,
                        0,
                        bytes.as_ptr(),
                        length,
                        wide.as_mut_ptr(),
                        wide_length,
                    )
                };
                if written > 0 {
                    return String::from_utf16_lossy(&wide[..written as usize]);
                }
            }
        }
    }
    String::from_utf8_lossy(bytes).into_owned()
}

pub(super) fn display_path(path: &std::path::Path) -> String {
    let path = path.display().to_string();
    #[cfg(windows)]
    return path.strip_prefix(r"\\?\").unwrap_or(&path).to_string();
    #[cfg(not(windows))]
    path
}
