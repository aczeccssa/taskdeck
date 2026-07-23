"use strict";

const $ = (selector, root = document) => root.querySelector(selector);
const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];

const state = {
  view: "tasks",
  nodes: [],
  nodesSignature: "",
  sessions: [],
  sessionsSignature: "",
  snapshot: null,
  snapshotNode: null,
  currentTask: null,
  renderedTask: null,
  tabsSignature: "",
  headerSignature: "",
  nodesRequest: 0,
  sessionsRequest: 0,
  snapshotRequest: 0,
  metricsRequest: 0,
  logsRequest: 0,
  callsRequest: 0,
  callDetailRequest: 0,
  callDetailId: null,
  configRequest: 0,
  tail: Number(localStorage.getItem("taskdeck-log-tail")) || 1000,
  logLines: [],
  logContext: "",
  logGeneration: null,
  lastLogSeq: null,
  follow: true,
  search: "",
  matchIndex: 0,
  scrollToMatch: false,
  suppressScroll: false,
  workspaceMode: localStorage.getItem("taskdeck-worker-mode") || "split",
  metrics: null,
  calls: [],
  callsSignature: "",
  callPage: { page: 1, page_size: 20, total: 0, total_pages: 0, has_next: false, has_previous: false },
  callFilters: { q: "", operation: "", status: "all", session: "", task: "", page: 1, pageSize: 20 },
  callsDebounce: null,
  config: null,
  configSession: null,
  configNode: null,
  configTasks: [],
  configTaskIndex: 0,
  configSaving: false,
  configDirty: false,
  toastTimer: null,
};

const icons = {
  play: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m8 5 11 7-11 7z"/></svg>',
  pause: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5v14M16 5v14"/></svg>',
  restart: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M19 8V4m0 0h-4m4 0-3 3a7 7 0 1 0 2 8"/></svg>',
  stop: '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>',
  settings: '<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M19 12a7 7 0 0 0-.1-1l2-1.5-2-3.4-2.4 1A7 7 0 0 0 15 6l-.3-2.6h-4L10.4 6A7 7 0 0 0 9 7.1l-2.4-1-2 3.4 2 1.5a7 7 0 0 0 0 2l-2 1.5 2 3.4 2.4-1A7 7 0 0 0 10.4 18l.3 2.6h4L15 18a7 7 0 0 0 1.5-1.1l2.4 1 2-3.4-2-1.5a7 7 0 0 0 .1-1z"/></svg>',
};

