import {useEffect, useRef, useState} from "react";
import {createRoot} from "react-dom/client";
import "./styles.css";

const DOC_HASHES = new Set(["quickstart", "install", "configure", "cli", "webui", "cluster", "api", "mcp", "operations", "upgrade", "troubleshooting", "releases"]);
const syncIntroVisibility = (): void => {
  const key = location.hash.slice(1);
  const home = !key || key === "home" || !DOC_HASHES.has(key);
  const landing = document.getElementById("landing");
  const root = document.getElementById("root");
  if (landing) landing.hidden = !home;
  if (root) root.hidden = home;
};
addEventListener("hashchange", syncIntroVisibility);
document.addEventListener("click", (event) => {
  const target = event.target instanceof Element ? event.target.closest(".brand") : null;
  if (target) { event.preventDefault(); history.pushState({}, "", "#home"); syncIntroVisibility(); }
});
syncIntroVisibility();

const externalizeLinks=():void=>{
  document.querySelectorAll<HTMLAnchorElement>('a[href^="http"]').forEach(link=>{link.target="_blank";link.rel="noopener noreferrer"});
  const footer=document.querySelector<HTMLElement>(".landing-footer");
  if(footer) footer.innerHTML=`<div class="footer-brand"><strong>Taskdeck</strong><span>Durable task control for every project.</span></div><div class="footer-links"><div><b>Explore</b><a href="#capabilities">Capabilities</a><a href="#architecture">Architecture</a></div><div><b>Resources</b><a href="#quickstart">Documentation</a><a href="${repo}/releases" target="_blank" rel="noopener noreferrer">Releases ↗</a></div><div><b>Project</b><a href="${repo}" target="_blank" rel="noopener noreferrer">GitHub ↗</a><a href="${repo}/blob/main/LICENSE" target="_blank" rel="noopener noreferrer">MIT License ↗</a></div></div><div class="footer-meta"><span>v${version}</span><span>© ${new Date().getFullYear()} Taskdeck</span></div>`;
};

type Language = "en" | "zh";
type Theme = "system" | "light" | "dark";
type PageId = "quickstart"|"install"|"configure"|"cli"|"webui"|"cluster"|"api"|"mcp"|"operations"|"upgrade"|"troubleshooting"|"releases";
type Block = {title?: string; text?: string; code?: string; list?: string[]; note?: string};
type Page = {id:PageId; label:string; group:string; intro:string; blocks:Block[]};
const version=(import.meta.env.VITE_TASKDECK_VERSION??"0.2.0").replace(/^v/,"");
const repo="https://github.com/aczeccssa/taskdeck";
const release=`${repo}/releases/tag/v${version}`;
const download=`${repo}/releases/download/v${version}`;
externalizeLinks();

