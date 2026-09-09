//! launchd (macOS) service backend.

use super::LABEL;
use super::{daemon_running, extract_environment_home, status_home, write_atomic, run_quiet, ServiceAction, ServiceSpec, ServiceStatus};

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use super::command_output;
use super::windows_backend::{shell_quote, xml_escape};
use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::protocol::ServiceScope;
pub(crate) fn macos(scope: ServiceScope, action: ServiceAction, spec: &ServiceSpec) -> Result<ServiceStatus> {
    let unit = LABEL.to_string();
    let plist_path = macos_plist_path(scope)?;
    let installed = plist_path.exists();
    let domain = macos_domain(scope)?;
    let error: Option<String> = None;
    match action {
        ServiceAction::Status => {}
        ServiceAction::Install => {
            if let Some(parent) = plist_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            let plist = render_macos_plist(&unit, spec);
            write_atomic(&plist_path, plist.as_bytes())?;
            if !run_quiet(
                "launchctl",
                &["bootstrap", &domain, &plist_path.to_string_lossy()],
            ) {
                run_quiet("launchctl", &["load", "-w", &plist_path.to_string_lossy()]);
            }
        }
        ServiceAction::Uninstall => {
            if installed {
                if !run_quiet(
                    "launchctl",
                    &[
                        "bootout",
                        &format!("{domain}/{unit}"),
                        &plist_path.to_string_lossy(),
                    ],
                ) {
                    run_quiet("launchctl", &["unload", &plist_path.to_string_lossy()]);
                }
                let _ = fs::remove_file(&plist_path);
            }
        }
        ServiceAction::Start => {
            if !installed {
                bail!("service is not installed");
            }
            if !run_quiet(
                "launchctl",
                &["kickstart", "-k", &format!("{domain}/{unit}")],
            ) {
                run_quiet(
                    "launchctl",
                    &["bootstrap", &domain, &plist_path.to_string_lossy()],
                );
            }
        }
        ServiceAction::Stop => {
            if !run_quiet("launchctl", &["bootout", &format!("{domain}/{unit}")]) {
                run_quiet("launchctl", &["unload", &plist_path.to_string_lossy()]);
            }
        }
    }
    let installed = action == ServiceAction::Uninstall || plist_path.exists();
    Ok(ServiceStatus {
        platform: "macos",
        scope,
        installed,
        enabled: installed,
        running: daemon_running(&status_home(scope, spec)),
        unit,
        executable: spec.executable.display().to_string(),
        home: status_home(scope, spec).display().to_string(),
        error,
    })
}

pub(crate) fn macos_domain(scope: ServiceScope) -> Result<String> {
    if scope == ServiceScope::System {
        return Ok("system".to_string());
    }
    let uid = command_output(Command::new("id").arg("-u"))?;
    Ok(format!("gui/{uid}"))
}

pub(crate) fn macos_plist_path(scope: ServiceScope) -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")?;
    Ok(match scope {
        ServiceScope::User => home
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist")),
        ServiceScope::System => {
            PathBuf::from("/Library/LaunchDaemons").join(format!("{LABEL}.plist"))
        }
    })
}

pub(crate) fn render_macos_plist(label: &str, spec: &ServiceSpec) -> String {
    let home = spec.home.display().to_string();
    let executable = spec.executable.display().to_string();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{executable}</string>
    <string>daemon</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>TASKDECK_HOME</key><string>{home}</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>{home}/daemon.log</string>
  <key>StandardErrorPath</key><string>{home}/daemon.log</string>
</dict>
</plist>
"#,
        label = xml_escape(label),
        executable = xml_escape(&executable),
        home = xml_escape(&home),
    )
}