function escapeHtml(value) {
  return String(value ?? "").replace(/[&<>"']/g, (char) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[char]);
}

function escapeAttr(value) {
  return escapeHtml(value);
}

async function requestJson(url, options) {
  const response = await fetch(url, options);
  const payload = await response.json();
  return payload;
}

function showToast(message) {
  const toast = $("#toast");
  toast.textContent = message;
  toast.classList.add("visible");
  clearTimeout(state.toastTimer);
  state.toastTimer = setTimeout(() => {
    toast.classList.remove("visible");
    toast.textContent = "";
  }, 2200);
}

function setConnection(connected) {
  const connection = $("#connection-state");
  connection.classList.toggle("offline", !connected);
  $("span", connection).textContent = connected ? "Daemon connected" : "Daemon unavailable";
}

function setSidebar(collapsed) {
  $("#app-shell").classList.toggle("sidebar-collapsed", collapsed);
  const button = $("#sidebar-toggle");
  button.setAttribute("aria-expanded", String(!collapsed));
  button.setAttribute("aria-label", collapsed ? "Expand navigation" : "Collapse navigation");
  button.title = collapsed ? "Expand navigation" : "Collapse navigation";
  localStorage.setItem("taskdeck-sidebar-collapsed", String(collapsed));
}

function setTheme() {
  const current = document.documentElement.dataset.theme || "system";
  const next = current === "system" ? "light" : current === "light" ? "dark" : "system";
  if (next === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = next;
  localStorage.setItem("taskdeck-theme", next);
  showToast(`Theme: ${next}`);
}

function applySavedPreferences() {
  const theme = localStorage.getItem("taskdeck-theme");
  if (theme && theme !== "system") document.documentElement.dataset.theme = theme;
  setSidebar(localStorage.getItem("taskdeck-sidebar-collapsed") === "true");
  if (![100, 500, 1000, 5000].includes(state.tail)) state.tail = 1000;
  if (!["split", "monitor", "log"].includes(state.workspaceMode)) state.workspaceMode = "split";
}

function setView(view) {
  state.view = view;
  $$(".nav-button").forEach((button) => button.classList.toggle("active", button.dataset.view === view));
  $$(".view").forEach((panel) => {
    const active = panel.id === `${view}-view`;
    panel.hidden = !active;
    panel.classList.toggle("active", active);
  });
  $("#sessions").hidden = view !== "tasks";
  $("#nodes").hidden = view !== "tasks";
  $("#page-title").textContent = view === "calls" ? "MCP Calls" : view === "docs" ? "MCP Guide" : "Task workspace";
  if (view === "calls") loadMcpCalls();
  updateMeta();
}

function selectedNode() {
  return $("#nodes").value;
}

function selectedNodeState() {
  return state.nodes.find((node) => node.id === selectedNode()) || null;
}

function addNodeQuery(query = new URLSearchParams()) {
  const node = selectedNode();
  if (node) query.set("node", node);
  return query;
}

async function loadNodes() {
  const requestId = ++state.nodesRequest;
  try {
    const response = await requestJson("/api/nodes");
    if (requestId !== state.nodesRequest) return;
    if (!response.ok) throw new Error(response.message);
    const nodes = response.data || [];
    const select = $("#nodes");
    const selected = select.value;
    state.nodes = nodes;
    const signature = JSON.stringify(nodes);
    if (signature !== state.nodesSignature) {
      state.nodesSignature = signature;
      select.innerHTML = nodes.length
        ? nodes.map((node) => `<option value="${escapeAttr(node.id)}">${escapeHtml(node.is_self ? `This device · ${node.name}` : `${node.name}${node.online ? "" : " · offline"}`)}</option>`).join("")
        : '<option value="">No nodes</option>';
      state.headerSignature = "";
    }
    if (nodes.some((node) => node.id === selected)) select.value = selected;
    else if (nodes.length) select.value = nodes[0].id;
    if (state.snapshot && state.snapshotNode === select.value) renderTask();
    await loadSessions();
    setConnection(true);
  } catch (error) {
    if (requestId !== state.nodesRequest) return;
    setConnection(false);
    if (state.view === "tasks") $("#meta").textContent = error.message || "Daemon unavailable";
  }
}

function updateMeta() {
  const meta = $("#meta");
  if (state.view === "calls") {
    meta.textContent = `${state.callPage.total || 0} retained call${state.callPage.total === 1 ? "" : "s"}`;
  } else if (state.view === "docs") {
    meta.textContent = "Local Streamable HTTP endpoint";
  } else if (state.snapshot) {
    meta.textContent = `${state.snapshot.project} - ${state.snapshot.source}`;
  } else {
    meta.textContent = "No sessions registered";
  }
}

async function loadSessions() {
  const requestId = ++state.sessionsRequest;
  const node = selectedNode();
  let daemonReached = false;
  if (!node) {
    clearWorkspace();
    return;
  }
  const nodeState = selectedNodeState();
  if (nodeState?.online === false) {
    const sessions = nodeState.sessions || [];
    const select = $("#sessions");
    const selected = select.value;
    state.sessions = sessions;
    state.sessionsSignature = JSON.stringify(sessions);
    select.innerHTML = sessions.length
      ? sessions.map((name) => `<option value="${escapeAttr(name)}">${escapeHtml(name)}</option>`).join("")
      : '<option value="">No cached sessions</option>';
    if (sessions.includes(selected)) select.value = selected;
    else if (sessions.length) select.value = sessions[0];
    setConnection(true);
    if (state.snapshot && state.snapshotNode === node) {
      renderTask();
    } else {
      clearWorkspace();
      const lastSeen = nodeState.last_seen_ms ? new Date(nodeState.last_seen_ms).toLocaleString() : "Unknown";
      $("#task-pane").innerHTML = `<div class="empty-state"><div><h1>Worker offline</h1><p>Last seen ${escapeHtml(lastSeen)}</p></div></div>`;
    }
    $("#meta").textContent = `${nodeState.name} is offline`;
    return;
  }
  try {
    const response = await requestJson(`/api/sessions?${addNodeQuery()}`);
    daemonReached = true;
    if (requestId !== state.sessionsRequest || selectedNode() !== node) return;
    if (!response.ok) throw new Error(response.message);
    const sessions = response.data || [];
    const select = $("#sessions");
    const selected = select.value;
    state.sessions = sessions;
    const signature = JSON.stringify(sessions);
    if (signature !== state.sessionsSignature) {
      state.sessionsSignature = signature;
      select.innerHTML = sessions.length
        ? sessions.map((name) => `<option value="${escapeAttr(name)}">${escapeHtml(name)}</option>`).join("")
        : '<option value="">No sessions</option>';
    }
    if (sessions.includes(selected)) select.value = selected;
    else if (sessions.length) select.value = sessions[0];
    setConnection(true);
    if (sessions.length && (!state.snapshot || state.snapshot.name !== select.value || state.snapshotNode !== node)) {
      state.currentTask = null;
      state.renderedTask = null;
      await loadSnapshot();
    } else if (!sessions.length) {
      clearWorkspace();
    }
  } catch (error) {
    if (requestId !== state.sessionsRequest || selectedNode() !== node) return;
    setConnection(daemonReached);
    if (state.view === "tasks") $("#meta").textContent = daemonReached ? error.message : "Daemon unavailable";
  }
}

function clearWorkspace() {
  state.snapshotRequest += 1;
  state.metricsRequest += 1;
  state.logsRequest += 1;
  state.snapshot = null;
  state.snapshotNode = null;
  state.currentTask = null;
  state.renderedTask = null;
  state.metrics = null;
  state.tabsSignature = "";
  state.headerSignature = "";
  resetLogCursor();
  $("#tabs").innerHTML = "";
  $("#task-pane").innerHTML = '<div class="empty-state"><div><h1>No active workspace</h1><p>Register a project from the CLI or TUI.</p></div></div>';
  updateMeta();
}

async function loadSnapshot() {
  const session = $("#sessions").value;
  const node = selectedNode();
  if (!session || !node) return;
  const requestId = ++state.snapshotRequest;
  try {
    const query = addNodeQuery(new URLSearchParams({ tail: "0" }));
    const response = await requestJson(`/api/sessions/${encodeURIComponent(session)}?${query}`);
    if (requestId !== state.snapshotRequest || $("#sessions").value !== session || selectedNode() !== node) return;
    if (!response.ok) throw new Error(response.message);
    state.snapshot = response.data;
    state.snapshotNode = node;
    const labels = Object.keys(state.snapshot.tasks || {});
    if (!labels.includes(state.currentTask)) {
      state.currentTask = labels[0] || null;
      state.renderedTask = null;
      state.metrics = null;
      resetLogCursor();
    }
    renderTabs(labels);
    renderTask();
    if (!state.logContext) loadLogs();
    updateMeta();
    setConnection(true);
  } catch (error) {
    $("#meta").textContent = error.message || "Snapshot unavailable";
  }
}

function renderTabs(labels) {
  const signature = JSON.stringify([labels, state.currentTask]);
  if (signature === state.tabsSignature) return;
  state.tabsSignature = signature;
  $("#tabs").innerHTML = labels.map((label) => `<button class="tab ${label === state.currentTask ? "active" : ""}" type="button" role="tab" tabindex="${label === state.currentTask ? "0" : "-1"}" aria-selected="${label === state.currentTask}" data-task="${escapeAttr(label)}">${escapeHtml(label)}</button>`).join("");
}

function ensureTaskScaffold() {
  if (!state.currentTask || state.renderedTask === state.currentTask) return;
  state.renderedTask = state.currentTask;
  state.follow = true;
  state.search = "";
  state.matchIndex = 0;
  $("#task-pane").innerHTML = `
    <div class="task-header" id="task-header"></div>
    <div class="worker-stage" id="worker-stage">
      <section class="log-panel" id="log-panel" aria-label="Task logs">
        <div class="log-toolbar">
          <select id="log-tail" aria-label="Maximum log lines" title="Maximum log lines">
            ${[100, 500, 1000, 5000].map((value) => `<option value="${value}" ${value === state.tail ? "selected" : ""}>${value} lines</option>`).join("")}
          </select>
          <div class="log-search">
            <input id="log-search" type="search" placeholder="Search output" aria-label="Search output" autocomplete="off">
            <span id="log-match-count">0 / 0</span>
          </div>
          <button class="icon-button" type="button" data-log="previous" aria-label="Previous match" title="Previous match">&#8593;</button>
          <button class="icon-button" type="button" data-log="next" aria-label="Next match" title="Next match">&#8595;</button>
          <button class="icon-button" type="button" data-log="clear" aria-label="Clear search" title="Clear search">&#215;</button>
          <button class="icon-button" type="button" data-log="top" aria-label="Go to top" title="Go to top">&#8679;</button>
          <button class="icon-button" type="button" data-log="bottom" aria-label="Go to bottom" title="Go to bottom">&#8681;</button>
          <button class="button compact" id="follow-button" type="button" data-log="follow">Focus</button>
          <button class="icon-button" type="button" data-log="fullscreen" aria-label="Full screen logs" title="Full screen logs">&#x26F6;</button>
          <span class="log-line-count" id="log-line-count">0 lines</span>
        </div>
        <div class="logs" id="logs" tabindex="0"></div>
      </section>
      <aside class="monitor-panel" id="monitor-panel" aria-label="Task performance">
        <header class="monitor-header"><strong>Performance</strong><span id="monitor-state">Waiting</span></header>
        <div class="monitor-body" id="monitor-body"><div class="monitor-empty">No samples</div></div>
      </aside>
    </div>`;
  bindLogControls();
  applyWorkspaceMode();
}

function renderTask() {
  if (!state.snapshot || !state.currentTask) {
    clearWorkspace();
    return;
  }
  ensureTaskScaffold();
  const task = state.snapshot.tasks[state.currentTask];
  renderTaskHeader(task);
  renderLogs(state.logLines);
  renderMetrics();
}

function renderTaskHeader(task) {
  const status = task.status || "unknown";
  const service = task.service || {};
  const node = selectedNodeState();
  const online = node?.online !== false;
  const signature = JSON.stringify([state.currentTask, status, task.pid, task.cwd, task.label, service, online]);
  if (signature === state.headerSignature) {
    applyWorkspaceMode();
    return;
  }
  state.headerSignature = signature;
  const focused = document.activeElement?.closest?.("[data-action],[data-mode],[data-config]");
  const focusSelector = focused?.dataset.action ? `[data-action="${focused.dataset.action}"]` : focused?.dataset.mode ? `[data-mode="${focused.dataset.mode}"]` : focused?.hasAttribute("data-config") ? "[data-config]" : null;
  const canStart = ["idle", "exited", "failed"].includes(status);
  const canPause = status === "running";
  const canResume = status === "paused";
  const canStop = ["running", "paused"].includes(status);
  const technology = service.technology || {};
  const technologyLabel = technology.framework || technology.runtime || "";
  const endpointMarkup = (service.endpoints || []).map((endpoint) => {
    const label = `${endpoint.bind_host}:${endpoint.port}`;
    const isLink = endpoint.state === "listening" && ["http", "https"].includes(endpoint.protocol);
    return isLink
      ? `<a class="endpoint-chip listening" href="${escapeAttr(`${endpoint.protocol}://${endpoint.bind_host}:${endpoint.port}`)}" target="_blank" rel="noreferrer">${escapeHtml(label)}</a>`
      : `<span class="endpoint-chip ${escapeAttr(endpoint.state || "")}" title="${escapeAttr(`${endpoint.source || "unknown"} · ${endpoint.state || "unknown"}`)}">${escapeHtml(label)}</span>`;
  }).join("");
  $("#task-header").innerHTML = `
    <div class="task-heading"><div class="eyebrow">${online ? "Selected task" : "Offline snapshot"}</div><h1>${escapeHtml(task.label)}</h1><div class="task-detail"><span class="status ${escapeAttr(status)}">${escapeHtml(status)}</span>${task.pid ? `<span>PID ${task.pid}</span>` : ""}${technologyLabel ? `<span class="technology-chip" title="${escapeAttr((technology.evidence || []).join(" · "))}">${escapeHtml(technologyLabel)}</span>` : ""}${endpointMarkup}<span class="cwd">${escapeHtml(task.cwd)}</span></div></div>
    <div class="view-modes" aria-label="Workspace layout">
      <button class="mobile-log-mode" type="button" data-mode="log">Logs</button>
      <button class="desktop-split-mode" type="button" data-mode="split">Split</button>
      <button type="button" data-mode="monitor">Monitor</button>
    </div>
    <div class="actions">
      <button class="button primary" type="button" data-action="start" ${online && canStart ? "" : "disabled"}>${icons.play}Start</button>
      <button class="button" type="button" data-action="pause" ${online && canPause ? "" : "disabled"}>${icons.pause}Pause</button>
      <button class="button" type="button" data-action="resume" ${online && canResume ? "" : "disabled"}>${icons.play}Resume</button>
      <button class="button" type="button" data-action="restart" ${online ? "" : "disabled"}>${icons.restart}Restart</button>
      <button class="button danger" type="button" data-action="stop" ${online && canStop ? "" : "disabled"}>${icons.stop}Stop</button>
      <button class="icon-button" type="button" data-config aria-label="Edit configuration" title="Edit configuration" ${online ? "" : "disabled"}>${icons.settings}</button>
    </div>`;
  applyWorkspaceMode();
  if (focusSelector) $(focusSelector, $("#task-header"))?.focus();
}

function bindLogControls() {
  const logs = $("#logs");
  logs.addEventListener("scroll", () => {
    if (state.suppressScroll) return;
    const atBottom = logs.scrollHeight - logs.scrollTop - logs.clientHeight < 8;
    if (!atBottom) state.follow = false;
    updateFollowButton();
  }, { passive: true });
  $("#log-search").addEventListener("input", (event) => {
    state.search = event.target.value;
    state.matchIndex = 0;
    state.scrollToMatch = Boolean(state.search);
    if (state.search) state.follow = false;
    renderLogs(currentLogs());
  });
  $("#log-search").addEventListener("keydown", (event) => {
    if (event.key !== "Enter") return;
    event.preventDefault();
    moveMatch(event.shiftKey ? -1 : 1);
  });
  $("#log-tail").addEventListener("change", (event) => {
    state.tail = Number(event.target.value);
    localStorage.setItem("taskdeck-log-tail", String(state.tail));
    resetLogCursor();
    loadLogs();
  });
}

function currentLogs() {
  return state.logLines;
}

function resetLogCursor() {
  state.logLines = [];
  state.logGeneration = null;
  state.lastLogSeq = null;
  state.logContext = "";
}

async function loadLogs() {
  const session = $("#sessions").value;
  const task = state.currentTask;
  const node = selectedNode();
  if (!session || !task || !node) return;
  const context = `${node}\u0000${session}\u0000${task}\u0000${state.tail}`;
  if (state.logContext !== context) {
    state.logContext = context;
    state.logLines = [];
    state.logGeneration = null;
    state.lastLogSeq = null;
  }
  const requestId = ++state.logsRequest;
  const query = addNodeQuery(new URLSearchParams({ limit: String(state.tail) }));
  if (state.lastLogSeq != null) query.set("after", String(state.lastLogSeq));
  try {
    const response = await requestJson(`/api/sessions/${encodeURIComponent(session)}/tasks/${encodeURIComponent(task)}/logs?${query}`);
    if (requestId !== state.logsRequest || selectedNode() !== node || $("#sessions").value !== session || state.currentTask !== task || state.logContext !== context) return;
    if (!response.ok) throw new Error(response.message);
    const payload = response.data || { generation: null, reset: false, lines: [] };
    const generationChanged = state.logGeneration != null && payload.generation !== state.logGeneration;
    if (payload.reset || generationChanged || state.lastLogSeq == null) {
      state.logLines = payload.lines || [];
      state.lastLogSeq = state.logLines.at(-1)?.seq ?? null;
    } else {
      state.logLines = [...state.logLines, ...(payload.lines || [])].slice(-state.tail);
      state.lastLogSeq = state.logLines.at(-1)?.seq ?? state.lastLogSeq;
    }
    state.logGeneration = payload.generation ?? null;
    renderLogs(state.logLines);
  } catch (error) {
    if (requestId === state.logsRequest) showToast(error.message || "Logs unavailable");
  }
}

function matchOffsets(text, query) {
  if (!query) return [];
  const value = String(text ?? "");
  const lower = value.toLocaleLowerCase();
  const needle = query.toLocaleLowerCase();
  if (!needle) return [];
  const offsets = [];
  let index = lower.indexOf(needle);
  while (index >= 0) {
    offsets.push(index);
    index = lower.indexOf(needle, index + Math.max(needle.length, 1));
  }
  return offsets;
}

function highlightedText(text, query, currentOccurrence) {
  const value = String(text ?? "");
  if (!query) return escapeHtml(value);
  const needleLength = query.toLocaleLowerCase().length;
  const offsets = matchOffsets(value, query);
  if (!offsets.length) return escapeHtml(value);
  let output = "";
  let cursor = 0;
  offsets.forEach((index, occurrence) => {
    output += escapeHtml(value.slice(cursor, index));
    output += `<mark class="${occurrence === currentOccurrence ? "current-hit" : ""}">${escapeHtml(value.slice(index, index + needleLength))}</mark>`;
    cursor = index + needleLength;
  });
  return output + escapeHtml(value.slice(cursor));
}

function logRowMarkup(line, index, query, offsets, currentMatch) {
  const match = offsets.length > 0;
  const current = match && index === currentMatch?.lineIndex;
  return `<div class="log-row ${escapeAttr(line.stream)} ${match ? "match" : ""} ${current ? "current-match" : ""}" data-line-index="${index}" data-seq="${line.seq}"><span class="log-number">${line.seq}</span><span class="log-text">${highlightedText(line.text, query, current ? currentMatch.occurrenceIndex : -1)}</span></div>`;
}

function renderLogs(lines) {
  const container = $("#logs");
  if (!container) return;
  const previousTop = container.scrollTop;
  const query = state.search.trim();
  const matches = [];
  const offsetsByLine = lines.map((line, lineIndex) => {
    const offsets = matchOffsets(line.text, query);
    offsets.forEach((_, occurrenceIndex) => matches.push({ lineIndex, occurrenceIndex }));
    return offsets;
  });
  if (!matches.length) state.matchIndex = 0;
  else state.matchIndex = ((state.matchIndex % matches.length) + matches.length) % matches.length;
  const currentMatch = matches[state.matchIndex];
  const currentLineIndex = currentMatch?.lineIndex;
  const existingRows = $$(".log-row", container);
  const existingLast = Number(existingRows.at(-1)?.dataset.seq);
  const firstSeq = lines[0]?.seq;
  const lastSeq = lines.at(-1)?.seq;
  const canPatch = !query
    && container.dataset.logContext === state.logContext
    && container.dataset.logGeneration === String(state.logGeneration ?? "")
    && container.dataset.search === ""
    && existingRows.length > 0
    && Number.isFinite(existingLast)
    && lines.some((line) => line.seq === existingLast);

  let removedHeight = 0;
  if (canPatch) {
    existingRows.forEach((row) => {
      if (Number(row.dataset.seq) < firstSeq) {
        removedHeight += row.getBoundingClientRect().height;
        row.remove();
      }
    });
    const additions = lines
      .map((line, index) => ({ line, index }))
      .filter(({ line }) => line.seq > existingLast)
      .map(({ line, index }) => logRowMarkup(line, index, "", [], null))
      .join("");
    if (additions) container.insertAdjacentHTML("beforeend", additions);
  } else {
    container.innerHTML = lines.length
      ? lines.map((line, index) => logRowMarkup(line, index, query, offsetsByLine[index], currentMatch)).join("")
      : '<div class="log-empty">No output yet.</div>';
  }
  container.dataset.logContext = state.logContext;
  container.dataset.logGeneration = state.logGeneration ?? "";
  container.dataset.search = query;
  container.dataset.firstSeq = firstSeq ?? "";
  container.dataset.lastSeq = lastSeq ?? "";
  $("#log-line-count").textContent = `${lines.length} line${lines.length === 1 ? "" : "s"}`;
  $("#log-match-count").textContent = matches.length ? `${state.matchIndex + 1} / ${matches.length}` : "0 / 0";
  state.suppressScroll = true;
  if (state.scrollToMatch && currentLineIndex != null) {
    $(`[data-line-index="${currentLineIndex}"]`, container)?.scrollIntoView({ block: "center" });
    state.scrollToMatch = false;
  } else if (state.follow) {
    container.scrollTop = container.scrollHeight;
  } else {
    container.scrollTop = Math.max(0, previousTop - removedHeight);
  }
  requestAnimationFrame(() => { state.suppressScroll = false; });
  updateFollowButton();
}

function updateFollowButton() {
  const button = $("#follow-button");
  if (!button) return;
  button.textContent = state.follow ? "Unfocus" : "Focus";
  button.setAttribute("aria-pressed", String(state.follow));
}

function moveMatch(direction) {
  if (!state.search.trim()) return;
  const total = currentLogs().reduce((count, line) => count + matchOffsets(line.text, state.search.trim()).length, 0);
  if (!total) return;
  state.follow = false;
  state.matchIndex = (state.matchIndex + direction + total) % total;
  state.scrollToMatch = true;
  renderLogs(currentLogs());
}

function handleLogAction(action) {
  const logs = $("#logs");
  if (!logs) return;
  if (action === "previous") moveMatch(-1);
  if (action === "next") moveMatch(1);
  if (action === "clear") {
    state.search = "";
    state.matchIndex = 0;
    $("#log-search").value = "";
    renderLogs(currentLogs());
    $("#log-search").focus();
  }
  if (action === "top") {
    state.follow = false;
    logs.scrollTop = 0;
    updateFollowButton();
  }
  if (action === "bottom") {
    state.follow = true;
    logs.scrollTop = logs.scrollHeight;
    updateFollowButton();
  }
  if (action === "follow") {
    state.follow = !state.follow;
    if (state.follow) logs.scrollTop = logs.scrollHeight;
    updateFollowButton();
  }
  if (action === "fullscreen") toggleLogFullscreen();
}

async function toggleLogFullscreen() {
  const panel = $("#log-panel");
  if (document.fullscreenElement === panel) {
    await document.exitFullscreen();
    return;
  }
  if (panel.requestFullscreen) {
    try { await panel.requestFullscreen(); return; } catch (_) { /* fallback below */ }
  }
  panel.classList.toggle("fallback-fullscreen");
  updateFullscreenButton();
}

function updateFullscreenButton() {
  const button = $("[data-log=\"fullscreen\"]");
  const active = document.fullscreenElement === $("#log-panel") || $("#log-panel")?.classList.contains("fallback-fullscreen");
  if (button) {
    button.setAttribute("aria-pressed", String(Boolean(active)));
    button.title = active ? "Exit full screen" : "Full screen logs";
  }
}

function applyWorkspaceMode() {
  const stage = $("#worker-stage");
  if (!stage) return;
  const narrow = matchMedia("(max-width: 820px)").matches;
  let mode = state.workspaceMode;
  if (narrow && mode === "split") mode = "log";
  if (!narrow && mode === "log") mode = "split";
  stage.className = `worker-stage mode-${mode}`;
  $$("[data-mode]", $("#task-header")).forEach((button) => {
    const active = button.dataset.mode === mode || (narrow && button.dataset.mode === "log" && mode === "split");
    button.classList.toggle("active", active);
    button.setAttribute("aria-pressed", String(active));
  });
}

function setWorkspaceMode(mode) {
  state.workspaceMode = mode;
  localStorage.setItem("taskdeck-worker-mode", mode);
  applyWorkspaceMode();
  if (mode === "monitor") loadMetrics();
}

async function act(action, button) {
  const session = state.snapshot?.name;
  const task = state.currentTask;
  const node = selectedNode();
  if (!node || !session || !task || $("#sessions").value !== session || button.disabled) return;
  button.disabled = true;
  try {
    const response = await requestJson("/api/action", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ node, session, task, action }),
    });
    if (!response.ok) throw new Error(response.message);
    if ($("#sessions").value === session && state.currentTask === task) await loadSnapshot();
  } catch (error) {
    showToast(error.message || "Action failed");
  } finally {
    button.disabled = false;
  }
}