const enPages:Page[]=[
{id:"quickstart",label:"Quick start",group:"start",intro:"Install one binary, register a workspace, and start a task.",blocks:[
{title:"Install and open the project",text:"Use the installer for your platform, then run Taskdeck from the project directory. The first run creates ~/.taskdeck/state.db.",code:`# macOS / Linux
./scripts/install-local.sh
cd /path/to/project
taskdeck

# Windows PowerShell
.\\scripts\\install-local.ps1
Set-Location C:\\path\\to\\project
taskdeck.exe`},
{title:"Register a workspace",text:"init imports .vscode/tasks.json when present and creates taskdeck.yaml. register only records an existing configuration.",code:`taskdeck init --project ./my-app --session api
taskdeck register --project ./my-app --session api
taskdeck list`},
{title:"Run and inspect a task",code:`taskdeck start --session api --task "Run Backend API"
taskdeck status --session api
taskdeck logs --session api --task "Run Backend API"
taskdeck stop --session api`},
{note:"Running taskdeck without a subcommand opens the TUI. Detaching the TUI does not stop managed tasks."}]},
{id:"install",label:"Installation",group:"start",intro:"Choose a release binary, build from source, or run a pure master with Compose.",blocks:[
{title:"Release binaries",text:"Every stable tag publishes Linux x86_64/arm64, macOS x86_64/arm64 and Windows x86_64/arm64 archives with LICENSE, README.md, taskdeck.example.yaml and SHA256SUMS.",code:`# Linux x86_64 example
curl -LO ${download}/taskdeck-v${version}-x86_64-unknown-linux-gnu.tar.gz
curl -LO ${download}/SHA256SUMS
sha256sum -c SHA256SUMS --ignore-missing
tar -xzf taskdeck-v${version}-x86_64-unknown-linux-gnu.tar.gz
sudo install taskdeck /usr/local/bin/taskdeck`},
{title:"Build from source",text:"The Rust build embeds the React frontend. Install Bun 1.3.14 or newer before compiling.",code:`git clone ${repo}.git
cd taskdeck
bun install --cwd frontend --frozen-lockfile
cargo build --release
./target/release/taskdeck --help`},
{title:"Docker / Compose",text:"The supplied Compose deployment is intentionally a leader/pure_master control plane. Workers run in their real project toolchain.",code:`export TASKDECK_ENROLLMENT_TOKEN='replace-with-a-long-random-token'
./scripts/deploy-compose.sh
# or
TASKDECK_PORT=9837 docker compose up -d`},
{title:"Platform requirements",list:["Linux: glibc target x86_64 or arm64; systemd is optional.","macOS: Intel or Apple Silicon; launchd user service is available.","Windows: x86_64 or arm64; Task Scheduler integration uses PowerShell.","Native installs bind to 127.0.0.1 by default. Compose intentionally binds to 0.0.0.0 and opts in with TASKDECK_ALLOW_REMOTE_BIND=true."]}]},
{id:"configure",label:"Configuration",group:"start",intro:"Configure tasks, environment, node identity and authentication without losing state.",blocks:[
{title:"taskdeck.yaml",text:"The optional project file overlays imported VS Code tasks. Workspace values are inherited by every task; task env wins over workspace_env.",code:`version: 1
session: api
workspace_env:
  APP_ENV: development
task_order: ["Run Backend API", "Run frontend"]
tasks:
  "Run Backend API":
    type: process
    command: cargo
    args: [run]
    cwd: .
    env:
      RUST_LOG: debug
    auto_start: false
    schedule: "*/10 * * * *"`},
{title:"Node roles",text:"There is one binary and one stable node ID per installation. A worker can connect outbound to one leader; leaders never connect to another leader.",list:["worker: runs local tasks; MCP is scoped to the local node; optional leader connection.","leader / standard: runs local tasks as self and controls workers.","leader / pure_master: control plane only; controls workers and runs no local project tasks."]},
{title:"Authentication",code:`export TASKDECK_AUTH_ENABLED=true
export TASKDECK_ACCESS_KEY='a-long-random-access-key'
printf '%s' 'new-key' | taskdeck auth set-key
taskdeck auth status`},
{title:"Environment variables",list:["TASKDECK_HOME isolates state.db and the update directory.","TASKDECK_AUTH_ENABLED and TASKDECK_ACCESS_KEY control access-key authentication.","TASKDECK_UPDATE_CHECK=0 disables background release checks.","TASKDECK_PORT, TASKDECK_NODE_NAME and TASKDECK_ENROLLMENT_TOKEN are used by Compose."]}]},
{id:"cli",label:"CLI reference",group:"work",intro:"Use the CLI for repeatable operations and scripts. Add --json for machine-readable output.",blocks:[
{title:"Workspace lifecycle",code:`taskdeck init --project PATH --session NAME
taskdeck register --project PATH --session NAME
taskdeck update --project PATH --session NAME
taskdeck list
taskdeck workspace list
taskdeck workspace set-alias --session NAME --alias "Backend API"
taskdeck workspace clear-alias --session NAME
taskdeck remove --session NAME
taskdeck unregister --session NAME`},
{title:"Task lifecycle",code:`taskdeck status --session NAME
taskdeck start --session NAME --task TASK
taskdeck pause --session NAME --task TASK
taskdeck resume --session NAME --task TASK
taskdeck restart --session NAME --task TASK
taskdeck stop --session NAME`},
{title:"Node and service management",code:`taskdeck node show
taskdeck node integrity-check
taskdeck node configure --role worker --name laptop \
  --leader-url http://leader:9837 --token "$TASKDECK_TOKEN"
taskdeck service status
taskdeck service install --scope user
taskdeck service start
taskdeck service stop
taskdeck service uninstall`},
{title:"JSON output",code:`taskdeck --json list
taskdeck status --session api --json`}]},
{id:"webui",label:"Web UI, TUI and workflows",group:"work",intro:"Use the same daemon state from a browser, terminal or automation client.",blocks:[
{title:"Web UI",text:"Open http://127.0.0.1:9837. On a leader, select a node, workspace and task. Settings can edit node configuration, service state, API tokens, quotas, alerts, scaling policies and board templates."},
{title:"TUI",text:"Run taskdeck with no subcommand. The terminal interface uses the daemon and remains useful over SSH. Closing the TUI leaves running processes untouched."},
{title:"Workflow groups",text:"Create a group, arrange cards on the canvas, connect directed edges and run it in topological order. Cycles are rejected. By default a run stops at the first failure; set stop_on_failure to false to continue."},
{title:"Boards, alerts and history",text:"Boards pin node/session/task cards. Task runs and events are stored in SQLite; logs stay in memory. Alert rules subscribe to task_started, task_exited, task_failed and task_stopped and can deliver a JSON webhook."}]},
{id:"cluster",label:"Nodes and deployment",group:"work",intro:"Scale from one local worker to a pure-master control plane with remote workers.",blocks:[
{title:"Configure a worker",code:`taskdeck node configure --role worker --name laptop \
  --leader-url http://leader.example:9837 \
  --token "$TASKDECK_TOKEN"`},
{title:"Configure a standard leader",code:`taskdeck node configure --role leader --leader-mode standard \
  --name workstation --token "$TASKDECK_TOKEN"`},
{title:"Configure a pure master",code:`taskdeck node configure --role leader --leader-mode pure-master \
  --name master --bind-host 0.0.0.0 --allow-remote-bind \
  --token "$TASKDECK_TOKEN"`},
{title:"SSH and reverse proxy",text:"Workers initiate WebSocket connections, so worker ingress is not required. Put TLS directly on Taskdeck or at a trusted reverse proxy before exposing a leader outside a private network. scripts/deploy-ssh.sh probes PowerShell before Unix commands and installs to the platform's user directory."}]},
{id:"api",label:"REST API",group:"operate",intro:"Automate Taskdeck with the authenticated HTTP API on port 9837.",blocks:[
{title:"Authentication and version",code:`curl -H "Authorization: Bearer tdk_..." http://127.0.0.1:9837/api/sessions
curl http://127.0.0.1:9837/api/version
curl http://127.0.0.1:9837/healthz`},
{title:"Core routes",code:`GET  /api/nodes
GET  /api/workspaces?node=NODE_ID
GET  /api/sessions?node=NODE_ID
GET  /api/sessions/{session}
GET  /api/sessions/{session}/tasks/{task}/logs
GET  /api/sessions/{session}/tasks/{task}/metrics
POST /api/action { node, session, task, action }
GET  /api/task-runs?node=&session=&task=
GET  /api/events`},
{title:"Boards, tokens and updates",code:`GET/POST /api/boards
GET/PUT/DELETE /api/boards/{board}
GET/POST /api/quotas
GET/POST /api/notification-rules
GET /api/notifications
POST /api/tokens
GET /api/update
POST /api/update/check
POST /api/update/install`},
{note:"Leader routes accept node=NODE_ID. API tokens are shown once and stored as hashes; revoke them when an integration is retired."}]},
{id:"mcp",label:"MCP",group:"operate",intro:"Connect an MCP client to the embedded Streamable HTTP server.",blocks:[
{title:"Endpoint and configuration",code:`http://127.0.0.1:9837/mcp

{
  "mcpServers": {
    "taskdeck": {
      "type": "http",
      "url": "http://127.0.0.1:9837/mcp",
      "headers": {"Authorization": "Bearer tdk_..."}
    }
  }
}`},
{title:"Scopes",text:"Workers expose local tools. Standard leaders expose self and connected workers. Pure masters expose connected and previously known workers. MCP calls are persisted in SQLite."},
{title:"Safe automation",list:["Create a dedicated API token for each client.","Use node/session/task identifiers from inspect responses rather than display aliases.","Keep enrollment tokens separate from API tokens; node APIs never return enrollment secrets."]}]},
{id:"operations",label:"Operations",group:"operate",intro:"Keep services healthy with quotas, scaling, alerts and durable history.",blocks:[
{title:"Quotas and dependencies",text:"Quotas cap concurrent running tasks per workspace or node. Dependencies are start gates across workspaces or nodes; cycles are rejected and every dependency must be running before a dependent task starts."},
{title:"Autoscaling",text:"Scaling policies watch task metrics and apply scale-out/scale-in thresholds with cooldowns. Configure them from Settings or /api/scaling-policies."},
{title:"Native service manager",code:`taskdeck service install --scope user
taskdeck service start

sudo taskdeck service install --scope system --home /var/lib/taskdeck
sudo taskdeck service start --scope system`},
{title:"State and logs",text:"Registrations, task runs, events, workflow revisions and integration records live in TASKDECK_HOME/state.db. Process logs are retained in memory and can be read through the Web UI or logs API."},
{title:"Audit storage and search",text:'Successful Web Snapshot, TaskLogs and TaskMetrics polling requests are not audited. Replicated audit history keeps the newest 10,000 records. While a worker is offline, the oldest 10,000 pending records are kept for replication and the newest 10,000 for local history; pending records in between are discarded once the backlog exceeds 20,000. Search indexes at most the first 4 KiB of each request, response and details JSON field; open a record to inspect its full stored detail. SQLite reuses pages freed by retention, so pruning does not shrink an existing state.db. To reclaim space, stop the daemon, back up TASKDECK_HOME, then run `sqlite3 "${TASKDECK_HOME:-$HOME/.taskdeck}/state.db" "VACUUM; PRAGMA wal_checkpoint(TRUNCATE);"` before restarting. Enrollment tokens are stored as plaintext in the local state.db; protect the state directory and clear unused tokens. Node APIs redact enrollment tokens.'}]},
{id:"upgrade",label:"Upgrade and rollback",group:"reference",intro:"Updates are release-only, checksum-verified and always user-confirmed.",blocks:[
{title:"Check or install",code:`taskdeck upgrade --check
taskdeck upgrade --install

# Disable background checks
TASKDECK_UPDATE_CHECK=0`},
{title:"What happens during an upgrade",list:["Taskdeck checks the GitHub latest stable release at most once every 24 hours.","The matching OS/architecture asset and SHA256SUMS are downloaded to TASKDECK_HOME/updates.","The checksum is verified before a helper stops the daemon and atomically replaces the executable.","A .bak is retained until the new process starts successfully; failures restore the previous binary."]},
{title:"Manual fallback",text:"If no matching asset exists, the binary is not writable, or a service needs elevation, the API returns a clear error and the release page remains available for manual installation. Historical versions use the same updater protocol."}]},
{id:"troubleshooting",label:"Troubleshooting",group:"reference",intro:"A short path through the failures most often seen during installation and development.",blocks:[
{title:"Program not found",text:"Taskdeck starts commands with the daemon's environment. Confirm the executable is on PATH for the service user, use an absolute command path, or set the task's options.env/PATH. For frontend tasks use npm.cmd on Windows when launched from a non-interactive process.",code:`where.exe npm
npm.cmd --version
taskdeck status --session api --json`},
{title:"Web UI cannot connect",text:"Check that the daemon listens on 127.0.0.1:9837, that auth settings match the browser, and that the selected node is online. Health is unauthenticated at /healthz."},
{title:"Worker is offline",text:"Verify the leader URL, enrollment token and reverse proxy WebSocket support. Workers keep local tasks running while disconnected; leaders refuse new remote actions until reconnect."},
{title:"Build failures",code:`bun --version
bun install --cwd frontend --frozen-lockfile
npm run build --prefix frontend
cargo check`}]},
{id:"releases",label:"Releases",group:"reference",intro:"Every stable release has matching binaries, checksums and versioned documentation.",blocks:[
{title:`v${version}`,text:"Current documentation is built from the repository tag and published alongside the GitHub Release. The release page contains the changelog and SHA256SUMS.",code:`${release}
${download}/SHA256SUMS`},
{title:"Known limits",text:"Scheduled occurrences missed while Taskdeck is offline are not replayed. The production history gap and the reported task-service restart issue were not independently reproduced during this review; see CHANGELOG.md for scope and follow-up notes."},
{title:"Versioned docs",text:"The Pages build keeps latest at / and freezes each stable tag under /versions/<tag>/. The version index records release date, release URL, checksum URL and documentation path."},
{title:"Release checklist",list:["Cargo.toml version and v<version> tag must match.","Six platform archives must be present.","SHA256SUMS must cover every archive.","Release notes link to the matching documentation version."]}]}
];
const detailedIntros:Partial<Record<PageId,string>>={
  quickstart:"This guide takes you from an empty project to a running, observable task. You will install the daemon, register a workspace, learn how Taskdeck stores state, and use the same identifiers from the CLI, Web UI, and automation clients.",
  install:"Taskdeck is distributed as one native binary, so installation is mostly about choosing the right platform artifact and deciding where the daemon should keep its state. The sections below cover verified release archives, source builds, Compose deployments, and the runtime assumptions that affect service managers.",
  configure:"Configuration has two layers: the project file describes workspaces and tasks, while node settings describe where the daemon runs and how it authenticates. This page explains precedence, role selection, environment isolation, and the choices that should be made before exposing a leader to other machines.",
  cli:"The CLI is designed for scripts as well as interactive use. Commands are grouped by lifecycle, return useful errors, and can emit stable JSON so a deployment script can inspect state without scraping terminal output.",
  webui:"The Web UI and TUI are views over the same daemon state; they do not create a second execution model. This guide explains what each interface is best at and how workflows, boards, alerts, metrics, and history fit together when a task moves from development to operations.",
  cluster:"A Taskdeck installation can stay local or become a small control plane without changing the task definition. Learn how workers connect outbound, how standard leaders differ from pure masters, and how to place TLS and service supervision around the connection.",
  api:"The HTTP API exposes the same concepts as the CLI: nodes, workspaces, sessions, tasks, runs, events, and integrations. Start with health and version checks, then add a bearer token and use the resource routes to build idempotent automation.",
  mcp:"The embedded MCP endpoint gives an MCP client node-aware tools backed by the daemon. This page explains endpoint configuration, role-dependent visibility, token separation, and the guardrails that keep an assistant from acting on the wrong workspace.",
  operations:"Operations features make long-running task systems predictable: quotas prevent noisy neighbors, dependencies define start gates, scaling policies respond to metrics, and durable history explains what happened after a restart. Use this page as the runbook for a managed installation.",
  upgrade:"Upgrades are deliberately conservative. Taskdeck checks a stable release, verifies the matching archive, keeps a rollback copy, and only changes the running binary after the user or an explicit automation step confirms the operation.",
  troubleshooting:"When a task system fails, separate daemon health, process launch, network reachability, and configuration errors. The checks on this page are ordered from the cheapest local observation to the deeper service and build diagnostics.",
  releases:"A release is more than a binary upload: the version, checksums, documentation snapshot, service behavior, and rollback path must agree. Use this page to consume a release safely or to verify the repository before publishing one."
};
const detailedDescriptions:Record<string,string>={
  "quickstart:Install and open the project":"The installer only places the executable; it does not need a project-specific runtime. On first launch Taskdeck creates its state database, discovers the current working directory as the project context, and starts the daemon that owns subsequent CLI, TUI, and Web UI requests. Keep the project directory stable when you register tasks so relative paths resolve the same way from a service or a remote action.",
  "quickstart:Register a workspace":"`init` is the onboarding command: it reads an existing VS Code task file when one is present, writes a reviewable `taskdeck.yaml`, and records a named session. Use `register` when the YAML already exists and you only want to add its workspace to the local state. The final `list` is an intentional verification step; it confirms the session name that later commands must use.",
  "quickstart:Run and inspect a task":"Starting a task creates a managed process record rather than a fire-and-forget child. `status` tells you whether the task is ready, running, paused, or stopped; `logs` reads the retained process output; and `stop` asks the daemon to terminate the process cleanly. In scripts, check status after start and treat the task identifier as data instead of relying on a display label.",
  "install:Release binaries":"Release archives are the safest installation path for a machine that does not need a Rust toolchain. Download the archive and checksum file from the same tag, verify before extracting, and install the executable only after the checksum succeeds. The archive also carries the example configuration and license so an installation can be reproduced without checking out the source repository.",
  "install:Build from source":"A source build is useful when testing an unreleased change or packaging Taskdeck for an internal platform. The frontend is compiled into the Rust binary, so the frontend lockfile must be installed before `cargo build`; a successful build can then be copied to a clean host without shipping `node_modules` or a JavaScript runtime.",
  "install:Docker / Compose":"Compose runs a pure-master control plane and keeps project execution on workers that have the real toolchains. Treat the enrollment token as a bootstrap secret: pass it through the environment or a secret store, enroll workers, then rotate or remove it. Bind the port to a private interface until authentication and TLS are configured.",
  "install:Platform requirements":"The supported archive name encodes both operating system and CPU architecture, which is why a build for macOS arm64 cannot be substituted for Linux arm64. The daemon defaults to port 9837 and binds to `127.0.0.1`; remote access requires the explicit `--allow-remote-bind` option. The Compose example opts in through `TASKDECK_ALLOW_REMOTE_BIND=true`, so protect its port with authentication, TLS, and a private network policy.",
  "configure:taskdeck.yaml":"The project file is intentionally declarative: it can be reviewed, committed, and applied again without losing the daemon database. Workspace environment values form the base, task-level `env` overrides them, and command arguments remain separate from the shell so quoting behaves consistently across platforms. Use `task_order` when the UI needs a stable presentation order; it does not replace dependency gates.",
  "configure:Node roles":"Role selection determines both execution authority and what remote clients can see. A worker owns local processes and may connect outbound; a standard leader also runs its own tasks; a pure master stores coordination state and delegates execution. Choose one stable node name per installation and avoid reusing an identity for two machines, because node IDs are used in API and event records.",
  "configure:Authentication":"Access-key authentication protects HTTP, Web UI, and MCP requests; it is independent from the enrollment token used when a worker first joins a leader. Store the key outside the repository, enable authentication before binding a leader to a shared network, and use `auth status` to verify whether the daemon is enforcing it. Keys are persisted as hashes, so a lost key must be replaced rather than read back.",
  "configure:Environment variables":"Environment variables are process-level defaults, not a substitute for documenting project configuration. `TASKDECK_HOME` is especially important for services because it controls where the state database and update files live. When a service manager starts the daemon with a reduced environment, set the required variables in the service scope instead of assuming your interactive shell will be inherited.",
  "cli:Workspace lifecycle":"Workspace commands separate discovery from execution. Initialize once when importing a project, register an existing file when onboarding a preconfigured workspace, and use `update` after task definitions change. Removal deletes the registration while unregistering the session leaves the project files untouched; confirm the target session with `list` before using either destructive lifecycle command.",
  "cli:Task lifecycle":"Task lifecycle commands are safe to repeat when their requested state is already true. `pause` preserves the process record for a resumable task, `restart` performs a stop/start cycle, and `stop` applies to the session when no task is supplied. Add `--json` to status calls in CI so failures can report the daemon state and task exit information directly.",
  "cli:Node and service management":"Node commands answer the two questions that are often confused during incidents: which identity is this binary using, and is the daemon supervised? Configure a worker before asking a leader to act on it, then install the native service only after a foreground `taskdeck` run works. User-scope services avoid elevated paths; system-scope services require explicit state and home directories.",
  "cli:JSON output":"JSON output is intended for automation and diagnostics, not for human presentation. Keep the command and schema version in your integration tests, handle missing remote nodes as a state transition rather than a parsing error, and record the returned node/session identifiers so later actions address the same resources.",
  "webui:Web UI":"The Web UI is a control surface over the selected node and workspace. Start by checking the node connection indicator, then inspect a session before sending an action; settings change durable configuration such as tokens, quotas, alerts, and scaling policies. Browser state does not replace daemon state, so a second browser or CLI sees the same running process and event history.",
  "webui:TUI":"The TUI is useful over SSH and in environments where opening a browser is inconvenient. It subscribes to the daemon rather than owning the child processes, so closing the terminal leaves running tasks alone. Use the CLI or Web UI when you need to edit configuration, inspect historical events, or operate on a remote node.",
  "webui:Workflow groups":"A workflow group is a directed graph of task cards. Edges are evaluated as dependencies, cycles are rejected before a run starts, and the runner follows topological order so a downstream task never starts before its prerequisites. Keep `stop_on_failure` enabled for deployment paths; disable it only when independent branches should continue and later inspection can tolerate partial results.",
  "webui:Boards, alerts and history":"Boards provide an operational view without duplicating task state. Alerts subscribe to lifecycle events and can feed both the notification inbox and a JSON webhook, while task runs and workflow revisions remain queryable in SQLite. Logs are retained in memory, so export important output from the logs API before a process restart if it must be kept as an artifact.",
  "cluster:Configure a worker":"A worker opens the WebSocket connection to its leader, which means the worker usually needs outbound access only. Give it a unique name and a token with the intended enrollment scope, then verify the worker appears online before dispatching tasks. Local tasks continue while the connection is down, but remote actions wait until the leader has a current worker view.",
  "cluster:Configure a standard leader":"A standard leader is both a worker for its own project and a coordinator for connected workers. This is a good default for a workstation or a small deployment: local tasks remain available if the network is unavailable, while the same node can route actions to remote workers. Use a separate pure master when control and execution need to be isolated.",
  "cluster:Configure a pure master":"A pure master owns coordination but deliberately does not run project tasks. It is useful in Compose or a central operations environment where workers are deployed near their source code and toolchains. Because it is a control plane, protect its bind address, authentication key, enrollment flow, and backup of `state.db` as one operational unit.",
  "cluster:SSH and reverse proxy":"Workers initiate connections, so reverse-proxy rules must support WebSocket upgrades on the leader endpoint. Terminate TLS at Taskdeck or a trusted proxy, forward the original host and protocol headers, and set idle timeouts longer than the expected task heartbeat. Test reconnect behavior, not only the first handshake, before treating a cluster as production-ready.",
  "api:Authentication and version":"Use `/healthz` for an unauthenticated liveness check and `/api/version` to confirm that a client is speaking to the expected server. All state-changing or private routes should use a bearer token and should fail closed when authentication is enabled. Keep the token in a secret manager and never include it in a browser URL, log line, or checked-in curl script.",
  "api:Core routes":"The API follows a resource-first shape: discover nodes, select a workspace or session, inspect task state, then send an action. Leader requests should include `node` explicitly so an operation cannot silently fall back to the leader's own task. Read logs and metrics after an action to build a useful result, and use events or task runs for durable audit information rather than polling forever.",
  "api:Boards, tokens and updates":"Integration endpoints are separated by responsibility. Board and quota routes change operational policy, token routes create credentials that are shown once, and update routes only stage or install a verified release. Give each integration its own token and revoke it independently; this makes audit records meaningful and limits the effect of a compromised client.",
  "mcp:Endpoint and configuration":"Configure MCP with the Streamable HTTP URL and a bearer token. The endpoint is intentionally the same daemon surface used by the API, so a client can discover node and session identifiers before requesting an action. Keep MCP clients on a private network or behind the same TLS and authentication boundary as the Web UI.",
  "mcp:Scopes":"Tool visibility follows the node role. Workers expose local tools, standard leaders add themselves to the connected worker set, and pure masters expose connected or previously known workers without gaining a local project directory. This distinction is part of the safety model: an assistant should select a node explicitly before it invokes a task action.",
  "mcp:Safe automation":"Treat an MCP client like any other production integration. Give it a dedicated token, log the node/session/task tuple before an action, use inspect responses instead of display aliases, and separate enrollment secrets from API credentials. For destructive actions, require the surrounding workflow to confirm the current state immediately before invocation.",
  "operations:Quotas and dependencies":"Quotas protect a workspace or node from unbounded concurrency, while dependencies express a start gate between tasks. A dependency is satisfied only when the prerequisite is actually running; a cycle is rejected because no valid start order exists. Set quotas from observed capacity, then make the dependency graph visible in a workflow or configuration review so a blocked task is explainable.",
  "operations:Autoscaling":"Scaling policies turn task metrics into controlled changes rather than reacting to every sample. Define scale-out and scale-in thresholds with a cooldown long enough for a new worker or process to become useful, and observe the policy through events before increasing its limits. A policy should have a clear lower bound, upper bound, and failure behavior when metrics are unavailable.",
  "operations:Native service manager":"Install a user service for a developer workstation and a system service for a dedicated host. Verify the service account can read the project, state directory, tokens, and toolchain before diagnosing the daemon itself. After installation, check status and logs from the service manager, then perform one controlled restart to confirm the state database and worker connections recover.",
  "operations:State and logs":"SQLite stores registrations, task runs, events, workflow revisions, and integration records so a restart does not erase the operational story. Process logs are intentionally short-lived memory data; use the logs API to inspect them while the task is alive and ship important output to your own artifact store. Back up `state.db` with the daemon stopped or with a filesystem snapshot that preserves SQLite consistency.",
  "upgrade:Check or install":"`upgrade --check` only reports an available stable release; it does not change the binary. The install command downloads the platform asset and checksum into the update directory, verifies the digest, and asks the helper to perform an atomic replacement. Keep a maintenance window for service-managed hosts and make sure the service account can write the update directory before starting an unattended procedure.",
  "upgrade:What happens during an upgrade":"The updater limits release checks, chooses the asset that matches the current OS and architecture, and verifies it before stopping the daemon. It retains a `.bak` until the new process starts successfully, so a failed launch can restore the previous executable. State remains in `TASKDECK_HOME`; the upgrade changes the binary and frontend bundle, not your project registrations or task history.",
  "upgrade:Manual fallback":"A manual install is appropriate when the host cannot write its service directory, the release has no matching architecture, or policy requires an administrator to approve the replacement. Preserve the existing binary and state directory, verify the downloaded checksum yourself, and start the daemon in the foreground once so permission and migration errors are visible before handing control back to the service manager.",
  "troubleshooting:Program not found":"The daemon launches commands with the service user's environment, which is often smaller than your interactive shell. Compare `where.exe` or `which` output inside the service context, then use an absolute path or a task-level PATH override. On Windows, `npm.cmd` is the executable a non-interactive process can usually launch; a successful terminal command alone does not prove the service can resolve it.",
  "troubleshooting:Web UI cannot connect":"Start at the boundary: confirm the daemon is listening on the expected bind address, request `/healthz`, and inspect the browser's selected node and authentication state. A healthy process with a wrong bind address looks identical to a stopped daemon from another machine. If a reverse proxy is involved, verify both authorization headers and WebSocket upgrade forwarding.",
  "troubleshooting:Worker is offline":"Check the leader URL and enrollment token on the worker, then inspect proxy and firewall logs for a WebSocket handshake or reconnect attempt. A worker can continue local work while offline, so distinguish an unavailable control connection from a failed task process. Once the connection returns, inspect the node record before retrying queued remote actions.",
  "troubleshooting:Build failures":"Reproduce the failure with the smallest toolchain check first: Bun and the frontend lockfile, then the frontend build, then Cargo. Keep the Rust and frontend versions pinned in CI and avoid using a global `npm` install to hide a missing project dependency. The first compiler or bundler error is usually more useful than the final aggregate failure message.",
  [`releases:v${version}`]:"Taskdeck 0.2.0 focuses on operational reliability and safer defaults: audit history now has bounded retention and searchable excerpts, current-schema database opens avoid migration-lock contention, more scheduled-run outcomes are persisted, and native remote binding requires explicit authorization. The release also documents SQLite compaction and the known follow-up limits. Download an archive and SHA256SUMS from this same tag.",
  "releases:Versioned docs":"Latest documentation is convenient for new users, but reproducible deployments need a versioned snapshot. The Pages build keeps the current docs at the root and stores stable tags below `/versions/<tag>/`, allowing an incident report or runbook to point at the exact commands that shipped with a binary.",
  "releases:Release checklist":"Run the checklist before creating a GitHub Release: compare the version in Cargo, the tag, and the generated archive names; verify every supported architecture has an asset; validate all checksums; and make the release notes link to the matching docs snapshot. A release is ready only when a fresh machine can follow the installation page without relying on unpublished files."
};
for(const page of enPages){const intro=detailedIntros[page.id];if(intro)page.intro=intro;for(const block of page.blocks){const detail=detailedDescriptions[`${page.id}:${block.title??""}`];if(detail)block.text=block.text?`${block.text}\n\n${detail}`:detail;}}
const zhLabels:Record<PageId,string>={quickstart:"快速开始",install:"安装",configure:"配置",cli:"CLI 参考",webui:"Web UI、TUI 与工作流",cluster:"节点与部署",api:"REST API",mcp:"MCP",operations:"运行与运维",upgrade:"升级与回滚",troubleshooting:"故障排查",releases:"版本发布"};
const zhIntro:Record<PageId,string>={quickstart:"安装一个二进制、注册工作区，然后启动任务。",install:"选择 Release 二进制、源码构建，或用 Compose 运行 pure master。",configure:"配置任务、环境、节点身份和认证，同时保留已有状态。",cli:"使用 CLI 完成可重复的操作；支持 --json 机器可读输出。",webui:"在浏览器、终端或自动化客户端中使用同一份 daemon 状态。",cluster:"从单机 worker 扩展到带远程 worker 的 pure master 控制平面。",api:"通过 9837 端口的认证 HTTP API 自动化 Taskdeck。",mcp:"连接内置的 Streamable HTTP MCP 服务。",operations:"使用配额、扩缩容、告警和持久化历史保持服务健康。",upgrade:"升级只来自 Release，先校验 checksum，并且始终由用户确认。",troubleshooting:"快速定位安装和开发中最常见的问题。",releases:"每个稳定版本都提供匹配的二进制、校验和和版本化文档。"};
Object.assign(zhIntro,{
  quickstart:"本页带你从一个空项目开始，完成安装、工作区注册、任务启动和运行状态检查。你会理解 Taskdeck 如何保存状态，以及 CLI、TUI、Web UI 和自动化客户端为什么可以共享同一组标识符。",
  install:"Taskdeck 以单个原生二进制发布，安装的关键是选择正确的平台构建，并决定 daemon 保存状态的位置。本页覆盖带校验和的 Release、源码构建、Compose 部署，以及服务管理器需要的运行条件。",
  configure:"配置分为两层：项目文件描述工作区和任务，节点设置描述 daemon 的身份、角色和认证方式。本页解释配置优先级、环境隔离和在把 leader 暴露到网络前必须做出的安全选择。",
  cli:"CLI 同时面向脚本和交互式使用。命令按生命周期组织，错误信息可用于诊断，并且可以输出稳定 JSON，让部署脚本读取状态而不必解析终端文本。",
  webui:"Web UI 和 TUI 都是同一份 daemon 状态的视图，不会创建第二套执行模型。本页说明两种界面的适用场景，以及工作流、看板、告警、指标和历史记录如何组成完整的运维闭环。",
  cluster:"Taskdeck 可以保持单机运行，也可以在不修改任务定义的情况下扩展为小型控制平面。本页说明 worker 如何主动连接、standard leader 与 pure master 的区别，以及如何为连接配置 TLS 和服务托管。",
  api:"HTTP API 暴露了与 CLI 相同的节点、工作区、session、任务、运行记录、事件和集成资源。先用 health 和 version 检查服务，再使用 bearer token 构建可重复执行的自动化。",
  mcp:"内置 MCP endpoint 为客户端提供由 daemon 管理、带节点范围的工具。本页说明 endpoint 配置、不同节点角色可见的范围、令牌隔离，以及避免助手操作错误工作区的安全边界。",
  operations:"运维功能让长期运行的任务系统更可预测：配额限制资源争抢，依赖定义启动门槛，扩缩容策略响应指标，持久化历史则解释重启后发生过什么。本页可以作为托管部署的运行手册。",
  upgrade:"升级流程刻意保持保守。Taskdeck 检查稳定版本、校验匹配的归档文件、保留回滚副本，并且只有在用户或明确的自动化步骤确认后才替换运行中的二进制。",
  troubleshooting:"任务系统出问题时，要先区分 daemon 健康、进程启动、网络可达性和配置错误。本页按照从本机最便宜的检查到服务和构建诊断的顺序组织排查路径。",
  releases:"Release 不只是上传一个二进制：版本号、校验和、文档快照、服务行为和回滚路径必须一致。本页既说明如何安全使用 Release，也说明发布前如何核对仓库。"
});
const zhTitles:Record<string,string>={"Install and open the project":"安装并打开项目","Register a workspace":"注册工作区","Run and inspect a task":"运行并检查任务","Release binaries":"Release 二进制","Build from source":"源码构建","Docker / Compose":"Docker / Compose","Platform requirements":"平台要求","taskdeck.yaml":"taskdeck.yaml 配置文件","Node roles":"节点角色","Authentication":"认证","Environment variables":"环境变量","Workspace lifecycle":"工作区生命周期","Task lifecycle":"任务生命周期","Node and service management":"节点与服务管理","JSON output":"JSON 输出","Web UI":"Web UI","TUI":"TUI 终端界面","Workflow groups":"工作流组","Boards, alerts and history":"看板、告警与历史","Configure a worker":"配置 worker","Configure a standard leader":"配置 standard leader","Configure a pure master":"配置 pure master","SSH and reverse proxy":"SSH 与反向代理","Authentication and version":"认证与版本","Core routes":"核心路由","Boards, tokens and updates":"看板、令牌与更新","Endpoint and configuration":"端点与配置","Scopes":"作用域","Safe automation":"安全自动化","Quotas and dependencies":"配额与依赖","Autoscaling":"自动扩缩容","Native service manager":"原生服务管理","State and logs":"状态与日志","Audit storage and search":"审计存储与搜索","Check or install":"检查或安装","What happens during an upgrade":"升级过程","Manual fallback":"手动回退","Program not found":"找不到程序","Web UI cannot connect":"Web UI 无法连接","Worker is offline":"Worker 离线","Build failures":"构建失败","Known limits":"已知限制","Versioned docs":"版本化文档","Release checklist":"发布检查清单"};
const zhTexts:Record<string,string>={
  [`releases:v${version}`]:"Taskdeck 0.2.0 聚焦运维可靠性和更安全的默认值：审计历史采用有界保留并限制搜索摘要；当前 schema 数据库打开时不再争抢迁移锁；更多定时任务结果会写入历史；原生安装需要显式授权才能远程监听。本版还说明 SQLite 空间回收步骤和已知后续限制。下载时请从同一个 tag 获取平台归档和 SHA256SUMS。",
  "releases:Known limits":"Taskdeck 离线期间错过的定时触发不会补跑。本次复审未能独立复现所报告的生产历史缺口或任务服务重启问题；范围和后续验证要求见 CHANGELOG.md。",
  "operations:Audit storage and search":"成功的 Web Snapshot、TaskLogs 和 TaskMetrics 轮询请求不会写入审计。已复制的审计历史最多保留最新 10,000 条。Worker 离线时，保留最早 10,000 条待同步记录和最新 10,000 条本地历史；待同步积压超过 20,000 条后，中间记录会被删除。搜索最多索引 request、response 和 details JSON 字段的前 4 KiB；打开单条记录可查看完整已存详情。保留策略释放 SQLite 页面供后续复用，但不会缩小已有的 state.db。回收空间前先停止 daemon 并备份整个状态目录，再执行 `sqlite3 \"${TASKDECK_HOME:-$HOME/.taskdeck}/state.db\" \"VACUUM; PRAGMA wal_checkpoint(TRUNCATE);\"`。Enrollment token 以明文保存在本地 state.db 中；请保护状态目录并清除不再使用的 token。Node API 会隐藏 token。",
  "install:Platform requirements":"归档文件名同时标识操作系统和 CPU 架构，macOS arm64 构建不能替代 Linux arm64。daemon 默认使用 9837 端口并监听 `127.0.0.1`；远程访问需要显式传入 `--allow-remote-bind`。Compose 示例通过 `TASKDECK_ALLOW_REMOTE_BIND=true` 开启远程监听，因此应配置认证、TLS 和私有网络策略。"
};
const pages=(lang:Language)=>lang==="en"?enPages:enPages.map(p=>({...p,label:zhLabels[p.id],intro:zhIntro[p.id],blocks:p.blocks.map(b=>({...b,title:b.title?(zhTitles[b.title]??b.title):b.title,text:b.title?(zhTexts[`${p.id}:${b.title}`]??b.text):b.text,note:b.note?"Taskdeck 会在不影响 daemon 运行的情况下记录状态；需要时可从 GitHub Release 手动安装。":b.note}))}));
const ui={en:{tagline:"PERSISTENT TASK CONTROL PLANE",hero:"Run every project task with a durable control plane.",heroText:"Taskdeck keeps local processes, remote workers, workflows, logs and automation in one observable workspace. One binary powers the CLI, TUI, Web UI and MCP server.",start:"Start in five minutes",releases:"GitHub Releases",search:"Search documentation…",groups:{start:"Get started",work:"Work with Taskdeck",operate:"Operate",reference:"Reference"}},zh:{tagline:"持久化任务控制平面",hero:"让每个项目任务都有可靠的控制平面。",heroText:"Taskdeck 将本地进程、远程 worker、工作流、日志和自动化统一到可观测的工作区。一个二进制同时提供 CLI、TUI、Web UI 和 MCP。",start:"五分钟开始",releases:"GitHub Releases",search:"搜索文档…",groups:{start:"开始使用",work:"使用 Taskdeck",operate:"运行与运维",reference:"参考"}}} as const;

