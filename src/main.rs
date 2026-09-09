mod cluster;
mod config;
mod daemon;
mod platform_service;
mod protocol;
mod runtime;
mod service;
mod state;
mod tui;
mod web;

use std::path::PathBuf;
#[cfg(unix)]
use std::process::{Command, Stdio};
use std::time::Duration;

#[cfg(windows)]
use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use crate::platform_service::{ServiceAction, service_control, service_status};
use crate::protocol::{Action, Request, Response, ServiceScope};
use crate::state::{LeaderMode, NodeRole, NodeSettingsUpdate, StateStore};

#[derive(Parser)]
#[command(version, about)]
pub(crate) struct Cli {
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
pub(crate) enum Commands {
    /// Run the singleton daemon in the foreground.
    Daemon {
        #[arg(long)]
        web_port: Option<u16>,
        #[arg(long, hide = true)]
        background: bool,
    },
    /// Inspect or configure this Taskdeck installation.
    Node {
        #[command(subcommand)]
        command: NodeCommands,
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
    /// Inspect registered workspace aliases.
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommands,
    },
    /// Manage the native Taskdeck daemon service.
    Service {
        #[command(subcommand)]
        command: ServiceCommands,
    },
    /// Configure single-user access-key authentication.
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
    },
}

#[derive(Subcommand)]
pub(crate) enum AuthCommands {
    /// Print whether access-key authentication is enabled and configured.
    Status,
    /// Enable authentication. Uses TASKDECK_ACCESS_KEY or generates a one-time key.
    Enable {
        #[arg(long)]
        generate: bool,
    },
    /// Disable authentication and remove its configured key hash.
    Disable,
    /// Replace the configured key by reading a new value from stdin.
    SetKey,
}

#[derive(Subcommand)]
pub(crate) enum WorkspaceCommands {
    /// List registered workspaces and their aliases.
    List,
    /// Set the display alias for a stable session name.
    SetAlias {
        #[arg(long)]
        session: String,
        #[arg(long)]
        alias: String,
    },
    /// Clear the display alias for a stable session name.
    ClearAlias {
        #[arg(long)]
        session: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum ServiceCommands {
    /// Print user-scope service status.
    Status {
        #[arg(long, value_enum, default_value_t = ServiceScope::User)]
        scope: ServiceScope,
    },
    /// Install the native daemon service.
    Install {
        #[arg(long, value_enum, default_value_t = ServiceScope::User)]
        scope: ServiceScope,
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Remove the native daemon service.
    Uninstall {
        #[arg(long, value_enum, default_value_t = ServiceScope::User)]
        scope: ServiceScope,
    },
    /// Start an installed native daemon service.
    Start {
        #[arg(long, value_enum, default_value_t = ServiceScope::User)]
        scope: ServiceScope,
    },
    /// Stop an installed native daemon service.
    Stop {
        #[arg(long, value_enum, default_value_t = ServiceScope::User)]
        scope: ServiceScope,
    },
}

#[derive(Subcommand)]
pub(crate) enum NodeCommands {
    /// Print the persisted node role and connection settings.
    Show,
    /// Configure this installation as a worker or leader.
    Configure {
        #[arg(long)]
        role: Option<NodeRole>,
        #[arg(long)]
        leader_mode: Option<LeaderMode>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, conflicts_with = "clear_leader")]
        leader_url: Option<String>,
        #[arg(long)]
        clear_leader: bool,
        #[arg(long, conflicts_with = "clear_token")]
        token: Option<String>,
        #[arg(long)]
        clear_token: bool,
        #[arg(long)]
        bind_host: Option<String>,
        #[arg(long)]
        web_port: Option<u16>,
    },
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
        #[cfg(unix)]
        {
            let log = daemon::open_daemon_log()?;
            let stderr = log.try_clone()?;
            daemonize::Daemonize::new()
                .stdout(log)
                .stderr(stderr)
                .start()
                .context("failed to detach taskdeck daemon")?;
        }
        #[cfg(windows)]
        attach_daemon_log()?;
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(cli))
}

async fn run(cli: Cli) -> Result<()> {
    if let Some(Commands::Daemon { web_port, .. }) = &cli.command {
        return daemon::run(*web_port).await;
    }
    if let Some(Commands::Auth { command }) = cli.command {
        return run_auth_command(command).await;
    }
    if let Some(Commands::Node { command }) = cli.command {
        return run_node_command(command).await;
    }
    if let Some(Commands::Service { command }) = cli.command {
        return run_service_command(command).await;
    }

    ensure_daemon().await?;
    if let Some(Commands::Workspace { command }) = cli.command {
        return run_workspace_command(command).await;
    }
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
            let settings = daemon::configured_settings()?;
            let display_host = if settings.bind_host == "0.0.0.0" {
                "127.0.0.1"
            } else {
                settings.bind_host.as_str()
            };
            println!("Web UI: http://{}:{}", display_host, settings.web_port);
            println!("MCP:    http://{}:{}/mcp", display_host, settings.web_port);
            println!("IPC:    {}", daemon::socket_path()?.display());
            Ok(())
        }
        Commands::Shutdown => print_response(daemon::request(&Request::Shutdown).await?),
        Commands::Daemon { .. }
        | Commands::Node { .. }
        | Commands::Auth { .. }
        | Commands::Workspace { .. }
        | Commands::Service { .. } => unreachable!(),
    }
}

mod cli;
#[allow(unused_imports)]
use cli::*;

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

    #[test]
    fn parses_pure_master_configuration() {
        let cli = Cli::try_parse_from([
            "taskdeck",
            "node",
            "configure",
            "--role",
            "leader",
            "--leader-mode",
            "pure-master",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Some(Commands::Node {
                command: NodeCommands::Configure {
                    role: Some(NodeRole::Leader),
                    leader_mode: Some(LeaderMode::PureMaster),
                    ..
                }
            })
        ));
    }
}
