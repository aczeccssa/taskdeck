mod config;
mod daemon;
mod protocol;
mod runtime;
mod tui;
mod web;

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use crate::protocol::{Action, Request, Response};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[arg(
        long,
        global = true,
        default_value = ".",
        value_parser = parse_project_path
    )]
    project: PathBuf,
    #[arg(long, global = true)]
    session: Option<String>,
    #[command(subcommand)]
    command: Option<Commands>,
}

fn parse_project_path(value: &str) -> std::result::Result<PathBuf, String> {
    PathBuf::from(value)
        .canonicalize()
        .map_err(|error| format!("invalid project path '{value}': {error}"))
}

#[derive(Subcommand)]
enum Commands {
    /// Run the singleton daemon in the foreground.
    Daemon {
        #[arg(long, default_value_t = daemon::DEFAULT_WEB_PORT)]
        web_port: u16,
        #[arg(long, hide = true)]
        background: bool,
    },
    /// Open the terminal interface (default command).
    Tui,
    /// Register a project without opening the TUI.
    Register,
    /// Reload configuration for a registered project.
    Update,
    /// List global sessions.
    List,
    /// Print a session snapshot.
    Status {
        #[arg(long, default_value_t = 50)]
        tail: usize,
    },
    /// Start a task, or every task when --task is omitted.
    Start {
        #[arg(long)]
        task: Option<String>,
    },
    /// Pause a task, or every task when --task is omitted.
    Pause {
        #[arg(long)]
        task: Option<String>,
    },
    /// Resume a task, or every task when --task is omitted.
    Resume {
        #[arg(long)]
        task: Option<String>,
    },
    /// Restart a task, or every task when --task is omitted.
    Restart {
        #[arg(long)]
        task: Option<String>,
    },
    /// Stop a task, or every task when --task is omitted.
    Stop {
        #[arg(long)]
        task: Option<String>,
    },
    /// Stop all tasks and remove a session.
    Remove,
    /// Print Web UI and MCP endpoints.
    Endpoints,
    /// Stop the global daemon and every managed task.
    Shutdown,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if matches!(
        cli.command,
        Some(Commands::Daemon {
            background: true,
            ..
        })
    ) {
        let log = daemon::open_daemon_log()?;
        let stderr = log.try_clone()?;
        daemonize::Daemonize::new()
            .stdout(log)
            .stderr(stderr)
            .start()
            .context("failed to detach taskdeck daemon")?;
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(cli))
}

async fn run(cli: Cli) -> Result<()> {
    if let Some(Commands::Daemon { web_port, .. }) = cli.command {
        return daemon::run(web_port).await;
    }

    ensure_daemon().await?;
    match cli.command.unwrap_or(Commands::Tui) {
        Commands::Tui => tui::run(&cli.project, cli.session).await,
        Commands::Register => print_response(
            daemon::request(&Request::Register {
                project: cli.project,
                session: cli.session,
            })
            .await?,
        ),
        Commands::Update => print_response(
            daemon::request(&Request::Update {
                project: cli.project,
                session: cli.session,
            })
            .await?,
        ),
        Commands::List => print_response(daemon::request(&Request::ListSessions).await?),
        Commands::Status { tail } => {
            let session = resolve_session(cli.session, Some(&cli.project)).await?;
            print_response(
                daemon::request(&Request::Snapshot {
                    session,
                    tail: Some(tail),
                })
                .await?,
            )
        }
        Commands::Start { task } => control(cli.session, &cli.project, task, Action::Start).await,
        Commands::Pause { task } => control(cli.session, &cli.project, task, Action::Pause).await,
        Commands::Resume { task } => control(cli.session, &cli.project, task, Action::Resume).await,
        Commands::Restart { task } => {
            control(cli.session, &cli.project, task, Action::Restart).await
        }
        Commands::Stop { task } => control(cli.session, &cli.project, task, Action::Stop).await,
        Commands::Remove => {
            let session = resolve_session(cli.session, Some(&cli.project)).await?;
            print_response(daemon::request(&Request::RemoveSession { session }).await?)
        }
        Commands::Endpoints => {
            println!("Web UI: http://127.0.0.1:{}", daemon::DEFAULT_WEB_PORT);
            println!("MCP:    http://127.0.0.1:{}/mcp", daemon::DEFAULT_WEB_PORT);
            println!("IPC:    {}", daemon::socket_path()?.display());
            Ok(())
        }
        Commands::Shutdown => print_response(daemon::request(&Request::Shutdown).await?),
        Commands::Daemon { .. } => unreachable!(),
    }
}

async fn control(
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

async fn resolve_session(
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

fn print_response(response: Response) -> Result<()> {
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

async fn ensure_daemon() -> Result<()> {
    if daemon::is_running().await {
        return Ok(());
    }
    let executable = std::env::current_exe().context("cannot locate taskdeck executable")?;
    Command::new(executable)
        .arg("daemon")
        .arg("--background")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to launch taskdeck daemon")?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_argument_is_absolute_before_crossing_daemon_boundary() {
        let cli = Cli::try_parse_from(["taskdeck", "status", "--project", "."]).unwrap();

        assert!(cli.project.is_absolute());
        assert_eq!(
            cli.project,
            std::env::current_dir().unwrap().canonicalize().unwrap()
        );
    }
}
