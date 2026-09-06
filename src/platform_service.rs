use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

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
    executable: PathBuf,
    home: PathBuf,
}

#[cfg(test)]
impl ServiceSpec {
    fn new(executable: &str, home: &str) -> Self {
        Self {
            executable: PathBuf::from(executable),
            home: PathBuf::from(home),
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

fn perform(
    scope: ServiceScope,
    action: ServiceAction,
    requested_home: Option<PathBuf>,
) -> Result<ServiceStatus> {
    let executable = std::env::current_exe().context("cannot locate taskdeck executable")?;
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
            },
        );
        ServiceSpec {
            executable,
            home: installed_home,
        }
    } else {
        ServiceSpec { executable, home }
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

fn daemon_running(home: &Path) -> bool {
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

fn command_output(command: &mut Command) -> Result<String> {
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

fn macos(scope: ServiceScope, action: ServiceAction, spec: &ServiceSpec) -> Result<ServiceStatus> {
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

fn linux(scope: ServiceScope, action: ServiceAction, spec: &ServiceSpec) -> Result<ServiceStatus> {
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

fn windows(
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

fn macos_domain(scope: ServiceScope) -> Result<String> {
    if scope == ServiceScope::System {
        return Ok("system".to_string());
    }
    let uid = command_output(Command::new("id").arg("-u"))?;
    Ok(format!("gui/{uid}"))
}

fn macos_plist_path(scope: ServiceScope) -> Result<PathBuf> {
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

fn linux_unit_path(scope: ServiceScope) -> Result<PathBuf> {
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

fn status_home(scope: ServiceScope, spec: &ServiceSpec) -> PathBuf {
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

fn extract_environment_home(content: &str) -> Option<String> {
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

fn render_macos_plist(label: &str, spec: &ServiceSpec) -> String {
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

fn render_systemd_unit(spec: &ServiceSpec, scope: ServiceScope) -> String {
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

fn render_windows_command(spec: &ServiceSpec) -> String {
    format!(
        "@echo off\r\nset \"TASKDECK_HOME={}\"\r\n\"{}\" daemon\r\n",
        spec.home.display(),
        spec.executable.display(),
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension("tmp");
    fs::write(&temp, bytes).with_context(|| format!("failed to write {}", temp.display()))?;
    fs::rename(&temp, path).with_context(|| format!("failed to install {}", path.display()))?;
    Ok(())
}

fn run_quiet(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ServiceSpec {
        ServiceSpec::new("/usr/local/bin/taskdeck", "/tmp/taskdeck home")
    }

    #[test]
    fn renders_macos_plist_with_foreground_daemon_and_home() {
        let plist = render_macos_plist(LABEL, &spec());
        assert!(plist.contains("<string>/usr/local/bin/taskdeck</string>"));
        assert!(plist.contains("<string>daemon</string>"));
        assert!(plist.contains("TASKDECK_HOME"));
        assert!(plist.contains("/tmp/taskdeck home"));
        assert!(!plist.contains("--background"));
    }

    #[test]
    fn renders_systemd_user_and_system_units() {
        let user = render_systemd_unit(&spec(), ServiceScope::User);
        let system = render_systemd_unit(&spec(), ServiceScope::System);
        assert!(user.contains("WantedBy=default.target"));
        assert!(system.contains("WantedBy=multi-user.target"));
        assert!(user.contains("Environment=\"TASKDECK_HOME="));
        assert!(!user.contains("--background"));
    }

    #[test]
    fn extracts_home_from_all_service_file_formats() {
        let linux = "Environment='TASKDECK_HOME=/opt/taskdeck'\n";
        let macos = "<key>TASKDECK_HOME</key><string>/Users/dev/.taskdeck</string>\n";
        let windows = "set \"TASKDECK_HOME=C:\\Taskdeck\"\r\n";
        assert_eq!(
            extract_environment_home(linux).as_deref(),
            Some("/opt/taskdeck")
        );
        assert_eq!(
            extract_environment_home(macos).as_deref(),
            Some("/Users/dev/.taskdeck")
        );
        assert_eq!(
            extract_environment_home(windows).as_deref(),
            Some("C:\\Taskdeck")
        );
    }

    #[test]
    fn renders_windows_login_command_with_home() {
        let command = render_windows_command(&spec());
        assert!(command.contains("set \"TASKDECK_HOME=/tmp/taskdeck home\""));
        assert!(command.contains("\"/usr/local/bin/taskdeck\" daemon"));
    }
}