async function loadMetrics() {
  const session = $("#sessions").value;
  const task = state.currentTask;
  const node = selectedNode();
  if (!node || !session || !task) return;
  const requestId = ++state.metricsRequest;
  try {
    const query = addNodeQuery(new URLSearchParams({ window: "600" }));
    const response = await requestJson(`/api/sessions/${encodeURIComponent(session)}/tasks/${encodeURIComponent(task)}/metrics?${query}`);
    if (requestId !== state.metricsRequest || selectedNode() !== node || $("#sessions").value !== session || state.currentTask !== task) return;
    if (!response.ok) throw new Error(response.message);
    state.metrics = response.data;
    renderMetrics();
  } catch (error) {
    const label = $("#monitor-state");
    if (requestId === state.metricsRequest && label) label.textContent = "Unavailable";
  }
}

function formatBytes(bytes) {
  const value = Number(bytes || 0);
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let size = value;
  let unit = -1;
  do { size /= 1024; unit += 1; } while (size >= 1024 && unit < units.length - 1);
  return `${size >= 100 ? size.toFixed(0) : size.toFixed(1)} ${units[unit]}`;
}

function formatRuntime(seconds) {
  const value = Number(seconds || 0);
  if (value < 60) return `${value}s`;
  if (value < 3600) return `${Math.floor(value / 60)}m ${value % 60}s`;
  return `${Math.floor(value / 3600)}h ${Math.floor((value % 3600) / 60)}m`;
}