function CodeBlock({code}:{code:string}){const[copied,setCopied]=useState(false);return <div className="code-wrap"><pre><code>{code}</code></pre><button className="copy" onClick={()=>{void navigator.clipboard?.writeText(code);setCopied(true);setTimeout(()=>setCopied(false),1200)}}>{copied?"Copied":"Copy"}</button></div>}
function Downloads({lang}:{lang:Language}){const l=lang==="zh"?{title:"下载当前版本",fallback:"打开 GitHub Release",checksum:"SHA256 校验和"}:{title:"Download the current release",fallback:"Open GitHub Release",checksum:"SHA256 checksums"};const rows=[["Linux","x86_64","x86_64-unknown-linux-gnu","tar.gz"],["Linux","arm64","aarch64-unknown-linux-gnu","tar.gz"],["macOS","x86_64","x86_64-apple-darwin","tar.gz"],["macOS","arm64","aarch64-apple-darwin","tar.gz"],["Windows","x86_64","x86_64-pc-windows-msvc","zip"],["Windows","arm64","aarch64-pc-windows-msvc","zip"]];return <section className="downloads"><span className="eyebrow">RELEASE v{version}</span><h2>{l.title}</h2><div className="download-table"><div className="download-head"><span>Platform</span><span>Architecture</span><span>Asset</span><span/></div>{rows.map(([os,arch,target,ext])=><a className="download-row" href={`${download}/taskdeck-v${version}-${target}.${ext}`} key={target}><span>{os}</span><span>{arch}</span><code>taskdeck-v{version}-{target}.{ext}</code><span>↓</span></a>)}</div><div className="download-links"><a href={`${download}/SHA256SUMS`}>{l.checksum} ↗</a><a href={release}>{l.fallback} ↗</a></div></section>}

