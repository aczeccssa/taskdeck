//! systemd (Linux) service backend.

use super::LINUX_UNIT;
use super::{
    ServiceAction, ServiceSpec, ServiceStatus, daemon_running, extract_environment_home, run_quiet,
    status_home, write_atomic,
};

use super::command_output;
use super::windows_backend::shell_quote;
use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::protocol::ServiceScope;
pub(crate) fn linux(
    scope: ServiceScope,
    action: ServiceAction,
    spec: &ServiceSpec,
) -> Result<ServiceStatus> {
    let unit_path = linux_unit_path(scope)?;
    let unit = LINUX_UNIT.to_string();
    let mode = match scope {
        ServiceScope::User => "--user",
        ServiceScope::System => "--system",
    };
    let error: Option<String> = None;
    match action {
        ServiceAction::Status => {}
        ServiceAction::Install => {
            if let Some(parent) = unit_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            write_atomic(&unit_path, render_systemd_unit(spec, scope).as_bytes())?;
            command_output(Command::new("systemctl").args([mode, "daemon-reload"]))
                .context("systemd daemon reload failed")?;
            command_output(Command::new("systemctl").args([mode, "enable", &unit]))
                .context("systemd enable failed")?;
        }
        ServiceAction::Uninstall => {
            if unit_path.exists() {
                let _ = Command::new("systemctl")
                    .args([mode, "disable", &unit])
                    .output();
                let _ = fs::remove_file(&unit_path);
                let _ = Command::new("systemctl")
                    .args([mode, "daemon-reload"])
                    .output();
            }
        }
        ServiceAction::Start => {
            command_output(Command::new("systemctl").args([mode, "start", &unit]))?;
        }
        ServiceAction::Stop => {
            command_output(Command::new("systemctl").args([mode, "stop", &unit]))?;
        }
    }
    let installed = action != ServiceAction::Uninstall && unit_path.exists();
    let enabled = installed
        && Command::new("systemctl")
            .args([mode, "is-enabled", &unit])
            .output()
            .is_ok_and(|output| output.status.success());
    let running = installed
        && Command::new("systemctl")
            .args([mode, "is-active", &unit])
            .output()
            .is_ok_and(|output| output.status.success());
    Ok(ServiceStatus {
        platform: "linux",
        scope,
        installed,
        enabled,
        running,
        unit,
        executable: spec.executable.display().to_string(),
        home: status_home(scope, spec).display().to_string(),
        error,
    })
}

pub(crate) fn linux_unit_path(scope: ServiceScope) -> Result<PathBuf> {
    Ok(match scope {
        ServiceScope::User => {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .context("HOME is not set")?;
            home.join(".config/systemd/user").join(LINUX_UNIT)
        }
        ServiceScope::System => PathBuf::from("/etc/systemd/system").join(LINUX_UNIT),
    })
}

pub(crate) fn render_systemd_unit(spec: &ServiceSpec, scope: ServiceScope) -> String {
    let wanted_by = match scope {
        ServiceScope::User => "default.target",
        ServiceScope::System => "multi-user.target",
    };
    format!(
        r#"[Unit]
Description=Taskdeck daemon
After=network.target

[Service]
Type=simple
ExecStart={executable} daemon
Environment="TASKDECK_HOME={home}"
Restart=on-failure
RestartSec=3

[Install]
WantedBy={wanted_by}
"#,
        executable = shell_quote(&spec.executable.display().to_string()),
        home = shell_quote(&spec.home.display().to_string()),
    )
}