function chartMarkup(samples, key, lineClass, formatter) {
  const values = samples.map((sample) => Number(sample[key] || 0));
  const max = Math.max(...values, 1);
  const points = values.map((value, index) => {
    const x = values.length <= 1 ? 0 : (index / (values.length - 1)) * 300;
    const y = 70 - (value / max) * 64;
    return `${x.toFixed(2)},${y.toFixed(2)}`;
  }).join(" ");
  return `<svg viewBox="0 0 300 72" preserveAspectRatio="none" aria-hidden="true"><path class="chart-grid" d="M0 70H300M0 38H300M0 6H300"/>${points ? `<polyline class="chart-line ${lineClass}" points="${points}"/>` : ""}</svg><div class="chart-title"><span>10 minute history</span><strong>${formatter(max)}</strong></div>`;
}

function renderMetrics() {
  const body = $("#monitor-body");
  if (!body) return;
  const previousTop = body.scrollTop;
  const previousLeft = body.scrollLeft;
  const metrics = state.metrics;
  if (!metrics) {
    body.innerHTML = '<div class="monitor-empty">No samples</div>';
    return;
  }
  const current = metrics.current || { cpu_percent: 0, memory_bytes: 0, process_count: 0 };
  const samples = metrics.samples || [];
  $("#monitor-state").textContent = metrics.running ? "Live - 1s" : "Stopped";
  const processRows = (metrics.processes || []).map((process) => `<tr><td>${process.pid}</td><td>${process.ppid ?? "-"}</td><td class="name" title="${escapeAttr(process.name)}">${escapeHtml(process.name)}</td><td>${Number(process.cpu_percent || 0).toFixed(1)}%</td><td>${formatBytes(process.memory_bytes)}</td><td>${escapeHtml(process.status)}</td><td>${formatRuntime(process.run_time_seconds)}</td></tr>`).join("");
  body.innerHTML = `
    <div class="metric-summary"><div><span>CPU</span><strong>${Number(current.cpu_percent || 0).toFixed(1)}%</strong></div><div><span>RSS</span><strong>${formatBytes(current.memory_bytes)}</strong></div><div><span>Processes</span><strong>${current.process_count || 0}</strong></div></div>
    <div class="metric-charts"><div class="metric-chart"><div class="chart-title"><span>CPU</span><strong>${Number(current.cpu_percent || 0).toFixed(1)}%</strong></div>${chartMarkup(samples, "cpu_percent", "", (value) => `${value.toFixed(1)}%`)}</div><div class="metric-chart"><div class="chart-title"><span>Memory</span><strong>${formatBytes(current.memory_bytes)}</strong></div>${chartMarkup(samples, "memory_bytes", "memory", formatBytes)}</div></div>
    <div class="process-table-wrap">${processRows ? `<table class="processes"><thead><tr><th>PID</th><th>PPID</th><th>Name</th><th>CPU</th><th>RSS</th><th>Status</th><th>Runtime</th></tr></thead><tbody>${processRows}</tbody></table>` : '<div class="monitor-empty">No running processes</div>'}</div>`;
  body.scrollTop = previousTop;
  body.scrollLeft = previousLeft;
}

