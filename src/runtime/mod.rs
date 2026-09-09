//! Task/session process runtimes.

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

mod log_buffer;
mod process;
mod task;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use task::{TaskActionEffect, TaskRuntime};
pub struct SessionRuntime {
    pub(super) name: String,
    pub(super) alias: Option<String>,
    pub(super) project: std::path::PathBuf,
    pub(super) source: String,
    pub(super) tasks: BTreeMap<String, TaskRuntime>,
    pub(super) task_order: Vec<String>,
}

impl SessionRuntime {
    pub fn new(definition: ProjectDefinition) -> Self {
        let task_order = definition.task_order;
        Self {
            name: definition.session,
            alias: None,
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

    pub fn set_alias(&mut self, alias: Option<String>) {
        self.alias = alias;
    }

    pub fn name(&self) -> &str {
        &self.name
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
            match runtime.apply(action) {
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
            alias: self.alias.clone(),
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
