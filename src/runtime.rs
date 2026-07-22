use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use command_group::{CommandGroup, GroupChild};
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;

use crate::config::{ProjectDefinition, TaskSpec};
use crate::protocol::{Action, LogLine, SessionSnapshot, TaskSnapshot, TaskStatus};

const MAX_LOG_LINES: usize = 5_000;

#[derive(Default)]
struct LogBuffer {
    next_seq: u64,
    lines: VecDeque<LogLine>,
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
}

pub struct TaskRuntime {
    spec: TaskSpec,
    status: TaskStatus,
    child: Option<GroupChild>,
    pid: Option<u32>,
    last_exit: Option<String>,
    logs: Arc<Mutex<LogBuffer>>,
}

impl TaskRuntime {
    fn new(spec: TaskSpec) -> Self {
        Self {
            spec,
            status: TaskStatus::Idle,
            child: None,
            pid: None,
            last_exit: None,
            logs: Arc::new(Mutex::new(LogBuffer::default())),
        }
    }

    fn push_system(&self, text: impl Into<String>) {
        self.logs.lock().expect("log lock").push("system", text);
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

        let rendered = self.spec.display_command();
        self.push_system(format!("starting: {rendered}"));
        self.push_system(format!("cwd: {}", self.spec.cwd.display()));

        let mut command = if self.spec.shell {
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
        self.child = Some(child);
        self.pid = Some(pid);
        self.status = TaskStatus::Running;
        self.last_exit = None;
        self.push_system(format!("running (pid {pid})"));
        Ok(())
    }

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
        self.signal(Signal::SIGSTOP)?;
        self.status = TaskStatus::Paused;
        self.push_system("paused");
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        self.poll()?;
        if self.status != TaskStatus::Paused {
            bail!("task '{}' is not paused", self.spec.label);
        }
        self.signal(Signal::SIGCONT)?;
        self.status = TaskStatus::Running;
        self.push_system("resumed");
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        self.poll()?;
        let Some(mut child) = self.child.take() else {
            self.status = TaskStatus::Idle;
            self.pid = None;
            return Ok(());
        };
        let pgid = child.id() as i32;
        self.push_system(format!("stopping process group {pgid}"));
        killpg(Pid::from_raw(pgid), Signal::SIGTERM).ok();
        let deadline = Instant::now() + Duration::from_millis(self.spec.stop_timeout_ms);
        let status = loop {
            if let Some(status) = child.try_wait().context("failed to poll child")? {
                break status;
            }
            if Instant::now() >= deadline {
                self.push_system("stop timed out; sending SIGKILL");
                killpg(Pid::from_raw(pgid), Signal::SIGKILL).ok();
                break child.wait().context("failed to reap child")?;
            }
            thread::sleep(Duration::from_millis(50));
        };
        self.pid = None;
        self.status = TaskStatus::Idle;
        self.last_exit = Some(status.to_string());
        self.push_system(format!("stopped ({status})"));
        Ok(())
    }

    fn restart(&mut self) -> Result<()> {
        self.stop()?;
        self.start()
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
            self.last_exit = Some(status.to_string());
            self.logs
                .lock()
                .expect("log lock")
                .push("system", format!("exited ({status})"));
            self.child = None;
        }
        Ok(())
    }

    fn apply(&mut self, action: Action) -> Result<()> {
        match action {
            Action::Start => self.start(),
            Action::Stop => self.stop(),
            Action::Restart => self.restart(),
            Action::Pause => self.pause(),
            Action::Resume => self.resume(),
        }
    }

    fn update_spec(&mut self, spec: TaskSpec) {
        if self.spec != spec {
            self.spec = spec;
            self.push_system("configuration updated; changes apply on next start");
        }
    }

    fn snapshot(&mut self, tail: usize) -> Result<TaskSnapshot> {
        self.poll()?;
        let logs = self.logs.lock().expect("log lock");
        let skip = logs.lines.len().saturating_sub(tail);
        Ok(TaskSnapshot {
            label: self.spec.label.clone(),
            status: self.status.clone(),
            pid: self.pid,
            command: self.spec.display_command(),
            cwd: self.spec.cwd.clone(),
            auto_start: self.spec.auto_start,
            last_exit: self.last_exit.clone(),
            logs: logs.lines.iter().skip(skip).cloned().collect(),
        })
    }
}

fn spawn_reader<R>(reader: R, stream: &'static str, logs: Arc<Mutex<LogBuffer>>)
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            match line {
                Ok(line) => logs.lock().expect("log lock").push(stream, line),
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

pub struct SessionRuntime {
    name: String,
    project: std::path::PathBuf,
    source: String,
    tasks: BTreeMap<String, TaskRuntime>,
}

impl SessionRuntime {
    pub fn new(definition: ProjectDefinition) -> Self {
        Self {
            name: definition.session,
            project: definition.project,
            source: definition.source,
            tasks: definition
                .tasks
                .into_iter()
                .map(|(label, spec)| (label, TaskRuntime::new(spec)))
                .collect(),
        }
    }

    pub fn same_project(&self, project: &std::path::Path) -> bool {
        self.project == project
    }

    pub fn auto_start(&mut self) {
        for task in self.tasks.values_mut().filter(|task| task.spec.auto_start) {
            let _ = task.start();
        }
    }

    pub fn update(&mut self, definition: ProjectDefinition) -> Result<()> {
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
                self.tasks.insert(label, TaskRuntime::new(spec));
            }
        }

        self.source = definition.source;
        self.auto_start();
        Ok(())
    }

    pub fn apply(&mut self, task: Option<&str>, action: Action) -> Result<()> {
        if let Some(label) = task {
            return self
                .tasks
                .get_mut(label)
                .with_context(|| format!("task '{label}' not found in session '{}'", self.name))?
                .apply(action);
        }
        let mut failures = Vec::new();
        for (label, runtime) in &mut self.tasks {
            if let Err(error) = runtime.apply(action.clone()) {
                failures.push(format!("{label}: {error}"));
            }
        }
        if failures.is_empty() {
            Ok(())
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
        })
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
        task.stop().unwrap();
        assert_eq!(task.status, TaskStatus::Idle);
        assert!(task.pid.is_none());
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
}