function mcpTarget(input) {
  if (input?.action === "sessions") return { primary: "All sessions", secondary: "Global daemon" };
  if (input?.task) return { primary: input.task, secondary: input.session || "Task" };
  if (input?.session) return { primary: input.session, secondary: input.tail ? `Last ${input.tail} lines` : "Session" };
  return { primary: "Taskdeck daemon", secondary: "No target" };
}

function callsQuery() {
  const filters = state.callFilters;
  const query = new URLSearchParams({ page: String(filters.page), page_size: String(filters.pageSize), status: filters.status });
  if (filters.q) query.set("q", filters.q);
  if (filters.operation) query.set("operation", filters.operation);
  if (filters.session) query.set("session", filters.session);
  if (filters.task) query.set("task", filters.task);
  return query.toString();
}

async function loadMcpCalls() {
  const query = callsQuery();
  const requestId = ++state.callsRequest;
  try {
    const response = await requestJson(`/api/mcp-calls?${query}`);
    if (requestId !== state.callsRequest || query !== callsQuery()) return;
    if (!response.ok) throw new Error(response.message);
    state.callPage = response.data || state.callPage;
    state.calls = state.callPage.items || [];
    renderMcpCalls();
    updateMeta();
  } catch (error) {
    if (requestId === state.callsRequest) $("#calls-summary").textContent = error.message || "Call history unavailable";
  }
}

