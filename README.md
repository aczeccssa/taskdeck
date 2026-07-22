# Taskdeck

Taskdeck runs project tasks in one per-user daemon and exposes the same live
processes through a TUI, CLI, Web UI, and MCP. A task started in one terminal
keeps running when that terminal or TUI exits and can be controlled elsewhere
by its global session name.

## What it reads

Taskdeck automatically imports `.vscode/tasks.json` from the project directory.
It supports `process` and `shell` tasks, `command`, `args`, `options.cwd`,
`options.env`, and these common variables:

- `${workspaceFolder}`
- `${workspaceFolderBasename}`
- `${env:NAME}`

VS Code JSON with comments and trailing commas is accepted. Add an optional
`taskdeck.yaml` beside the project README to name the session, add tasks, or
override imported tasks. See [taskdeck.example.yaml](taskdeck.example.yaml).

```yaml
version: 1
session: training-api
tasks:
  Run Backend API:
    auto_start: true
    stop_timeout_ms: 5000
  background-worker:
    command: cargo
    args: [run, --bin, worker]
    cwd: ./worker
    shell: false
```

## Install and run

```bash
cargo install --path .
cd /path/to/project
taskdeck
```

Running `taskdeck` opens the TUI and starts the singleton daemon when needed.
The project is registered using `taskdeck.yaml`'s `session`, or the directory
name by default. Use `--session NAME` to override it.

TUI keys:

| Key | Action |
| --- | --- |
| `Tab`, `Left`, `Right` | Switch the top task tab |
| `s` | Start selected task |
| `Space`, `p` | Pause or resume selected task |
| `r` | Restart selected task |
| `x` | Stop selected task |
| `Up`, `Down`, `PageUp`, `PageDown`, `End` | Browse/follow logs |
| `q`, `Esc`, `Ctrl+C` | Detach TUI; tasks keep running |

CLI examples:

```bash
taskdeck register --project /path/to/project --session api
taskdeck update --project /path/to/project --session api
taskdeck list
taskdeck status --session api
taskdeck start --session api --task "Run Backend API"
taskdeck pause --session api --task "Run Backend API"
taskdeck resume --session api --task "Run Backend API"
taskdeck restart --session api --task "Run Backend API"
taskdeck stop --session api --task "Run Backend API"
taskdeck stop --session api        # all tasks in the session
taskdeck remove --session api      # stop tasks and remove the session
```

After editing `.vscode/tasks.json` or `taskdeck.yaml`, run `taskdeck update` for
the registered project. Existing task processes and logs are retained; changed
settings take effect the next time a task starts or restarts. Removed tasks are
stopped, and newly added tasks honor `auto_start`. When one project has multiple
registered sessions, pass `--session` to select the one to update.

The per-user runtime directory is `~/.taskdeck`. Set `TASKDECK_HOME` to isolate
it, which is useful for tests or running a separate instance.

## Web UI and MCP

The daemon listens only on localhost:

- Web UI: `http://127.0.0.1:9837`
- Streamable HTTP MCP: `http://127.0.0.1:9837/mcp`

Example MCP client configuration:

```json
{
  "mcpServers": {
    "taskdeck": {
      "type": "http",
      "url": "http://127.0.0.1:9837/mcp"
    }
  }
}
```

The server exposes one tool, `taskdeck_control`. Its `action` is one of
`sessions`, `status`, `logs`, `start`, `stop`, `restart`, `pause`, or `resume`.
Pass `session` for every action except `sessions`; pass `task` to target one
task, or omit it to control all tasks in that session.

## Process behavior

- Each task gets its own Unix process group.
- Pause/resume sends `SIGSTOP`/`SIGCONT` to the full group.
- Stop sends `SIGTERM`, waits for `stop_timeout_ms`, then sends `SIGKILL`.
- Logs are retained in memory, up to 5,000 lines per task.
- The daemon and HTTP services bind per-user/localhost only. No authentication
  is provided, so do not proxy the Web UI or MCP port to an untrusted network.

Taskdeck currently targets macOS and Linux because process groups and Unix
domain sockets are central to its lifecycle guarantees.
