//! CLI subcommand runners.

use crate::config;
use crate::daemon::{self, request};
use crate::platform_service::{ServiceAction, service_control, service_status};
use crate::protocol::{Action, Request};
use crate::state::{NodeSettingsUpdate, StateStore};
use crate::{update, version};
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

#[cfg(windows)]
use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};

use super::{AuthCommands, NodeCommands, ServiceCommands, WorkspaceCommands};

mod output;
pub(crate) use output::{print_message, print_response, print_service, print_table, print_value};

pub(crate) async fn run_upgrade_command(check_only: bool, install_requested: bool, background: bool, json: bool) -> Result<()> {
    if background {
        let _ = tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let release = tokio::task::spawn_blocking(update::latest)
        .await
        .context("release check task failed")??;
    let status = update::status_from_release(release.clone(), update::timestamp_ms());
    if install_requested {
        if !status.available {
            return print_config_value(&status, "UPGRADE", json);
        }
        let _ = daemon::request(&Request::Shutdown).await;
        let message = update::install_latest_release(release).await?;
        if json { println!("{}", serde_json::json!({"ok":true,"message":message,"version":version::VERSION})); }
        else { print_message(&message); }
        return Ok(());
    }
    let _ = check_only;
    print_config_value(&status, "UPGRADE", json)
}

pub(crate) async fn run_list_command(json: bool) -> Result<()> {
    if json {
        return print_response(request(&Request::ListSessions).await?, true);
    }

    let response = request(&Request::ListSessions).await?;
    if !response.ok {
        return print_response(response, false);
    }
    let names = response
        .data
        .as_ref()
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let registrations = StateStore::open(&daemon::root_path()?)?.registrations()?;
    let rows = names
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(|session| {
            let status = registrations
                .iter()
                .find(|registration| registration.session == session)
                .map(|registration| {
                    if config::discover_inner(&registration.project, Some(session), true).is_ok() {
                        "available"
                    } else {
                        "unavailable"
                    }
                })
                .unwrap_or("unknown");
            vec![session.to_string(), status.to_string()]
        })
        .collect::<Vec<_>>();
    print_table("SESSIONS", &["SESSION", "STATUS/AVAILABILITY"], &rows);
    Ok(())
}

pub(crate) async fn run_workspace_command(command: WorkspaceCommands, json: bool) -> Result<()> {
    let response = match command {
        WorkspaceCommands::List => request(&Request::ListWorkspaces).await?,
        WorkspaceCommands::SetAlias { session, alias } => {
            request(&Request::SetWorkspaceAlias {
                session,
                alias: Some(alias),
            })
            .await?
        }
        WorkspaceCommands::ClearAlias { session } => {
            request(&Request::SetWorkspaceAlias {
                session,
                alias: None,
            })
            .await?
        }
    };
    print_response(response, json)
}

pub(crate) async fn run_service_command(command: ServiceCommands, json: bool) -> Result<()> {
    match command {
        ServiceCommands::Status { scope } => {
            print_service(
                tokio::task::spawn_blocking(move || service_status(scope)).await?,
                json,
            )?;
        }
        ServiceCommands::Install { scope, home } => {
            print_service(
                tokio::task::spawn_blocking(move || {
                    service_control(scope, ServiceAction::Install, home)
                })
                .await?,
                json,
            )?;
        }
        ServiceCommands::Uninstall { scope } => {
            print_service(
                tokio::task::spawn_blocking(move || {
                    service_control(scope, ServiceAction::Uninstall, None)
                })
                .await?,
                json,
            )?;
        }
        ServiceCommands::Start { scope } => {
            print_service(
                tokio::task::spawn_blocking(move || {
                    service_control(scope, ServiceAction::Start, None)
                })
                .await?,
                json,
            )?;
        }
        ServiceCommands::Stop { scope } => {
            print_service(
                tokio::task::spawn_blocking(move || {
                    service_control(scope, ServiceAction::Stop, None)
                })
                .await?,
                json,
            )?;
        }
    }
    Ok(())
}

pub(crate) async fn run_auth_command(command: AuthCommands, json: bool) -> Result<()> {
    let store = StateStore::open(&daemon::root_path()?)?;
    match command {
        AuthCommands::Status => {
            let settings = store.auth_settings()?;
            print_config_value(&settings.public(), "AUTH", json)?;
        }
        AuthCommands::Enable { generate } => {
            let generated = generate.then(|| uuid::Uuid::new_v4().simple().to_string());
            if let Some(key) = &generated {
                store.set_access_key(key)?;
            } else if std::env::var_os("TASKDECK_ACCESS_KEY").is_none() {
                bail!("provide TASKDECK_ACCESS_KEY or pass --generate");
            }
            let settings = store.configure_auth(true)?;
            print_config_value(&settings.public(), "AUTH", json)?;
            if let Some(key) = generated {
                if json {
                    println!("access key: {key}");
                } else {
                    print_table("ACCESS KEY", &["KEY"], &[vec![key]]);
                }
            }
            if daemon::is_running().await {
                let _ = request(&Request::Shutdown).await;
            }
        }
        AuthCommands::Disable => {
            let settings = store.configure_auth(false)?;
            print_config_value(&settings.public(), "AUTH", json)?;
        }
        AuthCommands::SetKey => {
            let mut line = String::new();
            std::io::Write::write_all(&mut std::io::stderr(), b"Enter new access key: ")?;
            std::io::stdin().read_line(&mut line)?;
            store.set_access_key(line.trim())?;
            if json {
                println!("access key updated");
            } else {
                print_message("access key updated");
            }
        }
    }
    Ok(())
}

fn print_config_value<T: serde::Serialize>(value: &T, title: &str, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        print_value(&serde_json::to_value(value)?, title);
    }
    Ok(())
}