function renderMcpCalls() {
  const body = $("#calls-body");
  const signature = JSON.stringify(state.calls.map((call) => [call.id, call.success, call.duration_ms]));
  if (signature !== state.callsSignature) {
    state.callsSignature = signature;
    const focusedCall = document.activeElement?.closest?.("[data-call-id]")?.dataset.callId;
    body.innerHTML = state.calls.length ? state.calls.map((call) => {
      const target = mcpTarget(call.input || {});
      return `<tr><td><div class="cell-stack"><strong>${escapeHtml(titleCase(call.operation || "MCP call"))}</strong><span>${escapeHtml(call.tool)}</span></div></td><td><div class="cell-stack"><strong>${escapeHtml(target.primary)}</strong><span>${escapeHtml(target.secondary)}</span></div></td><td><span class="status-pill ${call.success ? "" : "error"}">${call.success ? "Success" : "Error"}</span></td><td>${escapeHtml(new Date(call.started_at_ms).toLocaleString())}</td><td>${call.duration_ms} ms</td><td><button class="button compact" type="button" data-call-id="${call.id}">View</button></td></tr>`;
    }).join("") : '<tr class="empty-row"><td colspan="6">No matching MCP calls.</td></tr>';
    if (focusedCall) $(`[data-call-id="${CSS.escape(focusedCall)}"]`, body)?.focus();
  }
  $("#calls-summary").textContent = `${state.callPage.total || 0} retained call${state.callPage.total === 1 ? "" : "s"}`;
  $("#calls-page-label").textContent = state.callPage.total_pages ? `Page ${state.callPage.page} of ${state.callPage.total_pages}` : "Page 0 of 0";
  $("#calls-prev").disabled = !state.callPage.has_previous;
  $("#calls-next").disabled = !state.callPage.has_next;
}

function titleCase(value) {
  return String(value).replace(/[_-]+/g, " ").replace(/\b\w/g, (char) => char.toUpperCase());
}

function scheduleCallsReload() {
  clearTimeout(state.callsDebounce);
  state.callsDebounce = setTimeout(() => {
    state.callFilters.page = 1;
    loadMcpCalls();
  }, 200);
}

async function openCallDetails(id) {
  const targetId = String(id);
  const requestId = ++state.callDetailRequest;
  state.callDetailId = targetId;
  try {
    const response = await requestJson(`/api/mcp-calls/${encodeURIComponent(id)}`);
    if (requestId !== state.callDetailRequest || state.callDetailId !== targetId || state.view !== "calls") return;
    if (!response.ok) throw new Error(response.message);
    const call = response.data;
    if (String(call.id) !== targetId) throw new Error("Call detail response did not match the requested call");
    $("#call-dialog-title").textContent = `${titleCase(call.operation || "MCP")} request`;
    $("#call-dialog-subtitle").textContent = `${call.tool} - Call #${call.id}`;
    $("#call-request").textContent = JSON.stringify(call.request, null, 2);
    $("#call-response").textContent = JSON.stringify(call.response, null, 2);
    $("#call-dialog").showModal();
  } catch (error) {
    if (requestId === state.callDetailRequest && state.callDetailId === targetId) showToast(error.message || "Unable to load call");
  }
}

function closeCallDetails() {
  state.callDetailRequest += 1;
  state.callDetailId = null;
  if ($("#call-dialog").open) $("#call-dialog").close();
}

function taskToDraft(task) {
  return {
    label: task.label,
    command: task.command,
    args: [...(task.args || [])],
    cwd: task.cwd || ".",
    envRows: Object.entries(task.env || {}).map(([key, value]) => ({ key, value })),
    shell: Boolean(task.shell),
    auto_start: Boolean(task.auto_start),
    stop_timeout_ms: Number(task.stop_timeout_ms || 3000),
    origin: task.origin || { imported: false, has_yaml_override: false },
  };
}

async function openConfig() {
  const session = $("#sessions").value;
  if (!session) return;
  const dialog = $("#config-dialog");
  if (!dialog.open) dialog.showModal();
  await loadConfig(session, false);
}

async function loadConfig(session, confirmDiscard) {
  if (state.configSaving) return;
  if (confirmDiscard && state.configDirty && !confirm("Discard unsaved configuration changes and reload?")) return;
  const requestId = ++state.configRequest;
  showConfigMessage("Loading configuration...");
  try {
    const node = selectedNode();
    const response = await requestJson(`/api/sessions/${encodeURIComponent(session)}/config?${addNodeQuery()}`);
    if (requestId !== state.configRequest || !$("#config-dialog").open || $("#sessions").value !== session || selectedNode() !== node) return;
    if (!response.ok) throw new Error(response.message);
    state.config = response.data;
    state.configSession = session;
    state.configNode = node;
    state.configTasks = (response.data.tasks || []).map(taskToDraft);
    state.configTaskIndex = Math.max(0, state.configTasks.findIndex((task) => task.label === state.currentTask));
    state.configDirty = false;
    renderConfig();
    hideConfigMessage();
  } catch (error) {
    if (requestId === state.configRequest) showConfigMessage(error.message || "Unable to load configuration", true);
  }
}

function requestCloseConfig() {
  const dialog = $("#config-dialog");
  if (!dialog.open) return true;
  if (state.configSaving) {
    showConfigMessage("Wait for the current save to finish.");
    return false;
  }
  if (state.configDirty && !confirm("Discard unsaved configuration changes?")) return false;
  state.configRequest += 1;
  state.configDirty = false;
  dialog.close();
  return true;
}

function renderConfig() {
  if (!state.config) return;
  const node = state.nodes.find((candidate) => candidate.id === state.configNode);
  $("#config-context").innerHTML = `<div><span>Node</span><strong>${escapeHtml(node?.name || state.configNode || "Unknown")}</strong></div><div><span>Session</span><strong>${escapeHtml(state.config.session)}</strong></div><div><span>Project</span><strong title="${escapeAttr(state.config.project)}">${escapeHtml(state.config.project)}</strong></div><div><span>Revision</span><strong>${escapeHtml(String(state.config.revision).slice(0, 12))}</strong></div>`;
  renderConfigTaskList();
  renderConfigForm();
}

function renderConfigTaskList() {
  $("#config-task-list").innerHTML = state.configTasks.length ? state.configTasks.map((task, index) => `<button class="config-task-button ${index === state.configTaskIndex ? "active" : ""}" type="button" data-config-task="${index}"><span>${escapeHtml(task.label || "Untitled task")}</span><small>${task.origin.imported ? "VS Code import" : "YAML task"}</small></button>`).join("") : '<div class="monitor-empty">No tasks</div>';
}

