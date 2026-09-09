//! CLI subcommand runners.

use crate::daemon::{
    self, configured_settings, is_running, open_daemon_log, request, root_path, socket_path,
};
use crate::platform_service::{ServiceAction, service_control, service_status};
use crate::protocol::{Action, Request, Response};
use crate::state::{LeaderMode, NodeRole, NodeSettingsUpdate, StateStore};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use super::{AuthCommands, Cli, Commands, NodeCommands, ServiceCommands, WorkspaceCommands};

pub(crate) async fn run_workspace_command(command: WorkspaceCommands) -> Result<()> {
    match command {
        WorkspaceCommands::List => {
            print_response(daemon::request(&Request::ListWorkspaces).await?)?;
        }

        WorkspaceCommands::SetAlias { session, alias } => {
            print_response(
                daemon::request(&Request::SetWorkspaceAlias {
                    session,

                    alias: Some(alias),
                })
                .await?,
            )?;
        }

        WorkspaceCommands::ClearAlias { session } => {
            print_response(
                daemon::request(&Request::SetWorkspaceAlias {
                    session,

                    alias: None,
                })
                .await?,
            )?;
        }
    }

    Ok(())
}

pub(crate) fn print_service(
    status: anyhow::Result<crate::platform_service::ServiceStatus>,
) -> Result<()> {
    let status = status.map_err(|error| anyhow::anyhow!("{error:#}"))?;

    println!("{}", serde_json::to_string_pretty(&status)?);

    Ok(())
}

pub(crate) async fn run_service_command(command: ServiceCommands) -> Result<()> {
    match command {
        ServiceCommands::Status { scope } => {
            print_service(tokio::task::spawn_blocking(move || service_status(scope)).await?)?;
        }

        ServiceCommands::Install { scope, home } => {
            print_service(
                tokio::task::spawn_blocking(move || {
                    service_control(scope, ServiceAction::Install, home)
                })
                .await?,
            )?;
        }

        ServiceCommands::Uninstall { scope } => {
            print_service(
                tokio::task::spawn_blocking(move || {
                    service_control(scope, ServiceAction::Uninstall, None)
                })
                .await?,
            )?;
        }

        ServiceCommands::Start { scope } => {
            print_service(
                tokio::task::spawn_blocking(move || {
                    service_control(scope, ServiceAction::Start, None)
                })
                .await?,
            )?;
        }

        ServiceCommands::Stop { scope } => {
            print_service(
                tokio::task::spawn_blocking(move || {
                    service_control(scope, ServiceAction::Stop, None)
                })
                .await?,
            )?;
        }
    }

    Ok(())
}

pub(crate) async fn run_auth_command(command: AuthCommands) -> Result<()> {
    let root = daemon::root_path()?;

    let store = StateStore::open(&root)?;

    match command {
        AuthCommands::Status => {
            let settings = store.auth_settings()?;

            println!("{}", serde_json::to_string_pretty(&settings.public())?);
        }

        AuthCommands::Enable { generate } => {
            let generated = generate.then(|| uuid::Uuid::new_v4().simple().to_string());

            if let Some(key) = &generated {
                store.set_access_key(key)?;
            } else if !generate && std::env::var_os("TASKDECK_ACCESS_KEY").is_none() {
                bail!("provide TASKDECK_ACCESS_KEY or pass --generate");
            }

            let settings = store.configure_auth(true)?;

            println!("{}", serde_json::to_string_pretty(&settings.public())?);

            if let Some(key) = generated {
                println!("access key: {key}");
            }

            if daemon::is_running().await {
                let _ = daemon::request(&Request::Shutdown).await;
            }
        }

        AuthCommands::Disable => {
            let settings = store.configure_auth(false)?;

            println!("{}", serde_json::to_string_pretty(&settings.public())?);
        }

        AuthCommands::SetKey => {
            let mut line = String::new();

            std::io::Write::write_all(&mut std::io::stderr(), b"Enter new access key: ")?;

            std::io::stdin().read_line(&mut line)?;

            let key = line.trim();

            store.set_access_key(key)?;

            println!("access key updated");
        }
    }

    Ok(())
}

pub(crate) async fn run_node_command(command: NodeCommands) -> Result<()> {
    let root = daemon::root_path()?;

    let store = StateStore::open(&root)?;

    match command {
        NodeCommands::Show => {
            println!(
                "{}",
                serde_json::to_string_pretty(&store.node_settings()?.public())?
            );
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
                let _ = daemon::request(&Request::Shutdown).await;
            }

            println!("{}", serde_json::to_string_pretty(&settings.public())?);
        }
    }

    Ok(())
}

pub(crate) async fn control(
    requested_session: Option<String>,

    project: &std::path::Path,

    task: Option<String>,

    action: Action,
) -> Result<()> {
    let session = resolve_session(requested_session, Some(project)).await?;

    print_response(
        daemon::request(&Request::Action {
            session,

            task,

            action,
        })
        .await?,
    )
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

pub(crate) fn print_response(response: Response) -> Result<()> {
    if !response.ok {
        bail!(response.message);
    }

    if let Some(data) = response.data {
        println!("{}", serde_json::to_string_pretty(&data)?);
    } else {
        println!("{}", response.message);
    }

    Ok(())
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
