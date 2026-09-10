//! Platform process-tree operations (unix/windows).

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

pub(super) fn exit_code_for(status: &ExitStatus) -> Option<i32> {
    status.code()
}

#[cfg(windows)]
pub(super) fn exit_code_for(status: &ExitStatus) -> Option<i32> {
    status.code().map(|code| code as i32)
}

#[cfg(windows)]
pub(super) fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(windows)]
pub(super) fn process_tree_pids(root_pid: u32) -> Result<std::collections::HashSet<u32>> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error()).context("failed to snapshot processes");
    }
    let mut entries = Vec::new();
    let mut entry = PROCESSENTRY32 {
        dwSize: std::mem::size_of::<PROCESSENTRY32>() as u32,
        ..Default::default()
    };
    let mut has_entry = unsafe { Process32First(snapshot, &mut entry) } != 0;
    while has_entry {
        entries.push((entry.th32ProcessID, entry.th32ParentProcessID));
        has_entry = unsafe { Process32Next(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };

    let mut pids = std::collections::HashSet::from([root_pid]);
    loop {
        let previous_len = pids.len();
        for &(pid, parent) in &entries {
            if pids.contains(&parent) {
                pids.insert(pid);
            }
        }
        if pids.len() == previous_len {
            return Ok(pids);
        }
    }
}

#[cfg(windows)]
pub(super) fn suspend_process_tree(root_pid: u32) -> Result<Vec<u32>> {
    let process_ids = process_tree_pids(root_pid)?;
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error()).context("failed to snapshot process threads");
    }
    let mut suspended = Vec::new();
    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut has_entry = unsafe { Thread32First(snapshot, &mut entry) } != 0;
    while has_entry {
        if process_ids.contains(&entry.th32OwnerProcessID) {
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if !thread.is_null() {
                if unsafe { SuspendThread(thread) } != u32::MAX {
                    suspended.push(entry.th32ThreadID);
                }
                unsafe { CloseHandle(thread) };
            }
        }
        has_entry = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };
    if suspended.is_empty() {
        bail!("failed to suspend any threads in process job {root_pid}");
    }
    Ok(suspended)
}

#[cfg(windows)]
pub(super) fn resume_threads(thread_ids: &mut Vec<u32>) {
    for thread_id in thread_ids.drain(..) {
        let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, thread_id) };
        if thread.is_null() {
            continue;
        }
        unsafe { ResumeThread(thread) };
        unsafe { CloseHandle(thread) };
    }
}
