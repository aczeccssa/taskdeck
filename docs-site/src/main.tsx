import {useEffect, useState} from "react";
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

type Language = "en" | "zh";
type Theme = "system" | "light" | "dark";
type PageId = "quickstart"|"install"|"configure"|"cli"|"webui"|"cluster"|"api"|"mcp"|"operations"|"upgrade"|"troubleshooting"|"releases";
type Block = {title?: string; text?: string; code?: string; list?: string[]; note?: string};
type Page = {id:PageId; label:string; group:string; intro:string; blocks:Block[]};
const version=(import.meta.env.VITE_TASKDECK_VERSION??"0.1.0").replace(/^v/,"");
const repo="https://github.com/aczeccssa/taskdeck";
const release=`${repo}/releases/tag/v${version}`;
const download=`${repo}/releases/download/v${version}`;

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
{title:"Platform requirements",list:["Linux: glibc target x86_64 or arm64; systemd is optional.","macOS: Intel or Apple Silicon; launchd user service is available.","Windows: x86_64 or arm64; Task Scheduler integration uses PowerShell.","The daemon listens on 0.0.0.0:9837 by default; use 127.0.0.1 for local-only access."]}]},
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
  --name master --bind-host 0.0.0.0 --token "$TASKDECK_TOKEN"`},
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
{title:"State and logs",text:"Registrations, task runs, events, workflow revisions and integration records live in TASKDECK_HOME/state.db. Process logs are retained in memory and can be read through the Web UI or logs API."}]},
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
{title:"Versioned docs",text:"The Pages build keeps latest at / and freezes each stable tag under /versions/<tag>/. The version index records release date, release URL, checksum URL and documentation path."},
{title:"Release checklist",list:["Cargo.toml version and v<version> tag must match.","Six platform archives must be present.","SHA256SUMS must cover every archive.","Release notes link to the matching documentation version."]}]}
];
const zhLabels:Record<PageId,string>={quickstart:"快速开始",install:"安装",configure:"配置",cli:"CLI 参考",webui:"Web UI、TUI 与工作流",cluster:"节点与部署",api:"REST API",mcp:"MCP",operations:"运行与运维",upgrade:"升级与回滚",troubleshooting:"故障排查",releases:"版本发布"};
const zhIntro:Record<PageId,string>={quickstart:"安装一个二进制、注册工作区，然后启动任务。",install:"选择 Release 二进制、源码构建，或用 Compose 运行 pure master。",configure:"配置任务、环境、节点身份和认证，同时保留已有状态。",cli:"使用 CLI 完成可重复的操作；支持 --json 机器可读输出。",webui:"在浏览器、终端或自动化客户端中使用同一份 daemon 状态。",cluster:"从单机 worker 扩展到带远程 worker 的 pure master 控制平面。",api:"通过 9837 端口的认证 HTTP API 自动化 Taskdeck。",mcp:"连接内置的 Streamable HTTP MCP 服务。",operations:"使用配额、扩缩容、告警和持久化历史保持服务健康。",upgrade:"升级只来自 Release，先校验 checksum，并且始终由用户确认。",troubleshooting:"快速定位安装和开发中最常见的问题。",releases:"每个稳定版本都提供匹配的二进制、校验和和版本化文档。"};
const zhTitles:Record<string,string>={"Install and open the project":"安装并打开项目","Register a workspace":"注册工作区","Run and inspect a task":"运行并检查任务","Release binaries":"Release 二进制","Build from source":"源码构建","Docker / Compose":"Docker / Compose","Platform requirements":"平台要求","taskdeck.yaml":"taskdeck.yaml 配置文件","Node roles":"节点角色","Authentication":"认证","Environment variables":"环境变量","Workspace lifecycle":"工作区生命周期","Task lifecycle":"任务生命周期","Node and service management":"节点与服务管理","JSON output":"JSON 输出","Web UI":"Web UI","TUI":"TUI 终端界面","Workflow groups":"工作流组","Boards, alerts and history":"看板、告警与历史","Configure a worker":"配置 worker","Configure a standard leader":"配置 standard leader","Configure a pure master":"配置 pure master","SSH and reverse proxy":"SSH 与反向代理","Authentication and version":"认证与版本","Core routes":"核心路由","Boards, tokens and updates":"看板、令牌与更新","Endpoint and configuration":"端点与配置","Scopes":"作用域","Safe automation":"安全自动化","Quotas and dependencies":"配额与依赖","Autoscaling":"自动扩缩容","Native service manager":"原生服务管理","State and logs":"状态与日志","Check or install":"检查或安装","What happens during an upgrade":"升级过程","Manual fallback":"手动回退","Program not found":"找不到程序","Web UI cannot connect":"Web UI 无法连接","Worker is offline":"Worker 离线","Build failures":"构建失败","Versioned docs":"版本化文档","Release checklist":"发布检查清单"};
const pages=(lang:Language)=>lang==="en"?enPages:enPages.map(p=>({...p,label:zhLabels[p.id],intro:zhIntro[p.id],blocks:p.blocks.map(b=>({...b,title:b.title?(zhTitles[b.title]??b.title):b.title,note:b.note?"Taskdeck 会在不影响 daemon 运行的情况下记录状态；需要时可从 GitHub Release 手动安装。":b.note}))}));
const ui={en:{tagline:"PERSISTENT TASK CONTROL PLANE",hero:"Run every project task with a durable control plane.",heroText:"Taskdeck keeps local processes, remote workers, workflows, logs and automation in one observable workspace. One binary powers the CLI, TUI, Web UI and MCP server.",start:"Start in five minutes",releases:"GitHub Releases",search:"Search documentation…",groups:{start:"Get started",work:"Work with Taskdeck",operate:"Operate",reference:"Reference"}},zh:{tagline:"持久化任务控制平面",hero:"让每个项目任务都有可靠的控制平面。",heroText:"Taskdeck 将本地进程、远程 worker、工作流、日志和自动化统一到可观测的工作区。一个二进制同时提供 CLI、TUI、Web UI 和 MCP。",start:"五分钟开始",releases:"GitHub Releases",search:"搜索文档…",groups:{start:"开始使用",work:"使用 Taskdeck",operate:"运行与运维",reference:"参考"}}} as const;

function CodeBlock({code}:{code:string}){const[copied,setCopied]=useState(false);return <div className="code-wrap"><pre><code>{code}</code></pre><button className="copy" onClick={()=>{void navigator.clipboard?.writeText(code);setCopied(true);setTimeout(()=>setCopied(false),1200)}}>{copied?"Copied":"Copy"}</button></div>}
function Downloads({lang}:{lang:Language}){const l=lang==="zh"?{title:"下载当前版本",fallback:"打开 GitHub Release",checksum:"SHA256 校验和"}:{title:"Download the current release",fallback:"Open GitHub Release",checksum:"SHA256 checksums"};const rows=[["Linux","x86_64","x86_64-unknown-linux-gnu","tar.gz"],["Linux","arm64","aarch64-unknown-linux-gnu","tar.gz"],["macOS","x86_64","x86_64-apple-darwin","tar.gz"],["macOS","arm64","aarch64-apple-darwin","tar.gz"],["Windows","x86_64","x86_64-pc-windows-msvc","zip"],["Windows","arm64","aarch64-pc-windows-msvc","zip"]];return <section className="downloads"><span className="eyebrow">RELEASE v{version}</span><h2>{l.title}</h2><div className="download-table"><div className="download-head"><span>Platform</span><span>Architecture</span><span>Asset</span><span/></div>{rows.map(([os,arch,target,ext])=><a className="download-row" href={`${download}/taskdeck-v${version}-${target}.${ext}`} key={target}><span>{os}</span><span>{arch}</span><code>taskdeck-v{version}-{target}.{ext}</code><span>↓</span></a>)}</div><div className="download-links"><a href={`${download}/SHA256SUMS`}>{l.checksum} ↗</a><a href={release}>{l.fallback} ↗</a></div></section>}

function App(){const browserLang:Language=typeof navigator!=="undefined"&&navigator.language.toLowerCase().startsWith("zh")?"zh":"en";const[lang,setLang]=useState<Language>(()=>(localStorage.getItem("taskdeck-docs-lang") as Language)||browserLang);const[theme,setTheme]=useState<Theme>(()=>(localStorage.getItem("taskdeck-docs-theme") as Theme)||"system");const[page,setPage]=useState<PageId>(()=>{const p=location.hash.slice(1) as PageId;return enPages.some(x=>x.id===p)?p:"quickstart"});const[query,setQuery]=useState("");const t=ui[lang],list=pages(lang),current=list.find(x=>x.id===page)??list[0];useEffect(()=>{document.documentElement.dataset.theme=theme==="system"?"":theme;localStorage.setItem("taskdeck-docs-theme",theme)},[theme]);useEffect(()=>{document.documentElement.lang=lang==="zh"?"zh-CN":"en";localStorage.setItem("taskdeck-docs-lang",lang)},[lang]);useEffect(()=>{const f=()=>{const p=location.hash.slice(1) as PageId;if(enPages.some(x=>x.id===p))setPage(p)};addEventListener("hashchange",f);return()=>removeEventListener("hashchange",f)},[]);const filtered=list.filter(p=>!query||`${p.label} ${p.intro}`.toLowerCase().includes(query.toLowerCase()));const go=(id:PageId)=>{setPage(id);location.hash=id;scrollTo({top:0,behavior:"smooth"})};return <div className="site"><header className="topbar"><a href="#quickstart" className="brand" onClick={()=>go("quickstart")}><span className="logo">▦</span><span>taskdeck</span></a><div className="top-search"><span>⌕</span><input value={query} onChange={e=>setQuery(e.target.value)} placeholder={t.search}/><kbd>⌘ K</kbd></div><div className="top-actions"><button onClick={()=>setLang(lang==="en"?"zh":"en")}>{lang==="en"?"中文":"EN"}</button><button onClick={()=>setTheme(theme==="system"?"light":theme==="light"?"dark":"system")}>{theme==="dark"?"☾":theme==="light"?"☀":"◐"}</button><a className="github" href={repo}>GitHub ↗</a></div></header><div className="mobile-hero"><span className="eyebrow">{t.tagline}</span><h1>{t.hero}</h1><p>{t.heroText}</p><div className="hero-actions"><a className="button primary" href={release}>{t.releases} ↗</a><button className="button" onClick={()=>go("quickstart")}>{t.start} →</button></div></div><main className="layout"><aside className="sidebar"><div className="version-select"><span>Version</span><strong>v{version}</strong><span>⌄</span></div>{(["start","work","operate","reference"] as const).map(group=><div className="nav-group" key={group}><h3>{t.groups[group]}</h3>{filtered.filter(p=>p.group===group).map(item=><button key={item.id} className={item.id===page?"active":""} onClick={()=>go(item.id)}>{item.label}</button>)}</div>)}</aside><article className="content"><div className="breadcrumbs"><a href="#quickstart" onClick={()=>go("quickstart")}>Taskdeck</a><span>/</span><span>{current.label}</span></div><span className="eyebrow">{current.group.toUpperCase()} · v{version}</span><h1>{current.label}</h1><p className="intro">{current.intro}</p>{current.id==="quickstart"&&<div className="hero-inline"><span className="eyebrow">{t.tagline}</span><h2>{t.hero}</h2><p>{t.heroText}</p><div className="hero-actions"><a className="button primary" href={release}>{t.releases} ↗</a><a className="button" href={repo}>GitHub ↗</a></div></div>}{current.id==="install"&&<Downloads lang={lang}/>}<div className="doc-body">{current.blocks.map((b,i)=><section className="doc-block" key={`${current.id}-${i}`}>{b.title&&<h2 id={b.title}>{b.title}</h2>}{b.text&&<p>{b.text}</p>}{b.list&&<ul>{b.list.map(item=><li key={item}>{item}</li>)}</ul>}{b.code&&<CodeBlock code={b.code}/>} {b.note&&<div className="note">{b.note}</div>}</section>)}</div><div className="page-nav"><button disabled={list.findIndex(x=>x.id===page)<=0} onClick={()=>go(list[Math.max(0,list.findIndex(x=>x.id===page)-1)].id)}>← Previous</button><button disabled={list.findIndex(x=>x.id===page)>=list.length-1} onClick={()=>go(list[Math.min(list.length-1,list.findIndex(x=>x.id===page)+1)].id)}>Next →</button></div></article><aside className="toc"><strong>On this page</strong>{current.blocks.filter(b=>b.title).map(b=><a href={`#${b.title}`} key={b.title}>{b.title}</a>)}<div className="toc-rule"/><a href={release}>Release v{version} ↗</a><a href={`${download}/SHA256SUMS`}>SHA256SUMS ↗</a></aside></main><footer><span>Taskdeck · MIT License</span><span>v{version}</span><a href={repo}>GitHub ↗</a></footer></div>}
createRoot(document.getElementById("root")!).render(<App/>);
