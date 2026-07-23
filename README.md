# Taskdeck

Taskdeck runs project tasks in a persistent per-user daemon. The same binary can
run as a standalone worker, a standard leader that also runs local tasks, or a
pure master that only controls remote workers. CLI, TUI, Web UI, and MCP all
operate on the daemon's live state.

## Install and standalone use

```bash
./scripts/install-local.sh
cd /path/to/project
taskdeck
```

The default installation is an unlinked `worker`, preserving the normal local
workflow. Project registrations and node settings are stored in
`~/.taskdeck/state.db`; reinstalling or restarting Taskdeck does not remove
them. On daemon startup, Taskdeck reloads every registered project and restores
configured `auto_start` tasks. Missing or invalid project directories remain
visible as unavailable registrations instead of being silently forgotten.

Set `TASKDECK_HOME` to use an isolated state directory.

Taskdeck imports `.vscode/tasks.json` and applies optional `taskdeck.yaml`
overrides. It supports process and shell tasks, `command`, `args`, `options.cwd`,
`options.env`, JSON comments/trailing commas, and common workspace/environment
variables. See [taskdeck.example.yaml](taskdeck.example.yaml).

Use the optional top-level `task_order` list to share the Web/TUI tab order.
Each task may set `clear_logs_on_restart: true` to clear its retained logs and
performance history whenever it restarts; the default preserves history and
marks restarts in the performance charts.

```bash
taskdeck register --project /path/to/project --session api
taskdeck update --project /path/to/project --session api
taskdeck list
taskdeck status --session api
taskdeck start --session api --task "Run Backend API"
taskdeck pause --session api --task "Run Backend API"
taskdeck resume --session api --task "Run Backend API"
taskdeck restart --session api --task "Run Backend API"
taskdeck stop --session api
taskdeck remove --session api
```

Running `taskdeck` without a subcommand opens the TUI. Detaching the TUI does
not stop managed tasks.

## Nodes and roles

Every installation has one stable node ID and exactly one role. There are no
separate worker and leader builds.

| Configuration | Runs local tasks | MCP scope | Upstream connection |
| --- | --- | --- | --- |
| `worker` | Yes | Local node only | Optional single leader |
| `leader/standard` | Yes, as `self` | Local and all workers | None |
| `leader/pure_master` | No | All workers | None |

Inspect and configure the local installation:

```bash
taskdeck node show

# Link this installation to a leader. The worker initiates the connection.
taskdeck node configure --role worker --name laptop \
  --leader-url http://leader.example:9837 --token "$TASKDECK_TOKEN"

# A leader that also runs this machine's tasks.
taskdeck node configure --role leader --leader-mode standard \
  --name workstation --token "$TASKDECK_TOKEN"

# A control-plane-only leader.
taskdeck node configure --role leader --leader-mode pure-master \
  --name master --bind-host 0.0.0.0 --token "$TASKDECK_TOKEN"
```

Changing node configuration stops the active daemon; the next command starts
it with the new settings. A standard leader cannot switch to pure master while
local projects remain registered. Leaders cannot connect to another leader.

Workers connect outbound over WebSocket, so no worker ingress port is required.
If the leader is unavailable, local worker tasks, CLI, TUI, Web UI, and MCP keep
working. The leader retains the last worker inventory while offline and refuses
new remote actions rather than queueing them.

The enrollment token protects worker admission. Use TLS directly or at a
trusted reverse proxy before exposing a leader outside a private development
network. Node APIs never return the token.

## Pure master with Compose

The provided Compose deployment intentionally supports only
`leader/pure_master`. It uses the same general Taskdeck binary, configured at
runtime, and persists leader identity plus known worker inventory in a named
volume.

```bash
export TASKDECK_ENROLLMENT_TOKEN='replace-with-a-long-random-token'
./scripts/deploy-compose.sh
```

Open `http://127.0.0.1:9837`. Override the host port with `TASKDECK_PORT` and the
display name with `TASKDECK_NODE_NAME`.

There is deliberately no official worker or standard-leader Compose service.
Those roles execute project commands and must live in the project's actual
development image/toolchain. Install the Taskdeck binary into that environment,
persist its `TASKDECK_HOME`, and configure it as a worker pointing at the pure
master. The deployment does not mount the Docker socket.

## Deployment scripts