function App(){const browserLang:Language=typeof navigator!=="undefined"&&navigator.language.toLowerCase().startsWith("zh")?"zh":"en";const[lang,setLang]=useState<Language>(()=>(localStorage.getItem("taskdeck-docs-lang") as Language)||browserLang);const[theme,setTheme]=useState<Theme>(()=>(localStorage.getItem("taskdeck-docs-theme") as Theme)||"system");const[page,setPage]=useState<PageId>(()=>{const p=location.hash.slice(1) as PageId;return enPages.some(x=>x.id===p)?p:"quickstart"});const[query,setQuery]=useState("");const t=ui[lang],list=pages(lang),current=list.find(x=>x.id===page)??list[0];useEffect(()=>{document.documentElement.dataset.theme=theme==="system"?"":theme;localStorage.setItem("taskdeck-docs-theme",theme)},[theme]);useEffect(()=>{document.documentElement.lang=lang==="zh"?"zh-CN":"en";localStorage.setItem("taskdeck-docs-lang",lang)},[lang]);useEffect(()=>{const f=()=>{const p=location.hash.slice(1) as PageId;if(enPages.some(x=>x.id===p))setPage(p)};addEventListener("hashchange",f);return()=>removeEventListener("hashchange",f)},[]);const filtered=list.filter(p=>!query||`${p.label} ${p.intro}`.toLowerCase().includes(query.toLowerCase()));const go=(id:PageId)=>{setPage(id);location.hash=id;scrollTo({top:0,behavior:"smooth"})};return <div className="site"><header className="topbar"><a href="#quickstart" className="brand" onClick={()=>go("quickstart")}><span className="logo">▦</span><span>taskdeck</span></a><div className="top-search"><span>⌕</span><input value={query} onChange={e=>setQuery(e.target.value)} placeholder={t.search}/><kbd>⌘ K</kbd></div><div className="top-actions"><button onClick={()=>setLang(lang==="en"?"zh":"en")}>{lang==="en"?"中文":"EN"}</button><button onClick={()=>setTheme(theme==="system"?"light":theme==="light"?"dark":"system")}>{theme==="dark"?"☾":theme==="light"?"☀":"◐"}</button><a className="github" href={repo}>GitHub ↗</a></div></header><div className="mobile-hero"><span className="eyebrow">{t.tagline}</span><h1>{t.hero}</h1><p>{t.heroText}</p><div className="hero-actions"><a className="button primary" href={release}>{t.releases} ↗</a><button className="button" onClick={()=>go("quickstart")}>{t.start} →</button></div></div><main className="layout"><aside className="sidebar"><div className="version-select"><span>Version</span><strong>v{version}</strong><span>⌄</span></div>{(["start","work","operate","reference"] as const).map(group=><div className="nav-group" key={group}><h3>{t.groups[group]}</h3>{filtered.filter(p=>p.group===group).map(item=><button key={item.id} className={item.id===page?"active":""} onClick={()=>go(item.id)}>{item.label}</button>)}</div>)}</aside><article className="content"><div className="breadcrumbs"><a href="#quickstart" onClick={()=>go("quickstart")}>Taskdeck</a><span>/</span><span>{current.label}</span></div><span className="eyebrow">{current.group.toUpperCase()} · v{version}</span><h1>{current.label}</h1><p className="intro">{current.intro}</p>{current.id==="quickstart"&&<div className="hero-inline"><span className="eyebrow">{t.tagline}</span><h2>{t.hero}</h2><p>{t.heroText}</p><div className="hero-actions"><a className="button primary" href={release}>{t.releases} ↗</a><a className="button" href={repo}>GitHub ↗</a></div></div>}{current.id==="install"&&<Downloads lang={lang}/>}<div className="doc-body">{current.blocks.map((b,i)=><section className="doc-block" key={`${current.id}-${i}`}>{b.title&&<h2 id={b.title}>{b.title}</h2>}{b.text&&<p>{b.text}</p>}{b.list&&<ul>{b.list.map(item=><li key={item}>{item}</li>)}</ul>}{b.code&&<CodeBlock code={b.code}/>} {b.note&&<div className="note">{b.note}</div>}</section>)}</div><div className="page-nav"><button disabled={list.findIndex(x=>x.id===page)<=0} onClick={()=>go(list[Math.max(0,list.findIndex(x=>x.id===page)-1)].id)}>← Previous</button><button disabled={list.findIndex(x=>x.id===page)>=list.length-1} onClick={()=>go(list[Math.min(list.length-1,list.findIndex(x=>x.id===page)+1)].id)}>Next →</button></div></article><aside className="toc"><strong>On this page</strong>{current.blocks.filter(b=>b.title).map(b=><a href={`#${b.title}`} key={b.title}>{b.title}</a>)}<div className="toc-rule"/><a href={release}>Release v{version} ↗</a><a href={`${download}/SHA256SUMS`}>SHA256SUMS ↗</a></aside></main><footer><span>Taskdeck · MIT License</span><span>v{version}</span><a href={repo}>GitHub ↗</a></footer></div>}
const externalProps={target:"_blank",rel:"noopener noreferrer"} as const;

