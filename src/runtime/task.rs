//! Single-task process runtime.

use super::log_buffer::{LogBuffer, display_path, spawn_reader};
use super::process::exit_code_for;
#[cfg(windows)]
use super::process::{powershell_quote, process_tree_pids, resume_threads, suspend_process_tree};
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

pub struct TaskActionEffect {
    pub task: String,
    pub restarted: bool,
    pub history_cleared: bool,
}

pub struct TaskRuntime {
    pub(super) spec: TaskSpec,
    pub(super) status: TaskStatus,
    pub(super) child: Option<GroupChild>,
    pub(super) pid: Option<u32>,
    pub(super) start_generation: u64,
    pub(super) started_at_ms: u64,
    pub(super) history_generation: u64,
    pub(super) last_exit: Option<String>,
    pub(super) exit_code: Option<i32>,
    pub(super) logs: Arc<Mutex<LogBuffer>>,
    pub(super) service: ServiceObservation,
    #[cfg(windows)]
    pub(super) suspended_threads: Vec<u32>,
}

impl TaskRuntime {
    pub(super) fn new(spec: TaskSpec) -> Self {
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

    pub(super) fn push_system(&self, text: impl Into<String>) {
        self.logs.lock().expect("log lock").push("system", text);
    }

    pub(super) fn reset_runtime_service(&mut self, inspection: ServiceInspectionState) {
        self.service
            .endpoints
            .retain(|endpoint| endpoint.source == "config");
        self.service.inspection = inspection;
    }

    pub(super) fn start(&mut self) -> Result<()> {
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
    pub(super) fn signal(&self, signal: Signal) -> Result<()> {
        let child = self.child.as_ref().context("task is not running")?;
        let pgid = child.id() as i32;
        killpg(Pid::from_raw(pgid), signal)
            .with_context(|| format!("failed to send {signal:?} to process group {pgid}"))
    }

    pub(super) fn pause(&mut self) -> Result<()> {
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

    pub(super) fn resume(&mut self) -> Result<()> {
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

    pub(super) fn stop(&mut self) -> Result<()> {
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

    pub(super) fn restart(&mut self) -> Result<bool> {
        self.stop()?;
        let cleared = self.spec.clear_logs_on_restart;
        if cleared {
            self.clear_history();
        }
        self.start()?;
        Ok(cleared)
    }

    pub(super) fn poll(&mut self) -> Result<()> {
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

    pub(super) fn apply(&mut self, action: Action) -> Result<(bool, bool)> {
        match action {
            Action::Start => self.start().map(|()| (false, false)),
            Action::Stop => self.stop().map(|()| (false, false)),
            Action::Restart => self.restart().map(|cleared| (true, cleared)),
            Action::Pause => self.pause().map(|()| (false, false)),
            Action::Resume => self.resume().map(|()| (false, false)),
        }
    }

    pub(super) fn clear_history(&mut self) {
        self.history_generation = self.history_generation.wrapping_add(1).max(1);
        self.logs.lock().expect("log lock").clear();
    }

    pub(super) fn update_spec(&mut self, spec: TaskSpec) {
        if self.spec != spec {
            self.spec = spec;
            self.service = service::infer_service(&self.spec);
            self.push_system("configuration updated; changes apply on next start");
        }
    }

    pub(super) fn set_service_observation(
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

    pub(super) fn snapshot(&mut self, tail: usize) -> Result<TaskSnapshot> {
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

    pub(super) fn logs(&mut self, after: Option<u64>, limit: usize) -> Result<TaskLogsSnapshot> {
        self.poll()?;
        Ok(self.logs.lock().expect("log lock").snapshot(after, limit))
    }
}
