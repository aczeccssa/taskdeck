//! OS service install/control.
//! OS service install/control (launchd/systemd/schtasks).

mod linux_backend;
mod macos_backend;
mod windows_backend;

use linux_backend::linux;
use macos_backend::macos;
use windows_backend::windows;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub(crate) use linux_backend::{linux_unit_path, render_systemd_unit};
#[allow(unused_imports)]
pub(crate) use macos_backend::{macos_domain, macos_plist_path, render_macos_plist};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
#[allow(unused_imports)]
pub(crate) use windows_backend::{render_windows_command, shell_quote};

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::protocol::ServiceScope;

const LABEL: &str = "io.taskdeck.daemon";
const LINUX_UNIT: &str = "taskdeck.service";
const WINDOWS_TASK: &str = "TaskdeckDaemon";
const WINDOWS_WRAPPER: &str = "taskdeck-service.cmd";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceAction {
    Status,
    Install,
    Uninstall,
    Start,
    Stop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceStatus {
    pub platform: &'static str,
    pub scope: ServiceScope,
    pub installed: bool,
    pub enabled: bool,
    pub running: bool,
    pub unit: String,
    pub executable: String,
    pub home: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
struct ServiceSpec {
    pub(crate) executable: PathBuf,
    pub(crate) home: PathBuf,
    pub(crate) environment_path: String,
}

#[cfg(test)]
impl ServiceSpec {
    pub(crate) fn new(executable: &str, home: &str) -> Self {
        Self {
            executable: PathBuf::from(executable),
            home: PathBuf::from(home),
            environment_path: "/usr/local/bin:/usr/bin:/bin".to_string(),
        }
    }
}

pub fn service_status(scope: ServiceScope) -> Result<ServiceStatus> {
    perform(scope, ServiceAction::Status, None)
}

pub fn service_control(
    scope: ServiceScope,
    action: ServiceAction,
    home: Option<PathBuf>,
) -> Result<ServiceStatus> {
    perform(scope, action, home)
}

pub(crate) fn perform(
    scope: ServiceScope,
    action: ServiceAction,
    requested_home: Option<PathBuf>,
) -> Result<ServiceStatus> {
    let executable = std::env::current_exe().context("cannot locate taskdeck executable")?;
    let environment_path = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".to_string());
    let home = match requested_home {
        Some(home) => fs::canonicalize(&home)
            .with_context(|| format!("service home '{}' does not exist", home.display()))?,
        None if action == ServiceAction::Install && scope == ServiceScope::System => {
            bail!("system-scope services require an explicit --home / \"home\" TASKDECK_HOME")
        }
        None => crate::daemon::root_path()?,
    };
    if action == ServiceAction::Start && daemon_running(&home) {
        bail!("Taskdeck daemon is already running for {}", home.display());
    }
    let spec = if action == ServiceAction::Status {
        let installed_home = status_home(
            scope,
            &ServiceSpec {
                executable: executable.clone(),
                home,
                environment_path: environment_path.clone(),
            },
        );
        ServiceSpec {
            executable,
            home: installed_home,
            environment_path,
        }
    } else {
        ServiceSpec { executable, home, environment_path }
    };
    let mut status = match std::env::consts::OS {
        "macos" => macos(scope, action, &spec)?,
        "linux" => linux(scope, action, &spec)?,
        "windows" => windows(scope, action, &spec)?,
        platform => bail!("automatic service management is not supported on {platform}"),
    };
    status.platform = match std::env::consts::OS {
        "macos" => "macos",
        "linux" => "linux",
        "windows" => "windows",
        other => other,
    };
    Ok(status)
}

pub(crate) fn daemon_running(home: &Path) -> bool {
    let lock_path = home.join("daemon.lock");
    let Ok(lock) = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
    else {
        return false;
    };
    lock.try_lock_exclusive().is_err()
}

pub(crate) fn command_output(command: &mut Command) -> Result<String> {
    let output = command
        .output()
        .with_context(|| format!("failed to run {}", command.get_program().to_string_lossy()))?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!(
            "{} failed ({}): {}",
            command.get_program().to_string_lossy(),
            output.status,
            if stderr.is_empty() { stdout } else { stderr }
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub(crate) fn status_home(scope: ServiceScope, spec: &ServiceSpec) -> PathBuf {
    let installed_home = match std::env::consts::OS {
        "macos" => macos_plist_path(scope)
            .ok()
            .and_then(|path| fs::read_to_string(path).ok()),
        "linux" => linux_unit_path(scope)
            .ok()
            .and_then(|path| fs::read_to_string(path).ok()),
        "windows" => fs::read_to_string(spec.home.join(WINDOWS_WRAPPER)).ok(),
        _ => None,
    };
    installed_home
        .and_then(|content| extract_environment_home(&content))
        .map(PathBuf::from)
        .unwrap_or_else(|| spec.home.clone())
}

pub(crate) fn extract_environment_home(content: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("Environment=\"TASKDECK_HOME=") {
            if let Some(trimmed) = value.strip_suffix('"') {
                return Some(trimmed.to_string());
            }
        }
        if let Some(value) = line.strip_prefix("Environment='TASKDECK_HOME=") {
            if let Some(trimmed) = value.strip_suffix('\'') {
                return Some(trimmed.to_string());
            }
        }
        if line.contains("TASKDECK_HOME") {
            if let Some(start) = line.find("<string>") {
                let rest = &line[start + "<string>".len()..];
                if let Some(end) = rest.find("</string>") {
                    return Some(rest[..end].to_string());
                }
            }
            if let Some(value) = line.strip_prefix(r#"set "TASKDECK_HOME="#) {
                return value.strip_suffix('"').map(str::to_string);
            }
        }
    }
    None
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension("tmp");
    fs::write(&temp, bytes).with_context(|| format!("failed to write {}", temp.display()))?;
    fs::rename(&temp, path).with_context(|| format!("failed to install {}", path.display()))?;
    Ok(())
}

pub(crate) fn run_quiet(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .is_ok_and(|output| output.status.success())
}
