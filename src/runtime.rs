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

fn next_log_generation() -> u64 {
    let counter = NEXT_LOG_GENERATION.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    now.wrapping_add(counter).max(1)
}

struct LogBuffer {
    generation: u64,
    next_seq: u64,
    lines: VecDeque<LogLine>,
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
    fn push(&mut self, stream: &str, text: impl Into<String>) {
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

    fn snapshot(&self, after: Option<u64>, limit: usize) -> TaskLogsSnapshot {
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

    fn clear(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskActionEffect {
    pub task: String,
    pub restarted: bool,
    pub history_cleared: bool,
}

pub struct TaskRuntime {
    spec: TaskSpec,
    status: TaskStatus,
    child: Option<GroupChild>,
    pid: Option<u32>,
    start_generation: u64,
    started_at_ms: u64,
    history_generation: u64,
    last_exit: Option<String>,
    exit_code: Option<i32>,
    logs: Arc<Mutex<LogBuffer>>,
    service: ServiceObservation,
    #[cfg(windows)]
    suspended_threads: Vec<u32>,
}

impl TaskRuntime {
    fn new(spec: TaskSpec) -> Self {
        let service = service::infer_service(&spec);
        Self {
            spec,
            status: TaskStatus::Idle,
            child: None,
            pid: None,
            start_generation: 0,
            started_at_ms: 0,
            history_generation: 1,
            last_exit: None,
            exit_code: None,
            logs: Arc::new(Mutex::new(LogBuffer::default())),
            service,
            #[cfg(windows)]
            suspended_threads: Vec::new(),
        }
    }

    fn push_system(&self, text: impl Into<String>) {
        self.logs.lock().expect("log lock").push("system", text);
    }

    fn reset_runtime_service(&mut self, inspection: ServiceInspectionState) {
        self.service
            .endpoints
            .retain(|endpoint| endpoint.source == "config");
        self.service.inspection = inspection;
    }

    fn start(&mut self) -> Result<()> {
        self.poll()?;
        if self.child.is_some() {
            bail!("task '{}' is already running", self.spec.label);
        }
        if !self.spec.cwd.is_dir() {
            bail!(
                "working directory does not exist: {}",
                self.spec.cwd.display()
            );
        }
        self.reset_runtime_service(ServiceInspectionState::Pending);

        let rendered = self.spec.display_command();
        self.push_system(format!("starting: {rendered}"));
        self.push_system(format!("cwd: {}", display_path(&self.spec.cwd)));

        let mut command = if self.spec.shell {
            #[cfg(unix)]
            {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                let escaped_args = self
                    .spec
                    .args
                    .iter()
                    .map(|part| shell_escape::escape(part.into()).into_owned())
                    .collect::<Vec<_>>();
                let script = if escaped_args.is_empty() {
                    self.spec.program.clone()
                } else {
                    format!("{} {}", self.spec.program, escaped_args.join(" "))
                };
                let mut command = Command::new(shell);
                command.arg("-lc").arg(script);
                command
            }
            #[cfg(windows)]
            {
                let escaped_args = self
                    .spec
                    .args
                    .iter()
                    .map(|part| powershell_quote(part))
                    .collect::<Vec<_>>();
                let script = if escaped_args.is_empty() {
                    self.spec.program.clone()
                } else {
                    format!("{} {}", self.spec.program, escaped_args.join(" "))
                };
                let mut command = Command::new("powershell.exe");
                command
                    .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
                    .arg(script);
                command
            }
        } else {
            let mut command = Command::new(&self.spec.program);
            command.args(&self.spec.args);
            command
        };
        command
            .current_dir(&self.spec.cwd)
            .envs(&self.spec.env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command
            .group_spawn()
            .with_context(|| format!("failed to spawn task '{}': {rendered}", self.spec.label))?;
        let pid = child.inner().id();
        if let Some(stdout) = child.inner().stdout.take() {
            spawn_reader(stdout, "stdout", self.logs.clone());
        }
        if let Some(stderr) = child.inner().stderr.take() {
            spawn_reader(stderr, "stderr", self.logs.clone());
        }
        self.start_generation += 1;
        self.started_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.child = Some(child);
        self.pid = Some(pid);
        self.status = TaskStatus::Running;
        self.last_exit = None;
        self.exit_code = None;
        self.push_system(format!("running (pid {pid})"));
        Ok(())
    }

    #[cfg(unix)]
    fn signal(&self, signal: Signal) -> Result<()> {
        let child = self.child.as_ref().context("task is not running")?;
        let pgid = child.id() as i32;
        killpg(Pid::from_raw(pgid), signal)
            .with_context(|| format!("failed to send {signal:?} to process group {pgid}"))
    }

    fn pause(&mut self) -> Result<()> {
        self.poll()?;
        if self.status != TaskStatus::Running {
            bail!("task '{}' is not running", self.spec.label);
        }
        #[cfg(unix)]
        self.signal(Signal::SIGSTOP)?;
        #[cfg(windows)]
        {
            self.suspended_threads =
                suspend_process_tree(self.pid.context("running task has no process identifier")?)?;
        }
        self.status = TaskStatus::Paused;
        self.push_system("paused");
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        self.poll()?;
        if self.status != TaskStatus::Paused {
            bail!("task '{}' is not paused", self.spec.label);
        }
        #[cfg(unix)]
        self.signal(Signal::SIGCONT)?;
        #[cfg(windows)]
        resume_threads(&mut self.suspended_threads);
        self.status = TaskStatus::Running;
        self.push_system("resumed");
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        self.poll()?;
        let Some(mut child) = self.child.take() else {
            self.status = TaskStatus::Idle;
            self.pid = None;
            self.reset_runtime_service(ServiceInspectionState::NotRunning);
            return Ok(());
        };
        let pid = child.id();
        #[cfg(unix)]
        self.push_system(format!("stopping process group {pid}"));
        #[cfg(windows)]
        self.push_system(format!("stopping process job {pid}"));
        let started = Instant::now();
        let timeout = Duration::from_millis(self.spec.stop_timeout_ms);
        #[cfg(unix)]
        killpg(Pid::from_raw(pid as i32), Signal::SIGTERM).ok();
        #[cfg(windows)]
        child.kill().ok();
        let status = loop {
            if let Some(status) = child.try_wait().context("failed to poll child")? {
                break status;
            }
            if started.elapsed() >= timeout {
                #[cfg(unix)]
                self.push_system("stop timed out; sending SIGKILL");
                #[cfg(windows)]
                self.push_system("stop timed out; terminating process job");
                #[cfg(unix)]
                killpg(Pid::from_raw(pid as i32), Signal::SIGKILL).ok();
                #[cfg(windows)]
                child.kill().ok();
                break child.wait().context("failed to reap child")?;
            }
            thread::sleep(Duration::from_millis(50));
        };
        self.pid = None;
        #[cfg(windows)]
        self.suspended_threads.clear();
        self.status = TaskStatus::Idle;
        self.reset_runtime_service(ServiceInspectionState::NotRunning);
        self.last_exit = Some(status.to_string());
        self.exit_code = exit_code_for(&status);
        self.push_system(format!("stopped ({status})"));
        Ok(())
    }

    fn restart(&mut self) -> Result<bool> {
        self.stop()?;
        let cleared = self.spec.clear_logs_on_restart;
        if cleared {
            self.clear_history();
        }
        self.start()?;
        Ok(cleared)
    }

    fn poll(&mut self) -> Result<()> {
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        if let Some(status) = child.try_wait().context("failed to poll child")? {
            self.status = if status.success() {
                TaskStatus::Exited
            } else {
                TaskStatus::Failed
            };
            self.pid = None;
            self.reset_runtime_service(ServiceInspectionState::NotRunning);
            self.last_exit = Some(status.to_string());
            self.exit_code = exit_code_for(&status);
            self.logs
                .lock()
                .expect("log lock")
                .push("system", format!("exited ({status})"));
            self.child = None;
        }
        Ok(())
    }

    fn apply(&mut self, action: Action) -> Result<(bool, bool)> {
        match action {
            Action::Start => self.start().map(|()| (false, false)),
            Action::Stop => self.stop().map(|()| (false, false)),
            Action::Restart => self.restart().map(|cleared| (true, cleared)),
            Action::Pause => self.pause().map(|()| (false, false)),
            Action::Resume => self.resume().map(|()| (false, false)),
        }
    }

    fn clear_history(&mut self) {
        self.history_generation = self.history_generation.wrapping_add(1).max(1);
        self.logs.lock().expect("log lock").clear();
    }

    fn update_spec(&mut self, spec: TaskSpec) {
        if self.spec != spec {
            self.spec = spec;
            self.service = service::infer_service(&self.spec);
            self.push_system("configuration updated; changes apply on next start");
        }
    }

    fn set_service_observation(
        &mut self,
        endpoints: Vec<ServiceEndpoint>,
        inspection: ServiceInspectionState,
    ) {
        self.service
            .endpoints
            .retain(|endpoint| endpoint.source == "config");
        if !endpoints.is_empty() {
            self.service.endpoints.extend(endpoints);
            self.service.classification = crate::protocol::ServiceClassification::Service;
        }
        self.service.inspection = inspection;
    }

    fn snapshot(&mut self, tail: usize) -> Result<TaskSnapshot> {
        self.poll()?;
        let logs = self.logs.lock().expect("log lock");
        let skip = logs.lines.len().saturating_sub(tail);
        let mut service = self.service.clone();
        if !service
            .endpoints
            .iter()
            .any(|endpoint| endpoint.state == "listening")
        {
            service.endpoints.extend(service::endpoints_from_logs(
                logs.lines
                    .iter()
                    .rev()
                    .take(100)
                    .map(|line| line.text.as_str()),
            ));
            service::deduplicate_endpoints(&mut service.endpoints);
        }
        Ok(TaskSnapshot {
            label: self.spec.label.clone(),
            status: self.status.clone(),
            pid: self.pid,
            command: self.spec.display_command(),
            cwd: self.spec.cwd.clone(),
            auto_start: self.spec.auto_start,
            last_exit: self.last_exit.clone(),
            exit_code: self.exit_code,
            logs: logs.lines.iter().skip(skip).cloned().collect(),
            run_generation: self.start_generation,
            started_at_ms: self.started_at_ms,
            schedule: self.spec.schedule.clone(),
            service,
        })
    }

    fn logs(&mut self, after: Option<u64>, limit: usize) -> Result<TaskLogsSnapshot> {
        self.poll()?;
        Ok(self.logs.lock().expect("log lock").snapshot(after, limit))
    }
}

#[cfg(unix)]
fn exit_code_for(status: &ExitStatus) -> Option<i32> {
    status.code()
}

#[cfg(windows)]
fn exit_code_for(status: &ExitStatus) -> Option<i32> {
    status.code().map(|code| code as i32)
}

#[cfg(windows)]
fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(windows)]
fn process_tree_pids(root_pid: u32) -> Result<std::collections::HashSet<u32>> {
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
fn suspend_process_tree(root_pid: u32) -> Result<Vec<u32>> {
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
fn resume_threads(thread_ids: &mut Vec<u32>) {
    for thread_id in thread_ids.drain(..) {
        let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, thread_id) };
        if thread.is_null() {
            continue;
        }
        unsafe { ResumeThread(thread) };
        unsafe { CloseHandle(thread) };
    }
}

fn spawn_reader<R>(reader: R, stream: &'static str, logs: Arc<Mutex<LogBuffer>>)
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

fn decode_log_line(bytes: &[u8]) -> String {
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

fn display_path(path: &std::path::Path) -> String {
    let path = path.display().to_string();
    #[cfg(windows)]
    return path.strip_prefix(r"\\?\").unwrap_or(&path).to_string();
    #[cfg(not(windows))]
    path
}

pub struct SessionRuntime {
    name: String,
    project: std::path::PathBuf,
    source: String,
    tasks: BTreeMap<String, TaskRuntime>,
    task_order: Vec<String>,
}

impl SessionRuntime {
    pub fn new(definition: ProjectDefinition) -> Self {
        let task_order = definition.task_order;
        Self {
            name: definition.session,
            project: definition.project,
            source: definition.source,
            tasks: definition
                .tasks
                .into_iter()
                .map(|(label, spec)| (label, TaskRuntime::new(spec)))
                .collect(),
            task_order,
        }
    }

    pub fn same_project(&self, project: &std::path::Path) -> bool {
        self.project == project
    }

    pub fn project(&self) -> &std::path::Path {
        &self.project
    }

    pub fn has_task(&self, label: &str) -> bool {
        self.tasks.contains_key(label)
    }

    pub fn task_metric_identity(&self, label: &str) -> Option<(Option<u32>, u64, u64)> {
        self.tasks
            .get(label)
            .map(|task| (task.pid, task.start_generation, task.history_generation))
    }

    pub fn task_root_pids_for_metrics(&mut self) -> Vec<(String, Option<u32>, u64, u64)> {
        self.tasks
            .iter_mut()
            .map(|(label, task)| {
                let pid = match task.poll() {
                    Ok(()) => task.pid,
                    Err(error) => {
                        task.push_system(format!("task state poll error: {error:#}"));
                        None
                    }
                };
                (
                    label.clone(),
                    pid,
                    task.start_generation,
                    task.history_generation,
                )
            })
            .collect()
    }

    pub fn set_service_observation(
        &mut self,
        label: &str,
        endpoints: Vec<ServiceEndpoint>,
        inspection: ServiceInspectionState,
    ) {
        if let Some(task) = self.tasks.get_mut(label) {
            task.set_service_observation(endpoints, inspection);
        }
    }

    pub fn scheduled_start(&mut self, label: &str) -> Result<bool> {
        let task = self.tasks.get_mut(label).with_context(|| {
            format!(
                "scheduled task '{label}' not found in session '{}'",
                self.name
            )
        })?;
        task.poll()?;
        if matches!(task.status, TaskStatus::Running | TaskStatus::Paused) {
            return Ok(false);
        }
        task.start()?;
        Ok(true)
    }

    pub fn auto_start(&mut self) {
        for task in self.tasks.values_mut().filter(|task| task.spec.auto_start) {
            let _ = task.start();
        }
    }

    pub fn update(&mut self, definition: ProjectDefinition) -> Result<()> {
        self.task_order = definition.task_order;
        let updated_tasks = definition.tasks;
        let removed = self
            .tasks
            .keys()
            .filter(|label| !updated_tasks.contains_key(*label))
            .cloned()
            .collect::<Vec<_>>();
        for label in removed {
            self.tasks
                .get_mut(&label)
                .expect("removed task exists")
                .stop()?;
            self.tasks.remove(&label);
        }

        for (label, spec) in updated_tasks {
            if let Some(task) = self.tasks.get_mut(&label) {
                task.update_spec(spec);
            } else {
                let mut task = TaskRuntime::new(spec);
                if task.spec.auto_start {
                    task.start()
                        .with_context(|| format!("failed to auto-start new task '{label}'"))?;
                }
                self.tasks.insert(label, task);
            }
        }

        self.source = definition.source;
        Ok(())
    }

    pub fn apply(&mut self, task: Option<&str>, action: Action) -> Result<Vec<TaskActionEffect>> {
        if let Some(label) = task {
            let (restarted, history_cleared) = self
                .tasks
                .get_mut(label)
                .with_context(|| format!("task '{label}' not found in session '{}'", self.name))?
                .apply(action)?;
            return Ok(vec![TaskActionEffect {
                task: label.to_string(),
                restarted,
                history_cleared,
            }]);
        }
        let mut failures = Vec::new();
        let mut effects = Vec::new();
        for (label, runtime) in &mut self.tasks {
            match runtime.apply(action.clone()) {
                Ok((restarted, history_cleared)) => effects.push(TaskActionEffect {
                    task: label.clone(),
                    restarted,
                    history_cleared,
                }),
                Err(error) => failures.push(format!("{label}: {error}")),
            }
        }
        if failures.is_empty() {
            Ok(effects)
        } else {
            bail!(failures.join("; "))
        }
    }

    pub fn snapshot(&mut self, tail: usize) -> Result<SessionSnapshot> {
        let tasks = self
            .tasks
            .iter_mut()
            .map(|(label, task)| Ok((label.clone(), task.snapshot(tail)?)))
            .collect::<Result<_>>()?;
        Ok(SessionSnapshot {
            name: self.name.clone(),
            project: self.project.clone(),
            source: self.source.clone(),
            tasks,
            task_order: self.task_order.clone(),
        })
    }

    pub fn clear_task_history(&mut self, label: &str) -> Result<u64> {
        let task = self
            .tasks
            .get_mut(label)
            .with_context(|| format!("task '{label}' not found in session '{}'", self.name))?;
        task.clear_history();
        Ok(task.history_generation)
    }

    pub fn task_logs(
        &mut self,
        label: &str,
        after: Option<u64>,
        limit: usize,
    ) -> Result<TaskLogsSnapshot> {
        self.tasks
            .get_mut(label)
            .with_context(|| format!("task '{label}' not found in session '{}'", self.name))?
            .logs(after, limit)
    }

    pub fn stop_all(&mut self) {
        for task in self.tasks.values_mut() {
            let _ = task.stop();
        }
    }
}

pub type Sessions = BTreeMap<String, SessionRuntime>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn long_running_task() -> TaskRuntime {
        TaskRuntime::new(TaskSpec {
            label: "clock".to_string(),
            program: "while true; do echo tick; sleep 0.05; done".to_string(),
            args: Vec::new(),
            cwd: PathBuf::from("/tmp"),
            env: BTreeMap::new(),
            shell: true,
            auto_start: false,
            stop_timeout_ms: 500,
            clear_logs_on_restart: false,
            schedule: None,
        })
    }

    fn task_spec(label: &str, program: &str, auto_start: bool) -> TaskSpec {
        TaskSpec {
            label: label.to_string(),
            program: program.to_string(),
            args: Vec::new(),
            cwd: PathBuf::from("/tmp"),
            env: BTreeMap::new(),
            shell: true,
            auto_start,
            stop_timeout_ms: 500,
            clear_logs_on_restart: false,
            schedule: None,
        }
    }

    #[test]
    fn controls_a_process_group_through_its_lifecycle() {
        let mut task = long_running_task();
        task.start().unwrap();
        assert_eq!(task.status, TaskStatus::Running);
        assert!(task.pid.is_some());

        task.pause().unwrap();
        assert_eq!(task.status, TaskStatus::Paused);
        task.resume().unwrap();
        assert_eq!(task.status, TaskStatus::Running);
        task.restart().unwrap();
        assert_eq!(task.status, TaskStatus::Running);
        task.set_service_observation(
            vec![ServiceEndpoint {
                bind_host: "127.0.0.1".to_string(),
                port: 41023,
                protocol: "tcp".to_string(),
                pid: task.pid,
                source: "socket".to_string(),
                state: "listening".to_string(),
            }],
            ServiceInspectionState::Listening,
        );
        task.stop().unwrap();
        assert_eq!(task.status, TaskStatus::Idle);
        assert!(task.pid.is_none());
        assert_eq!(task.service.inspection, ServiceInspectionState::NotRunning);
        assert!(task.service.endpoints.is_empty());
    }

    #[test]
    fn incremental_logs_reset_when_cursor_falls_behind_or_limit_is_exceeded() {
        let mut logs = LogBuffer::default();
        for index in 0..6 {
            logs.push("stdout", format!("line {index}"));
        }

        let initial = logs.snapshot(None, 3);
        assert!(!initial.reset);
        assert!(initial.generation > 0);
        assert_eq!(
            initial
                .lines
                .iter()
                .map(|line| line.seq)
                .collect::<Vec<_>>(),
            [4, 5, 6]
        );

        let incremental = logs.snapshot(Some(4), 3);
        assert!(!incremental.reset);
        assert_eq!(incremental.generation, initial.generation);
        assert_eq!(
            incremental
                .lines
                .iter()
                .map(|line| line.seq)
                .collect::<Vec<_>>(),
            [5, 6]
        );

        let overflow = logs.snapshot(Some(1), 3);
        assert!(overflow.reset);
        assert_eq!(
            overflow
                .lines
                .iter()
                .map(|line| line.seq)
                .collect::<Vec<_>>(),
            [4, 5, 6]
        );

        let cursor_from_replaced_task = logs.snapshot(Some(60), 3);
        assert!(cursor_from_replaced_task.reset);
        assert_eq!(
            cursor_from_replaced_task
                .lines
                .iter()
                .map(|line| line.seq)
                .collect::<Vec<_>>(),
            [4, 5, 6]
        );

        let replacement = LogBuffer::default().snapshot(None, 3);
        assert_ne!(replacement.generation, initial.generation);
    }

    #[test]
    fn decodes_non_utf8_log_bytes_without_interrupting_the_log_reader() {
        assert_eq!(decode_log_line(b"ready\r\n"), "ready");
        assert!(!decode_log_line(&[0x81, 0x82, b'\n']).is_empty());
    }

    #[test]
    fn preserves_ansi_sequences_for_the_log_renderer() {
        assert_eq!(
            decode_log_line(b"\x1b[32mVITE\x1b[0m ready\n"),
            "\x1b[32mVITE\x1b[0m ready"
        );
    }

    #[test]
    fn updates_tasks_while_preserving_unchanged_runtime_state() {
        let mut runtime = SessionRuntime::new(ProjectDefinition {
            session: "demo".to_string(),
            project: PathBuf::from("/tmp"),
            source: "taskdeck.yaml".to_string(),
            tasks: BTreeMap::from([
                ("keep".to_string(), task_spec("keep", "echo old", false)),
                (
                    "remove".to_string(),
                    task_spec("remove", "echo remove", false),
                ),
            ]),
            task_order: vec!["keep".to_string(), "remove".to_string()],
        });
        runtime
            .tasks
            .get_mut("keep")
            .unwrap()
            .push_system("retained log");

        runtime
            .update(ProjectDefinition {
                session: "demo".to_string(),
                project: PathBuf::from("/tmp"),
                source: ".vscode/tasks.json + taskdeck.yaml".to_string(),
                tasks: BTreeMap::from([
                    ("keep".to_string(), task_spec("keep", "echo new", false)),
                    ("add".to_string(), task_spec("add", "echo add", false)),
                ]),
                task_order: vec!["keep".to_string(), "add".to_string()],
            })
            .unwrap();

        let snapshot = runtime.snapshot(20).unwrap();
        assert_eq!(snapshot.source, ".vscode/tasks.json + taskdeck.yaml");
        assert_eq!(
            snapshot.tasks.keys().cloned().collect::<Vec<_>>(),
            ["add", "keep"]
        );
        assert_eq!(snapshot.tasks["keep"].command, "echo new");
        assert!(
            snapshot.tasks["keep"]
                .logs
                .iter()
                .any(|line| line.text == "retained log")
        );
        assert!(
            snapshot.tasks["keep"]
                .logs
                .iter()
                .any(|line| line.text.contains("configuration updated"))
        );
    }

    #[test]
    fn update_only_auto_starts_new_tasks_and_reports_start_failures() {
        let program = "while true; do sleep 1; done";
        let mut runtime = SessionRuntime::new(ProjectDefinition {
            session: "demo".to_string(),
            project: PathBuf::from("/tmp"),
            source: "taskdeck.yaml".to_string(),
            tasks: BTreeMap::from([("existing".to_string(), task_spec("existing", program, true))]),
            task_order: vec!["existing".to_string()],
        });
        runtime.auto_start();
        runtime
            .apply(Some("existing"), Action::Stop)
            .expect("stop existing auto-start task");

        runtime
            .update(ProjectDefinition {
                session: "demo".to_string(),
                project: PathBuf::from("/tmp"),
                source: "taskdeck.yaml".to_string(),
                tasks: BTreeMap::from([
                    ("existing".to_string(), task_spec("existing", program, true)),
                    ("new".to_string(), task_spec("new", program, true)),
                ]),
                task_order: vec!["existing".to_string(), "new".to_string()],
            })
            .expect("update with new auto-start task");

        let snapshot = runtime.snapshot(20).unwrap();
        assert_eq!(snapshot.tasks["existing"].status, TaskStatus::Idle);
        assert_eq!(snapshot.tasks["new"].status, TaskStatus::Running);
        runtime.stop_all();

        let mut invalid = task_spec("broken", program, true);
        invalid.cwd = PathBuf::from("/tmp/taskdeck-directory-that-does-not-exist");
        let error = runtime
            .update(ProjectDefinition {
                session: "demo".to_string(),
                project: PathBuf::from("/tmp"),
                source: "taskdeck.yaml".to_string(),
                tasks: BTreeMap::from([("broken".to_string(), invalid)]),
                task_order: vec!["broken".to_string()],
            })
            .unwrap_err();
        assert!(error.to_string().contains("failed to auto-start new task"));
        assert!(!runtime.has_task("broken"));
    }

    #[test]
    fn clearing_history_replaces_log_generation_and_invalidates_metric_identity() {
        let mut task = TaskRuntime::new(task_spec("api", "echo ready", false));
        task.push_system("old output");
        let before_logs = task.logs(None, 20).unwrap();
        let before_history = task.history_generation;

        task.clear_history();

        let after_logs = task.logs(None, 20).unwrap();
        assert_ne!(after_logs.generation, before_logs.generation);
        assert!(after_logs.lines.is_empty());
        assert_ne!(task.history_generation, before_history);
    }

    #[test]
    fn restart_clear_setting_replaces_history_between_stop_and_start() {
        let mut task = long_running_task();
        task.spec.clear_logs_on_restart = true;
        task.start().unwrap();
        task.push_system("old output");
        let before_logs = task.logs(None, 20).unwrap();
        let before_history = task.history_generation;

        assert!(task.restart().unwrap());

        let after_logs = task.logs(None, 20).unwrap();
        assert_ne!(after_logs.generation, before_logs.generation);
        assert_ne!(task.history_generation, before_history);
        assert!(
            after_logs
                .lines
                .iter()
                .all(|line| line.text != "old output")
        );
        assert_eq!(task.status, TaskStatus::Running);
        task.stop().unwrap();
    }

    #[test]
    fn snapshot_exposes_configured_order() {
        let mut runtime = SessionRuntime::new(ProjectDefinition {
            session: "demo".to_string(),
            project: PathBuf::from("/tmp"),
            source: "taskdeck.yaml".to_string(),
            tasks: BTreeMap::from([
                ("api".to_string(), task_spec("api", "echo api", false)),
                ("web".to_string(), task_spec("web", "echo web", false)),
            ]),
            task_order: vec!["web".to_string(), "api".to_string()],
        });
        assert_eq!(runtime.snapshot(0).unwrap().task_order, ["web", "api"]);
    }

    #[test]
    fn scheduled_start_runs_once_then_skips_running_task() {
        let mut tasks = BTreeMap::new();
        tasks.insert(
            "one-shot".to_string(),
            TaskSpec {
                label: "one-shot".into(),
                program: "sleep".into(),
                args: vec!["0.05".into()],
                cwd: PathBuf::from("/tmp"),
                env: BTreeMap::new(),
                shell: true,
                auto_start: false,
                stop_timeout_ms: 1000,
                clear_logs_on_restart: false,
                schedule: Some("* * * * *".into()),
            },
        );
        let mut runtime = SessionRuntime::new(ProjectDefinition {
            session: "demo".into(),
            project: PathBuf::from("/tmp"),
            source: "test".into(),
            tasks,
            task_order: vec!["one-shot".into()],
        });
        assert!(runtime.scheduled_start("one-shot").unwrap());
        for _ in 0..50 {
            if let Ok(snapshot) = runtime.snapshot(0) {
                if snapshot.tasks["one-shot"].status == TaskStatus::Running {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!runtime.scheduled_start("one-shot").unwrap());
    }
}