The deployment helpers resolve the repository from their own location, so they
work from either the project root or the `scripts` directory:

```bash
# Install this checkout on the current macOS or Linux machine.
./scripts/install-local.sh

# Rebuild and recreate the default Compose service.
./scripts/deploy-compose.sh

# Deploy through another Docker engine/context.
./scripts/deploy-compose.sh --context production

# Detect a remote macOS/Linux target, build or reuse its binary, then install it.
./scripts/deploy-ssh.sh user@example.com
```

From inside `scripts`, use `./install-local.sh`, `./deploy-compose.sh`, or
`./deploy-ssh.sh` directly. Run any command with `--help` for its complete
options and environment variables. Local installation rejects Windows; SSH
deployment also reports Windows targets as unsupported.

All three commands accept `--sync`. Sync is deliberately opt-in: it requires a
clean worktree and a configured upstream, then runs a fetch followed by a
fast-forward-only merge before building or deploying. It never creates merge
commits, rebases, stashes, or discards local changes.

## Service discovery

Taskdeck classifies each task as a likely service, ordinary process, or unknown.
It combines:

- command and argument signals;
- nearby manifests such as `package.json`, `Cargo.toml`, `go.mod`, and project
  files;
- configured host/port arguments and environment variables;
- TCP listeners owned by the task process tree, discovered with `lsof`;
- recognizable startup log URLs as fallback evidence.

The Web UI shows the likely runtime/framework and endpoint chips in the task
header. Listening, configured, and reported endpoints remain distinct. A bind
such as `0.0.0.0:3000` is reported as observed state, not claimed to be an
externally reachable URL. Without `lsof`, Taskdeck degrades to static/log
evidence instead of pretending a configured port is live.

## Web UI and APIs

By default the daemon binds to `0.0.0.0:9837`, making its Web, MCP, and worker
agent endpoints available on every network interface. Use `127.0.0.1` when
accessing it from the same machine:

- Web UI: `http://127.0.0.1:9837`
- Streamable HTTP MCP: `http://127.0.0.1:9837/mcp`
- Worker agent connection: `ws://leader:9837/api/agent/connect`
- Health: `GET /healthz`

The leader UI selects node, then session, then task. Standard leaders expose
their executor as node `self`; pure masters list only connected or previously
known workers. Remote configuration editing remains revision checked and writes
the owning worker's `taskdeck.yaml`.

Existing session routes accept a `node` query parameter on leaders. Important
cluster endpoints are:

```text
GET  /api/nodes
GET  /api/sessions?node=NODE_ID
GET  /api/sessions/{session}?node=NODE_ID
GET  /api/sessions/{session}/tasks/{task}/logs?node=NODE_ID
GET  /api/sessions/{session}/tasks/{task}/metrics?node=NODE_ID
GET  /api/sessions/{session}/config?node=NODE_ID
PUT  /api/sessions/{session}/config?node=NODE_ID
POST /api/action              { node, session, task, action }
```

## MCP scope

Configure an MCP client with:

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

Worker MCP is deliberately local-only: its schema has no `node` field and it
rejects attempts to provide one. It supports `sessions`, `status`, `logs`,
`start`, `stop`, `restart`, `pause`, and `resume`.

Leader MCP adds `nodes` and `services`, returns node-qualified aggregate session
rows, and requires an explicit `node` for targeted reads or actions. Use
`node: "self"` for a standard leader's local tasks. It never guesses a target
from a globally unique-looking session name, because another worker may later
register the same name.

MCP calls are retained in memory and visible in the Web UI or through
`GET /api/mcp-calls`; call details preserve the complete request and response.

## Process behavior

- Each task runs in its own Unix process group.
- Pause/resume sends `SIGSTOP`/`SIGCONT` to the full group.
- Stop sends `SIGTERM`, waits for `stop_timeout_ms`, then sends `SIGKILL`.
- Logs retain up to 5,000 lines per task in memory.
- CPU, RSS, descendants, and process state are sampled once per second, with a
  ten-minute history.
- Graceful daemon shutdown stops managed tasks. Registrations are durable, but
  live log pipes and arbitrary orphaned processes are not reconstructed after a
  crash.

Taskdeck currently targets macOS and Linux because Unix process groups and
per-user Unix sockets are central to its lifecycle guarantees.