function renderConfigForm() {
  const body = $("#config-form-body");
  const task = state.configTasks[state.configTaskIndex];
  if (!task) {
    body.innerHTML = '<div class="monitor-empty">Add a task to begin.</div>';
    return;
  }
  body.innerHTML = `
    <label class="field"><span>Label</span><input data-field="label" value="${escapeAttr(task.label)}" required></label>
    <label class="field"><span>Command</span><input data-field="command" value="${escapeAttr(task.command)}" required></label>
    <div class="field-grid"><label class="field"><span>Working directory</span><input data-field="cwd" value="${escapeAttr(task.cwd)}"></label><label class="field"><span>Stop timeout (ms)</span><input data-field="stop_timeout_ms" type="number" min="1" max="300000" value="${task.stop_timeout_ms}"></label></div>
    <div class="toggle-row"><label><input data-field="shell" type="checkbox" ${task.shell ? "checked" : ""}> Run through shell</label><label><input data-field="auto_start" type="checkbox" ${task.auto_start ? "checked" : ""}> Auto start</label></div>
    <div class="origin-note">${task.origin.imported ? "Imported from .vscode/tasks.json; Taskdeck saves only overrides." : "Defined in taskdeck.yaml."}</div>
    <fieldset class="field"><legend>Arguments</legend><div class="repeater" id="args-rows">${task.args.map((arg, index) => `<div class="repeater-row"><input data-arg="${index}" value="${escapeAttr(arg)}" aria-label="Argument ${index + 1}"><button class="icon-button" type="button" data-remove-arg="${index}" aria-label="Remove argument" title="Remove">&#215;</button></div>`).join("")}</div><button class="button compact" type="button" data-add-arg>Add argument</button></fieldset>
    <fieldset class="field"><legend>Environment</legend><div class="repeater" id="env-rows">${task.envRows.map((row, index) => `<div class="repeater-row env"><input data-env-key="${index}" value="${escapeAttr(row.key)}" placeholder="NAME" aria-label="Environment key"><input data-env-value="${index}" value="${escapeAttr(row.value)}" placeholder="Value" aria-label="Environment value"><button class="icon-button" type="button" data-remove-env="${index}" aria-label="Remove environment variable" title="Remove">&#215;</button></div>`).join("")}</div><button class="button compact" type="button" data-add-env>Add variable</button></fieldset>
    <button class="button danger" type="button" data-delete-task>Delete task</button>`;
}

function showConfigMessage(message, error = false, reload = false) {
  const element = $("#config-message");
  element.hidden = false;
  element.textContent = message;
  element.classList.toggle("error", error);
  $("#reload-config").hidden = !reload;
}

function hideConfigMessage() {
  $("#config-message").hidden = true;
  $("#reload-config").hidden = true;
}

function addConfigTask() {
  state.configTasks.push({ label: "new-task", command: "", args: [], cwd: ".", envRows: [], shell: true, auto_start: false, stop_timeout_ms: 3000, origin: { imported: false, has_yaml_override: false } });
  state.configTaskIndex = state.configTasks.length - 1;
  state.configDirty = true;
  renderConfig();
}

function validateConfigTasks() {
  const labels = new Set();
  return state.configTasks.map((task, index) => {
    const label = task.label.trim();
    const command = task.command.trim();
    if (!label) throw new Error(`Task ${index + 1}: label is required`);
    if (labels.has(label)) throw new Error(`Duplicate task label: ${label}`);
    labels.add(label);
    if (!command) throw new Error(`${label}: command is required`);
    const timeout = Number(task.stop_timeout_ms);
    if (!Number.isInteger(timeout) || timeout < 1 || timeout > 300000) throw new Error(`${label}: stop timeout must be 1-300000 ms`);
    const env = {};
    task.envRows.forEach((row) => {
      const key = row.key.trim();
      if (!key) throw new Error(`${label}: environment key is required`);
      if (Object.hasOwn(env, key)) throw new Error(`${label}: duplicate environment key ${key}`);
      env[key] = row.value;
    });
    return { label, command, args: [...task.args], cwd: task.cwd.trim() || ".", env, shell: task.shell, auto_start: task.auto_start, stop_timeout_ms: timeout };
  });
}

async function saveConfig() {
  if (!state.config || state.configSaving) return;
  let tasks;
  try { tasks = validateConfigTasks(); } catch (error) {
    showConfigMessage(error.message, true);
    return;
  }
  state.configSaving = true;
  $("#save-config").disabled = true;
  hideConfigMessage();
  try {
    const session = state.configSession;
    if (!session || $("#sessions").value !== session || selectedNode() !== state.configNode) throw new Error("The selected node or session changed. Reopen configuration before saving.");
    const response = await requestJson(`/api/sessions/${encodeURIComponent(session)}/config?${addNodeQuery()}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ revision: state.config.revision, tasks }),
    });
    if (!response.ok) {
      const kind = response.data?.kind;
      if (kind === "stale_revision") {
        showConfigMessage("The file changed outside Taskdeck. Reload before applying again.", true, true);
        return;
      }
      if (kind === "reconciliation_error" && response.data?.saved) {
        state.config.revision = response.data.current_revision;
        state.configDirty = false;
        showConfigMessage("Saved to taskdeck.yaml, but one or more live sessions could not reconcile. Retry after checking the affected tasks.", true, true);
        await loadSnapshot();
        return;
      }
      throw new Error(response.message);
    }
    state.config = response.data;
    state.configTasks = (response.data.tasks || []).map(taskToDraft);
    state.configDirty = false;
    showToast("Configuration applied");
    $("#config-dialog").close();
    state.renderedTask = null;
    await loadSnapshot();
  } catch (error) {
    showConfigMessage(error.message || "Unable to save configuration", true);
  } finally {
    state.configSaving = false;
    $("#save-config").disabled = false;
  }
}