pub(crate) async fn run_node_command(command: NodeCommands, json: bool) -> Result<()> {
    let store = StateStore::open(&daemon::root_path()?)?;
    match command {
        NodeCommands::Show => {
            let settings = store.node_settings()?.public();
            print_config_value(&settings, "NODE", json)?;
        }
        NodeCommands::Configure {
            role,
            leader_mode,
            name,
            leader_url,
            clear_leader,
            token,
            clear_token,
            bind_host,
            web_port,
        } => {
            let settings = store.configure(NodeSettingsUpdate {
                role,
                leader_mode,
                name,
                leader_url: if clear_leader {
                    Some(None)
                } else {
                    leader_url.map(Some)
                },
                enrollment_token: if clear_token {
                    Some(None)
                } else {
                    token.map(Some)
                },
                bind_host,
                web_port,
            })?;
            if daemon::is_running().await {
                let _ = request(&Request::Shutdown).await;
            }
            print_config_value(&settings.public(), "NODE", json)?;
        }
    }
    Ok(())
}

pub(crate) async fn control(
    requested_session: Option<String>,

    project: &std::path::Path,

    task: Option<String>,

    action: Action,
    json: bool,
) -> Result<()> {
    let session = resolve_session(requested_session, Some(project)).await?;

    print_response(
        daemon::request(&Request::Action {
            session,

            task,

            action,
        })
        .await?,
        json,
    )
}

pub(crate) async fn run_init_command(
    project: &Path,
    requested_session: Option<&str>,
    json: bool,
) -> Result<()> {
    let initialized = config::init_project(project, requested_session)?;
    let response = match request(&Request::Register {
        project: initialized.project.clone(),
        session: Some(initialized.session.clone()),
        allow_empty: true,
    })
    .await
    {
        Ok(response) => response,
        Err(error) => {
            let _ = fs::remove_file(&initialized.config_path);
            return Err(error);
        }
    };
    if !response.ok {
        let _ = fs::remove_file(&initialized.config_path);
    }
    if !json {
        print_table(
            "INIT",
            &["SESSION", "CONFIG"],
            &[vec![
                initialized.session,
                initialized.config_path.display().to_string(),
            ]],
        );
    }
    print_response(response, json)
}

pub(crate) async fn resolve_session(
    requested: Option<String>,

    project: Option<&std::path::Path>,
) -> Result<String> {
    if let Some(session) = requested {
        return Ok(session);
    }

    if let Some(project) = project {
        let response = daemon::request(&Request::Register {
            project: project.to_path_buf(),

            session: None,
            allow_empty: false,
        })
        .await?;

        if !response.ok {
            bail!(response.message);
        }

        if let Some(name) = response
            .data
            .as_ref()
            .and_then(|data| data.get("name"))
            .and_then(|name| name.as_str())
        {
            return Ok(name.to_string());
        }
    }

    bail!("--session is required")
}

pub(crate) async fn ensure_daemon() -> Result<()> {
    if daemon::is_running().await {
        return Ok(());
    }

    let executable = std::env::current_exe().context("cannot locate taskdeck executable")?;

    spawn_background_daemon(&executable)?;

    for _ in 0..50 {
        if daemon::is_running().await {
            return Ok(());
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    bail!(
        "daemon did not become ready; inspect {}/daemon.log",
        daemon::root_path()?.display()
    )
}

#[cfg(unix)]

pub(crate) fn spawn_background_daemon(executable: &std::path::Path) -> Result<()> {
    Command::new(executable)
        .arg("daemon")
        .arg("--background")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to launch taskdeck daemon")?;

    Ok(())
}

#[cfg(windows)]

pub(crate) fn spawn_background_daemon(executable: &std::path::Path) -> Result<()> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            CREATE_BREAKAWAY_FROM_JOB, CREATE_NO_WINDOW, CreateProcessW, PROCESS_INFORMATION,
            STARTUPINFOW,
        },
    };

    let application = executable
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();

    let mut command_line = format!("\"{}\" daemon --background", executable.display())
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();

    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,

        ..Default::default()
    };

    let mut process = PROCESS_INFORMATION::default();

    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut process,
        )
    };

    if created == 0 {
        return Err(std::io::Error::last_os_error()).context("failed to launch taskdeck daemon");
    }

    unsafe {
        CloseHandle(process.hThread);

        CloseHandle(process.hProcess);
    }

    Ok(())
}

#[cfg(windows)]

pub(crate) fn attach_daemon_log() -> Result<()> {
    use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle};

    let stdout = daemon::open_daemon_log()?;

    let stderr = stdout.try_clone()?;

    let stdout_set = unsafe { SetStdHandle(STD_OUTPUT_HANDLE, stdout.as_raw_handle().cast()) } != 0;

    let stderr_set = unsafe { SetStdHandle(STD_ERROR_HANDLE, stderr.as_raw_handle().cast()) } != 0;

    if !stdout_set || !stderr_set {
        return Err(std::io::Error::last_os_error()).context("failed to attach daemon log");
    }

    std::mem::forget(stdout);

    std::mem::forget(stderr);

    Ok(())
}
