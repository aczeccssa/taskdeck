//! schtasks (Windows) service backend.

use super::{
    ServiceAction, ServiceSpec, ServiceStatus, daemon_running, extract_environment_home, run_quiet,
    status_home, write_atomic,
};
use super::{WINDOWS_TASK, WINDOWS_WRAPPER};

use super::command_output;
use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::protocol::ServiceScope;
pub(crate) fn windows(
    scope: ServiceScope,
    action: ServiceAction,
    spec: &ServiceSpec,
) -> Result<ServiceStatus> {
    let wrapper = spec.home.join(WINDOWS_WRAPPER);
    let error: Option<String> = None;
    match action {
        ServiceAction::Status => {}
        ServiceAction::Install => {
            fs::create_dir_all(&spec.home)
                .with_context(|| format!("failed to create {}", spec.home.display()))?;
            write_atomic(&wrapper, render_windows_command(spec).as_bytes())?;
            let mut args = vec![
                "/Create".to_string(),
                "/F".to_string(),
                "/TN".to_string(),
                WINDOWS_TASK.to_string(),
                "/TR".to_string(),
                format!("\"{}\"", wrapper.display()),
            ];
            match scope {
                ServiceScope::User => {
                    args.extend(["/SC".to_string(), "ONLOGON".to_string()]);
                }
                ServiceScope::System => {
                    args.extend([
                        "/SC".to_string(),
                        "ONSTART".to_string(),
                        "/RU".to_string(),
                        "SYSTEM".to_string(),
                        "/RL".to_string(),
                        "HIGHEST".to_string(),
                    ]);
                }
            }
            command_output(Command::new("schtasks").args(&args))
                .context("Windows Task Scheduler registration failed")?;
        }
        ServiceAction::Uninstall => {
            let _ = Command::new("schtasks")
                .args(["/Delete", "/F", "/TN", WINDOWS_TASK])
                .output();
            let _ = fs::remove_file(&wrapper);
        }
        ServiceAction::Start => {
            command_output(Command::new("schtasks").args(["/Run", "/TN", WINDOWS_TASK]))?;
        }
        ServiceAction::Stop => {
            command_output(Command::new("schtasks").args(["/End", "/TN", WINDOWS_TASK])).or_else(
                |_| command_output(Command::new(std::env::current_exe()?).arg("shutdown")),
            )?;
        }
    }
    let installed = action != ServiceAction::Uninstall && wrapper.exists();
    let enabled = installed
        && Command::new("schtasks")
            .args(["/Query", "/TN", WINDOWS_TASK])
            .output()
            .is_ok_and(|output| output.status.success());
    Ok(ServiceStatus {
        platform: "windows",
        scope,
        installed,
        enabled,
        running: daemon_running(&spec.home),
        unit: WINDOWS_TASK.to_string(),
        executable: spec.executable.display().to_string(),
        home: spec.home.display().to_string(),
        error,
    })
}

pub(crate) fn render_windows_command(spec: &ServiceSpec) -> String {
    format!(
        "@echo off\r\nset \"TASKDECK_HOME={}\"\r\n\"{}\" daemon\r\n",
        spec.home.display(),
        spec.executable.display(),
    )
}

pub(crate) fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