function highlightCode(code:string):string{
  const escaped=code.replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/>/g,"&gt;");
  return escaped.split("\n").map(line=>{
    const protectedTokens:string[]=[];
    const protect=(value:string,className:string)=>{const token=String.fromCharCode(0xe000+protectedTokens.length);protectedTokens.push(`<span class="${className}">${value}</span>`);return token};
    let output=line.replace(/(#[^&].*)$/g,value=>protect(value,"tok-comment"));
    output=output.replace(/(^|\s)(https?:\/\/[^\s]+)/g,(_,prefix,value)=>prefix+protect(value,"tok-link"));
    output=output.replace(/("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')/g,value=>protect(value,"tok-string"));
    output=output.replace(/\b(true|false|null|undefined)\b/g,'<span class="tok-boolean">$1</span>');
    output=output.replace(/\b(\d+(?:\.\d+)?)\b/g,'<span class="tok-number">$1</span>');
    output=output.replace(/\b(curl|cd|taskdeck|cargo|bun|npm|sudo|docker|export|printf|git|sha256sum|tar|where\.exe)\b/g,'<span class="tok-command">$1</span>');
    protectedTokens.forEach((token,index)=>{output=output.replace(String.fromCharCode(0xe000+index),token)});
    return output;
  }).join("\n");
}

function HighlightedCodeBlock({code}:{code:string}){const[copied,setCopied]=useState(false);return <div className="code-wrap"><pre><code dangerouslySetInnerHTML={{__html:highlightCode(code)}}/></pre><button className="copy" onClick={()=>{void navigator.clipboard?.writeText(code);setCopied(true);setTimeout(()=>setCopied(false),1200)}}>{copied?"Copied":"Copy"}</button></div>}

function SiteFooter({landing=false}:{landing?:boolean}){return <footer className={landing?"landing-footer":"docs-footer"}><div className="footer-brand"><strong>Taskdeck</strong><span>Durable task control for every project.</span></div><div className="footer-links"><div><b>Explore</b><a href={landing?"#capabilities":"#quickstart"}>{landing?"Capabilities":"Quick start"}</a><a href={landing?"#architecture":"#operations"}>{landing?"Architecture":"Operations"}</a></div><div><b>Resources</b><a href="#quickstart">Documentation</a><a href={release} {...externalProps}>Releases ↗</a></div><div><b>Project</b><a href={repo} {...externalProps}>GitHub ↗</a><a href={`${repo}/blob/main/LICENSE`} {...externalProps}>MIT License ↗</a></div></div><div className="footer-meta"><span>v{version}</span><span>© {new Date().getFullYear()} Taskdeck</span></div></footer>}

function RichDownloads({lang}:{lang:Language}){const l=lang==="zh"?{title:"下载当前版本",fallback:"打开 GitHub Release",checksum:"SHA256 校验和",description:"选择平台构建，校验 checksum，然后将二进制放到 PATH。每个压缩包都包含许可证和示例配置。"}:{title:"Download the current release",fallback:"Open GitHub Release",checksum:"SHA256 checksums",description:"Choose a platform build, verify its checksum, then place the binary on your PATH. Each archive includes the license and example configuration."};const rows=[["Linux","x86_64","x86_64-unknown-linux-gnu","tar.gz"],["Linux","arm64","aarch64-unknown-linux-gnu","tar.gz"],["macOS","x86_64","x86_64-apple-darwin","tar.gz"],["macOS","arm64","aarch64-apple-darwin","tar.gz"],["Windows","x86_64","x86_64-pc-windows-msvc","zip"],["Windows","arm64","aarch64-pc-windows-msvc","zip"]];return <section className="downloads"><span className="eyebrow">RELEASE v{version}</span><h2>{l.title}</h2><p className="downloads-description">{l.description}</p><div className="download-table"><div className="download-head"><span>Platform</span><span>Architecture</span><span>Asset</span><span/></div>{rows.map(([os,arch,target,ext])=><a className="download-row" href={`${download}/taskdeck-v${version}-${target}.${ext}`} {...externalProps} key={target}><span>{os}</span><span>{arch}</span><code>taskdeck-v{version}-{target}.{ext}</code><span>↓</span></a>)}</div><div className="download-links"><a href={`${download}/SHA256SUMS`} {...externalProps}>{l.checksum} ↗</a><a href={release} {...externalProps}>{l.fallback} ↗</a></div></section>}

function SiteApp(){const browserLang:Language=typeof navigator!=="undefined"&&navigator.language.toLowerCase().startsWith("zh")?"zh":"en";const[lang,setLang]=useState<Language>(()=>(localStorage.getItem("taskdeck-docs-lang") as Language)||browserLang);const[theme,setTheme]=useState<Theme>(()=>(localStorage.getItem("taskdeck-docs-theme") as Theme)||"system");const[page,setPage]=useState<PageId>(()=>{const p=location.hash.slice(1) as PageId;return enPages.some(x=>x.id===p)?p:"quickstart"});const[query,setQuery]=useState("");const[searchOpen,setSearchOpen]=useState(false);const searchInputRef=useRef<HTMLInputElement>(null);const t=ui[lang],list=pages(lang),current=list.find(x=>x.id===page)??list[0];useEffect(()=>{document.documentElement.dataset.theme=theme==="system"?"":theme;localStorage.setItem("taskdeck-docs-theme",theme)},[theme]);useEffect(()=>{document.documentElement.lang=lang==="zh"?"zh-CN":"en";localStorage.setItem("taskdeck-docs-lang",lang)},[lang]);useEffect(()=>{const onHash=()=>{const p=location.hash.slice(1) as PageId;if(enPages.some(x=>x.id===p))setPage(p)};addEventListener("hashchange",onHash);return()=>removeEventListener("hashchange",onHash)},[]);useEffect(()=>{const onKeyDown=(event:KeyboardEvent)=>{if((event.metaKey||event.ctrlKey)&&event.key.toLowerCase()==="k"){event.preventDefault();setSearchOpen(true)}if(event.key==="Escape")setSearchOpen(false)};addEventListener("keydown",onKeyDown);return()=>removeEventListener("keydown",onKeyDown)},[]);useEffect(()=>{if(searchOpen)requestAnimationFrame(()=>searchInputRef.current?.focus())},[searchOpen]);const filtered=list.filter(p=>!query||`${p.label} ${p.intro}`.toLowerCase().includes(query.toLowerCase()));const searchResults=list.flatMap(p=>[{page:p,title:p.label,description:p.intro},...p.blocks.filter(b=>b.title).map(b=>({page:p,title:b.title!,description:b.text??"Documentation section"}))]).filter(result=>`${result.title} ${result.description}`.toLowerCase().includes(query.toLowerCase())).slice(0,12);const scrollPastBanner=()=>{const banner=document.querySelector<HTMLElement>(".mobile-hero");const topbar=document.querySelector<HTMLElement>(".topbar");const target=(banner?.offsetTop??0)+(banner?.offsetHeight??0)-(topbar?.offsetHeight??0);scrollTo({top:Math.max(0,target),behavior:"smooth"})};const go=(id:PageId)=>{setPage(id);location.hash=id;requestAnimationFrame(scrollPastBanner);setSearchOpen(false)};return <div className="site"><header className="topbar"><a href="#quickstart" className="brand" onClick={()=>go("quickstart")}><span className="logo">▦</span><span>taskdeck</span></a><button className="top-search" type="button" onClick={()=>setSearchOpen(true)} aria-label="Open documentation search"><span>⌕</span><span className="search-label">{t.search}</span><kbd>⌘ K</kbd></button><div className="top-actions"><button onClick={()=>setLang(lang==="en"?"zh":"en")}>{lang==="en"?"中文":"EN"}</button><button onClick={()=>setTheme(theme==="system"?"light":theme==="light"?"dark":"system")}>{theme==="dark"?"☾":theme==="light"?"☀":"◐"}</button><a className="github" href={repo} {...externalProps}>GitHub ↗</a></div></header><div className="mobile-hero"><span className="eyebrow">{t.tagline}</span><h1>{t.hero}</h1><p>{t.heroText}</p><div className="hero-actions"><a className="button primary" href={release} {...externalProps}>{t.releases} ↗</a><button className="button" onClick={()=>go("quickstart")}>{t.start} →</button></div></div><main className="layout"><aside className="sidebar"><div className="version-select"><span>Version</span><strong>v{version}</strong><span>⌄</span></div>{(["start","work","operate","reference"] as const).map(group=><div className="nav-group" key={group}><h3>{t.groups[group]}</h3>{filtered.filter(p=>p.group===group).map(item=><button key={item.id} className={item.id===page?"active":""} onClick={()=>go(item.id)}>{item.label}</button>)}</div>)}</aside><article className="content"><div className="breadcrumbs"><a href="#quickstart" onClick={()=>go("quickstart")}>Taskdeck</a><span>/</span><span>{current.label}</span></div><span className="eyebrow">{current.group.toUpperCase()} · v{version}</span><h1>{current.label}</h1><p className="intro">{current.intro}</p>{current.id==="quickstart"&&<div className="hero-inline"><span className="eyebrow">{t.tagline}</span><h2>{t.hero}</h2><p>{t.heroText}</p><div className="hero-actions"><a className="button primary" href={release} {...externalProps}>{t.releases} ↗</a><a className="button" href={repo} {...externalProps}>GitHub ↗</a></div></div>}{current.id==="install"&&<RichDownloads lang={lang}/>}<div className="doc-body">{current.blocks.map((b,i)=><section className="doc-block" key={`${current.id}-${i}`}>{b.title&&<h2 id={b.title}>{b.title}</h2>}{b.text&&<p>{b.text}</p>}{b.list&&<ul>{b.list.map(item=><li key={item}>{item}</li>)}</ul>}{b.code&&<HighlightedCodeBlock code={b.code}/>} {b.note&&<div className="note">{b.note}</div>}</section>)}</div><div className="page-nav"><button disabled={list.findIndex(x=>x.id===page)<=0} onClick={()=>go(list[Math.max(0,list.findIndex(x=>x.id===page)-1)].id)}>← Previous</button><button disabled={list.findIndex(x=>x.id===page)>=list.length-1} onClick={()=>go(list[Math.min(list.length-1,list.findIndex(x=>x.id===page)+1)].id)}>Next →</button></div></article><aside className="toc"><strong>On this page</strong>{current.blocks.filter(b=>b.title).map(b=><a href={`#${b.title}`} key={b.title}>{b.title}</a>)}<div className="toc-rule"/><a href={release} {...externalProps}>Release v{version} ↗</a><a href={`${download}/SHA256SUMS`} {...externalProps}>SHA256SUMS ↗</a></aside></main><SiteFooter/>{searchOpen&&<div className="search-backdrop" role="presentation" onMouseDown={event=>{if(event.target===event.currentTarget)setSearchOpen(false)}}><section className="search-dialog" role="dialog" aria-modal="true" aria-label="Search documentation"><div className="search-dialog-head"><span className="eyebrow">QUICK SEARCH</span><button type="button" onClick={()=>setSearchOpen(false)}>Esc</button></div><input ref={searchInputRef} value={query} onChange={event=>setQuery(event.target.value)} placeholder={t.search}/><div className="search-results">{searchResults.length?searchResults.map((result,index)=><button type="button" className="search-result" key={`${result.page.id}-${result.title}-${index}`} onClick={()=>go(result.page.id)}><span>{result.page.label}</span><strong>{result.title}</strong><small>{result.description}</small></button>):<p className="search-empty">No matching documentation sections.</p>}</div></section></div>}</div>}

const appRoot=createRoot(document.getElementById("root")!);
appRoot.render(<SiteApp/>);
if(import.meta.hot) import.meta.hot.dispose(()=>appRoot.unmount());
const scrollToDocsContent=()=>{if(!DOC_HASHES.has(location.hash.slice(1)))return;const banner=document.querySelector<HTMLElement>(".mobile-hero");const topbar=document.querySelector<HTMLElement>(".topbar");const target=(banner?.offsetTop??0)+(banner?.offsetHeight??0)-(topbar?.offsetHeight??0);window.scrollTo({top:Math.max(0,target),behavior:"smooth"})};
requestAnimationFrame(scrollToDocsContent);
addEventListener("hashchange",()=>requestAnimationFrame(scrollToDocsContent));