function bindEvents() {
  $("#sidebar-toggle").addEventListener("click", () => setSidebar(!$("#app-shell").classList.contains("sidebar-collapsed")));
  $("#theme").addEventListener("click", setTheme);
  $("#sidebar").addEventListener("click", (event) => {
    const button = event.target.closest("[data-view]");
    if (button) setView(button.dataset.view);
  });
  $("#nodes").addEventListener("change", () => {
    if ($("#config-dialog").open && !requestCloseConfig()) {
      $("#nodes").value = state.configNode || state.snapshotNode || "";
      return;
    }
    clearWorkspace();
    state.sessions = [];
    state.sessionsSignature = "";
    $("#sessions").innerHTML = '<option value="">Loading sessions</option>';
    loadSessions();
  });
  $("#sessions").addEventListener("change", () => {
    if ($("#config-dialog").open && !requestCloseConfig()) {
      $("#sessions").value = state.configSession || state.snapshot?.name || "";
      return;
    }
    state.snapshotRequest += 1;
    state.metricsRequest += 1;
    state.logsRequest += 1;
    state.snapshot = null;
    state.currentTask = null;
    state.renderedTask = null;
    state.metrics = null;
    state.tabsSignature = "";
    state.headerSignature = "";
    resetLogCursor();
    $("#tabs").innerHTML = "";
    $("#task-pane").innerHTML = '<div class="empty-state"><div><h1>Loading workspace</h1></div></div>';
    loadSnapshot();
  });
  $("#tabs").addEventListener("click", (event) => {
    const button = event.target.closest("[data-task]");
    if (!button) return;
    state.currentTask = button.dataset.task;
    state.renderedTask = null;
    state.metrics = null;
    state.headerSignature = "";
    resetLogCursor();
    renderTabs(Object.keys(state.snapshot.tasks));
    renderTask();
    loadLogs();
    loadMetrics();
  });
  $("#tabs").addEventListener("keydown", (event) => {
    if (!["ArrowLeft", "ArrowRight"].includes(event.key)) return;
    const tabs = $$("[data-task]", $("#tabs"));
    const index = tabs.indexOf(document.activeElement);
    if (index < 0 || !tabs.length) return;
    event.preventDefault();
    tabs[(index + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length].click();
    $("[data-task].active", $("#tabs"))?.focus();
  });
  $("#task-pane").addEventListener("click", (event) => {
    const action = event.target.closest("[data-action]");
    if (action) act(action.dataset.action, action);
    const logAction = event.target.closest("[data-log]");
    if (logAction) handleLogAction(logAction.dataset.log);
    const mode = event.target.closest("[data-mode]");
    if (mode) setWorkspaceMode(mode.dataset.mode);
    if (event.target.closest("[data-config]")) openConfig();
  });
  $("#refresh-calls").addEventListener("click", loadMcpCalls);
  $("#calls-body").addEventListener("click", (event) => {
    const button = event.target.closest("[data-call-id]");
    if (button) openCallDetails(button.dataset.callId);
  });
  $("#calls-search").addEventListener("input", (event) => { state.callFilters.q = event.target.value.trim(); scheduleCallsReload(); });
  $("#calls-operation").addEventListener("change", (event) => { state.callFilters.operation = event.target.value; scheduleCallsReload(); });
  $("#calls-status").addEventListener("change", (event) => { state.callFilters.status = event.target.value; scheduleCallsReload(); });
  $("#calls-session").addEventListener("input", (event) => { state.callFilters.session = event.target.value.trim(); scheduleCallsReload(); });
  $("#calls-task").addEventListener("input", (event) => { state.callFilters.task = event.target.value.trim(); scheduleCallsReload(); });
  $("#calls-page-size").addEventListener("change", (event) => { state.callFilters.pageSize = Number(event.target.value); scheduleCallsReload(); });
  $("#clear-call-filters").addEventListener("click", () => {
    state.callFilters = { q: "", operation: "", status: "all", session: "", task: "", page: 1, pageSize: Number($("#calls-page-size").value) || 20 };
    $("#calls-search").value = ""; $("#calls-operation").value = ""; $("#calls-status").value = "all"; $("#calls-session").value = ""; $("#calls-task").value = "";
    loadMcpCalls();
  });
  $("#calls-prev").addEventListener("click", () => { if (state.callPage.has_previous) { state.callFilters.page -= 1; loadMcpCalls(); } });
  $("#calls-next").addEventListener("click", () => { if (state.callPage.has_next) { state.callFilters.page += 1; loadMcpCalls(); } });
  $("#close-call-dialog").addEventListener("click", closeCallDetails);
  $("#close-config").addEventListener("click", requestCloseConfig);
  $("#call-dialog").addEventListener("click", (event) => { if (event.target === $("#call-dialog")) closeCallDetails(); });
  $("#call-dialog").addEventListener("close", () => { state.callDetailRequest += 1; state.callDetailId = null; });
  $("#config-dialog").addEventListener("click", (event) => { if (event.target === $("#config-dialog")) requestCloseConfig(); });
  $("#config-dialog").addEventListener("cancel", (event) => { event.preventDefault(); requestCloseConfig(); });
  $("#add-config-task").addEventListener("click", addConfigTask);
  $("#reload-config").addEventListener("click", () => loadConfig(state.configSession || $("#sessions").value, true));
  $("#config-task-list").addEventListener("click", (event) => {
    const button = event.target.closest("[data-config-task]");
    if (!button) return;
    state.configTaskIndex = Number(button.dataset.configTask);
    renderConfigTaskList();
    renderConfigForm();
  });
  $("#config-form").addEventListener("submit", (event) => { event.preventDefault(); saveConfig(); });
  $("#config-form-body").addEventListener("input", handleConfigInput);
  $("#config-form-body").addEventListener("change", handleConfigInput);
  $("#config-form-body").addEventListener("click", handleConfigButton);
  $("#docs-view").addEventListener("click", async (event) => {
    const button = event.target.closest("[data-copy] .copy");
    if (!button) return;
    try {
      await navigator.clipboard.writeText($("code", button.closest("[data-copy]")).textContent);
      showToast("Copied to clipboard");
    } catch (_) {
      showToast("Clipboard access unavailable");
    }
  });
  document.addEventListener("fullscreenchange", () => { if (!document.fullscreenElement) $("#log-panel")?.classList.remove("fallback-fullscreen"); updateFullscreenButton(); });
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && $("#log-panel")?.classList.contains("fallback-fullscreen")) {
      event.preventDefault();
      $("#log-panel").classList.remove("fallback-fullscreen");
      updateFullscreenButton();
    }
  });
  addEventListener("resize", applyWorkspaceMode);
}

function handleConfigInput(event) {
  const task = state.configTasks[state.configTaskIndex];
  if (!task) return;
  const target = event.target;
  if (target.dataset.field) {
    const field = target.dataset.field;
    task[field] = target.type === "checkbox" ? target.checked : field === "stop_timeout_ms" ? Number(target.value) : target.value;
    if (field === "label") renderConfigTaskList();
  }
  if (target.dataset.arg != null) task.args[Number(target.dataset.arg)] = target.value;
  if (target.dataset.envKey != null) task.envRows[Number(target.dataset.envKey)].key = target.value;
  if (target.dataset.envValue != null) task.envRows[Number(target.dataset.envValue)].value = target.value;
  state.configDirty = true;
}

function handleConfigButton(event) {
  const task = state.configTasks[state.configTaskIndex];
  if (!task) return;
  const button = event.target.closest("button");
  if (!button) return;
  if (button.dataset.addArg != null) { task.args.push(""); renderConfigForm(); }
  if (button.dataset.removeArg != null) { task.args.splice(Number(button.dataset.removeArg), 1); renderConfigForm(); }
  if (button.dataset.addEnv != null) { task.envRows.push({ key: "", value: "" }); renderConfigForm(); }
  if (button.dataset.removeEnv != null) { task.envRows.splice(Number(button.dataset.removeEnv), 1); renderConfigForm(); }
  if (button.dataset.deleteTask != null) {
    state.configTasks.splice(state.configTaskIndex, 1);
    state.configTaskIndex = Math.max(0, Math.min(state.configTaskIndex, state.configTasks.length - 1));
    renderConfig();
  }
  state.configDirty = true;
}

function updateEndpoint() {
  const endpoint = `${location.origin}/mcp`;
  $("#endpoint-label").textContent = endpoint.replace(/^https?:\/\//, "");
  $("#client-config").textContent = JSON.stringify({ mcpServers: { taskdeck: { type: "http", url: endpoint } } }, null, 2);
}

function tick() {
  if (state.view === "tasks") {
    if (selectedNodeState()?.online === false) return;
    loadSnapshot();
    loadLogs();
    loadMetrics();
  } else if (state.view === "calls") {
    loadMcpCalls();
  }
}

applySavedPreferences();
bindEvents();
updateEndpoint();
setView("tasks");
loadNodes();
setInterval(tick, 1000);
setInterval(loadNodes, 5000);
