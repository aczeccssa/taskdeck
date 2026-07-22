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

The task workspace provides stable log line numbers, 100/500/1,000/5,000-line
tails, case-insensitive search with previous/next navigation, focused live
follow, top/bottom controls, and a fullscreen log view. On desktop, worker
performance can be shown beside the log or as the full workspace; narrow
screens use separate Logs and Monitor views. The navigation sidebar can be
collapsed, and that preference is restored on the next visit.

### Editing registered configuration

Use the configuration button for a task to edit the registered project's task
definitions. The Web UI reads and writes the project-level `taskdeck.yaml`; a
registered session name remains read-only. Each response includes a content
`revision`, and Apply rejects the write if the file changed after it was read.
Writes use a temporary file followed by an atomic rename. YAML is normalized
when saved: unknown fields are preserved, but comments and formatting are not.

All registered sessions that point at the same project share this YAML and are
updated together after Apply. Existing running processes keep their current
command and environment until the next start or restart. Removed tasks stop
immediately, and newly added tasks with `auto_start: true` start immediately.
Deleting an imported VS Code task persists `enabled: false`; deleting a YAML
task removes its entry.

The corresponding endpoints are:

- `GET /api/sessions/{session}/config`
- `PUT /api/sessions/{session}/config` with `{ "revision": "...", "tasks": [...] }`

### Worker performance

The daemon samples every running task once per second and retains up to 600
samples (10 minutes). A task includes its root process and descendants. The UI
shows aggregate CPU, RSS memory, process count, and a process table containing
PID, PPID, name, CPU, RSS, runtime, and status. CPU uses `100%` for one fully
utilized logical CPU and can exceed `100%` for a multi-process tree. After a
task stops, its current metrics become unavailable while its history remains.

Read monitoring data without expanding the ordinary session snapshot:

```text
GET /api/sessions/{session}/tasks/{task}/metrics?window=600
```

`window` is clamped to 1-600 seconds. The response contains `current`,
`processes`, `samples`, `sample_interval_ms`, and the running state.

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

The Web UI reads MCP call history from `GET /api/mcp-calls`. Supported query
parameters:

- `q`: case-insensitive search across tool name, operation, session, task, and
  serialized request arguments
- `operation`: exact operation filter
- `status`: `all`, `success`, or `error`
- `session`: exact session filter
- `task`: exact task filter
- `page`: 1-based page number, default `1`
- `page_size`: default `20`; supported sizes are `20`, `50`, and `100`

`page_size` snaps to the nearest supported size for UI-friendly requests. Ties
snap down, so `35 -> 20` and `75 -> 50`.

`GET /api/mcp-calls` returns a standard envelope whose `data` object contains:

- `items`: newest-first call summaries with `id`, `tool`, `operation`,
  `started_at_ms`, `duration_ms`, `success`, and `input`
- `page`
- `page_size`
- `total`
- `total_pages`
- `has_next`
- `has_previous`

Requests with invalid `status`, `page`, or `page_size` return a validation
error in the normal response envelope. `GET /api/mcp-calls/{id}` remains the
unchanged detail endpoint.

## Process behavior

- Each task gets its own Unix process group.
- Pause/resume sends `SIGSTOP`/`SIGCONT` to the full group.
- Stop sends `SIGTERM`, waits for `stop_timeout_ms`, then sends `SIGKILL`.
- Logs are retained in memory, up to 5,000 lines per task.
- The daemon and HTTP services bind per-user/localhost only. No authentication
  is provided, so do not proxy the Web UI or MCP port to an untrusted network.

Taskdeck currently targets macOS and Linux because process groups and Unix
domain sockets are central to its lifecycle guarantees.
