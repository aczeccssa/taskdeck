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
  workspaces: [],
  workspacesRequest: 0,
  workflowGroups: [],
  workflowTargets: [],
  workflowUngrouped: [],
  workflowRequest: 0,
  workflowEditingId: null,
  workflowEditorActive: false,
  workflowDraftMembers: [],
  workflowLastResults: null,
  boards: [],
  boardTargets: [],
  boardRequest: 0,
  boardsSignature: "",
  boardEditingId: null,
  boardEditorActive: false,
  boardDraftCards: [],
  boardSnapshots: {},
  boardCardData: {},
  boardLiveBusy: false,
  nodeSettings: null,
  nodeSettingsRequest: 0,
  serviceStatus: null,
  snapshotRequest: 0,
  metricsRequest: 0,
  logsRequest: 0,
  callsRequest: 0,
  callDetailRequest: 0,
  callDetailId: null,
  callDetailMode: "result",
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
  auditRequest: 0,
  auditDetailRequest: 0,
  auditDetailId: null,
  auditDetailMode: "summary",
  audits: [],
  auditSignature: "",
  auditPage: { page: 1, page_size: 20, total: 0, total_pages: 0, has_next: false, has_previous: false },
  auditFilters: { q: "", source: "all", status: "all", node: "", session: "", task: "", operation: "", page: 1, pageSize: 20 },
  auditDebounce: null,
  config: null,
  configSession: null,
  configNode: null,
  configTasks: [],
  configWorkspaceEnvRows: [],
  runs: [],
  runPage: { page: 1, page_size: 20, total: 0, total_pages: 0 },
  runFilters: { session:"",task:"",status:"all",trigger:"",page:1,pageSize:20 },
  events: [],
  eventPage: { page: 1, page_size: 20, total: 0, total_pages: 0 },

  configSaving: false,
  configDirty: false,
  tabOrderSaving: false,
  suppressTabClick: false,
  seenExits: loadSeenExits(),
  toastTimer: null,
};

const icons = {
  play: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m8 5 11 7-11 7z"/></svg>',
  pause: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5v14M16 5v14"/></svg>',
  restart: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M19 8V4m0 0h-4m4 0-3 3a7 7 0 1 0 2 8"/></svg>',
  stop: '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>',
  settings: '<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M19 12a7 7 0 0 0-.1-1l2-1.5-2-3.4-2.4 1A7 7 0 0 0 15 6l-.3-2.6h-4L10.4 6A7 7 0 0 0 9 7.1l-2.4-1-2 3.4 2 1.5a7 7 0 0 0 0 2l-2 1.5 2 3.4 2.4-1A7 7 0 0 0 10.4 18l.3 2.6h4L15 18a7 7 0 0 0 1.5-1.1l2.4 1 2-3.4-2-1.5a7 7 0 0 0 .1-1z"/></svg>',
  trash: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 7h16M9 7V4h6v3m3 0-1 13H7L6 7m4 4v5m4-5v5"/></svg>',
  grip: '<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="9" cy="7" r="1"/><circle cx="15" cy="7" r="1"/><circle cx="9" cy="12" r="1"/><circle cx="15" cy="12" r="1"/><circle cx="9" cy="17" r="1"/><circle cx="15" cy="17" r="1"/></svg>',
  pin: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M9 3h6v6l2 3v2h-4v7l-1 1-1-1v-7H7v-2l2-3z"/></svg>',
};

function loadSeenExits() {
  try {
    const value = JSON.parse(localStorage.getItem("taskdeck-seen-exits") || "{}");
    return value && typeof value === "object" ? value : {};
  } catch (_) {
    return {};
  }
}

function exitKey(task) {
  return `${selectedNode()}\u0000${state.snapshot?.name || ""}\u0000${task}`;
}

function markExitSeen(task) {
  const snapshot = state.snapshot?.tasks?.[task];
  if (!snapshot || snapshot.status !== "exited") return;
  state.seenExits[exitKey(task)] = Number(snapshot.run_generation || 0);
  const keys = Object.keys(state.seenExits);
  keys.slice(0, Math.max(0, keys.length - 500)).forEach((key) => delete state.seenExits[key]);
  localStorage.setItem("taskdeck-seen-exits", JSON.stringify(state.seenExits));
}

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
  if (response.status === 401) { location.assign("/login"); throw new Error("Authentication required"); }
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
  $("#nodes").hidden = false;
  $("#page-title").textContent = view === "calls" ? "MCP Calls" : view === "audit" ? "Audit Log" : view === "runs" ? "Task Runs" : view === "docs" ? "MCP Guide" : view === "settings" ? "Settings" : view === "workflows" ? "Workflows" : view === "boards" ? "Boards" : "Task workspace";
  if (view === "calls") loadMcpCalls();
  if (view === "audit") loadAudit();
  if (view === "runs") loadRuns().catch((error) => showToast(error.message || "Unable to load runs"));
  if (view === "workflows") loadWorkflowGroups().catch((error) => showToast(error.message || "Unable to load workflows"));
  if (view === "boards") loadBoards().catch((error) => showToast(error.message || "Unable to load boards"));
  if (view === "settings") {
    loadNodeSettings().catch((error) => showToast(error.message || "Unable to load settings"));
    loadServiceStatus();
    renderAliasForm();
  }
  updateMeta();
}

function selectedNode() {
  return $("#nodes").value;
}

function selectedNodeState() {
  return state.nodes.find((node) => node.id === selectedNode()) || null;
}

function workspaceFor(session) {
  return state.workspaces.find((workspace) => workspace.session === session) || null;
}

function sessionOptionLabel(session) {
  const workspace = workspaceFor(session);
  return workspace?.alias ? `${workspace.alias} · ${session}` : session;
}

function renderSessionOptions(sessions, emptyLabel = "No sessions") {
  const select = $("#sessions");
  return sessions.length
    ? sessions.map((session) => `<option value="${escapeAttr(session)}">${escapeHtml(sessionOptionLabel(session))}</option>`).join("")
    : `<option value="">${escapeHtml(emptyLabel)}</option>`;
}

async function loadWorkspaces() {
  const requestId = ++state.workspacesRequest;
  const node = selectedNode();
  if (!node) return;
  try {
    const response = await requestJson(`/api/workspaces?${addNodeQuery()}`);
    if (requestId !== state.workspacesRequest || selectedNode() !== node) return;
    if (!response.ok) throw new Error(response.message);
    state.workspaces = response.data || [];
    const sessions = state.workspaces.map((workspace) => workspace.session);
    if (!$("#sessions").options.length || [...$("#sessions").options].some((option) => !sessions.includes(option.value))) {
      const selected = $("#sessions").value;
      $("#sessions").innerHTML = renderSessionOptions(sessions, "No cached sessions");
      if (sessions.includes(selected)) $("#sessions").value = selected;
      else if (sessions.length) $("#sessions").value = sessions[0];
    } else {
      [...$("#sessions").options].forEach((option) => { option.textContent = sessionOptionLabel(option.value); });
    }
    renderAliasForm();
    setConnection(true);
  } catch (error) {
    if (requestId === state.workspacesRequest) console.warn("Unable to load workspace aliases", error);
  }
}

async function loadWorkflowGroups() {
  const requestId = ++state.workflowRequest;
  try {
    const response = await requestJson("/api/workflow-groups");
    if (requestId !== state.workflowRequest) return;
    if (!response.ok) throw new Error(response.message);
    state.workflowGroups = response.data?.groups || [];
    state.workflowTargets = response.data?.targets || [];
    state.workflowUngrouped = response.data?.ungrouped || [];
    renderWorkflows();
    setConnection(true);
    updateMeta();
  } catch (error) {
    if (requestId !== state.workflowRequest) return;
    state.workflowGroups = [];
    state.workflowTargets = [];
    state.workflowUngrouped = [];
    const groups = $("#workflow-groups");
    if (groups) groups.innerHTML = `<div class="empty-state compact"><div><h1>Workflows unavailable</h1><p>${escapeHtml(error.message || "Leader workflow groups are unavailable")}</p></div></div>`;
    const summary = $("#workflows-summary");
    if (summary) summary.textContent = "Leader-only workflow grouping";
    renderWorkflowEditor();
  }
}

function renderWorkflows() {
  const groups = $("#workflow-groups");
  const summary = $("#workflows-summary");
  if (!groups || !summary) return;
  const memberCount = state.workflowGroups.reduce((total, group) => total + (group.members?.length || 0), 0);
  summary.textContent = `${state.workflowGroups.length} groups · ${memberCount} members · ${state.workflowUngrouped.length} ungrouped workspaces`;
  groups.innerHTML = state.workflowGroups.length
    ? state.workflowGroups.map(renderWorkflowCard).join("")
    : '<div class="empty-state compact"><div><h1>No workflow groups</h1><p>Create a group to organize tasks across workspaces and nodes.</p></div></div>';
  const ungrouped = $("#ungrouped-workspaces");
  if (ungrouped) {
    ungrouped.innerHTML = state.workflowUngrouped.length
      ? state.workflowUngrouped.map((target) => `<button class="workflow-target" type="button" data-workflow-open data-node="${escapeAttr(target.node_id)}" data-session="${escapeAttr(target.session)}"><strong>${escapeHtml(target.workspace_display_name)}</strong><span>${escapeHtml(target.node_name)} · ${escapeHtml(target.session)} · ${target.tasks?.length || 0} task${(target.tasks?.length || 0) === 1 ? "" : "s"}</span></button>`).join("")
      : '<div class="muted">Every visible workspace is assigned to a workflow group.</div>';
  }
  renderWorkflowEditor();
}

function renderWorkflowCard(group) {
  const members = group.members?.length
    ? group.members.map((member) => `<button class="workflow-member ${member.available ? "" : "unavailable"}" type="button" data-workflow-open data-node="${escapeAttr(member.node_id)}" data-session="${escapeAttr(member.session)}"><span class="status-pill ${member.available ? "" : "error"}">${escapeHtml(member.available ? "ready" : member.skip_reason || "skipped")}</span><strong>${escapeHtml(member.workspace_display_name)}</strong><span>${escapeHtml(member.node_name || member.node_id)} · ${escapeHtml(member.session)} · ${escapeHtml(member.task)}</span></button>`).join("")
    : '<div class="muted">No members yet.</div>';
  return `<article class="workflow-card" data-workflow-id="${escapeAttr(group.id)}">
    <header><div><h2>${escapeHtml(group.name)}</h2><p>${group.members?.length || 0} member${(group.members?.length || 0) === 1 ? "" : "s"}</p></div><div class="workflow-card-actions"><button class="button compact" type="button" data-workflow-edit="${escapeAttr(group.id)}">Edit</button><button class="button compact danger" type="button" data-workflow-delete="${escapeAttr(group.id)}">Delete</button></div></header>
    <div class="workflow-members">${members}</div>
    <footer class="workflow-actions">${["start", "stop", "restart", "pause", "resume"].map((action) => `<button class="button compact" type="button" data-workflow-action="${action}" data-workflow-id="${escapeAttr(group.id)}">${action}</button>`).join("")}</footer>
  </article>`;
}

function beginWorkflowEditor(group = null) {
  state.workflowEditorActive = true;
  state.workflowEditingId = group?.id || null;
  state.workflowDraftMembers = (group?.members || []).map((member) => ({
    node_id: member.node_id,
    session: member.session,
    task: member.task,
  }));
  $("#workflow-name").value = group?.name || "";
  $("#workflow-results").innerHTML = "";
  renderWorkflowEditor();
}

function cancelWorkflowEditor() {
  state.workflowEditorActive = false;
  state.workflowEditingId = null;
  state.workflowDraftMembers = [];
  $("#workflow-name").value = "";
  $("#workflow-results").innerHTML = "";
  renderWorkflowEditor();
}

function workflowTargetForMember(member) {
  return state.workflowTargets.find((target) => target.node_id === member.node_id && target.session === member.session) || null;
}

function renderWorkflowEditor() {
  const members = $("#workflow-members");
  if (!members) return;
  $("#workflow-editor-title").textContent = state.workflowEditingId ? "Edit workflow" : "New workflow";
  $("#save-workflow").disabled = !state.workflowEditorActive;
  $("#add-workflow-member").disabled = !state.workflowEditorActive;
  $("#cancel-workflow").disabled = !state.workflowEditorActive;
  if (!state.workflowEditorActive) {
    members.innerHTML = '<div class="muted">Select a workflow to edit, or create a new one.</div>';
    showWorkflowMessage("", "");
    return;
  }
  members.innerHTML = state.workflowDraftMembers.length
    ? state.workflowDraftMembers.map(renderWorkflowMemberEditor).join("")
    : '<div class="muted">Add at least one workspace task member.</div>';
}

function renderWorkflowMemberEditor(member, index) {
  const targetIndex = state.workflowTargets.findIndex((target) => target.node_id === member.node_id && target.session === member.session);
  const missingOption = targetIndex < 0 && member.node_id && member.session
    ? `<option value="-1" selected>${escapeHtml(member.node_id)} / ${escapeHtml(member.session)} (cached or missing)</option>`
    : "";
  const targetOptions = state.workflowTargets.map((target, optionIndex) => `<option value="${optionIndex}" ${optionIndex === targetIndex ? "selected" : ""}>${escapeHtml(target.node_name)} / ${escapeHtml(target.workspace_display_name)} (${escapeHtml(target.session)})</option>`).join("");
  const target = targetIndex >= 0 ? state.workflowTargets[targetIndex] : workflowTargetForMember(member);
  const taskOptions = uniqueStrings([...(target?.tasks || []), member.task].filter(Boolean));
  return `<div class="workflow-member-editor" data-workflow-member-row="${index}">
    <select data-workflow-target="${index}" aria-label="Workflow member workspace">${missingOption}${targetOptions}</select>
    <select data-workflow-task="${index}" aria-label="Workflow member task">${taskOptions.length ? taskOptions.map((task) => `<option value="${escapeAttr(task)}" ${task === member.task ? "selected" : ""}>${escapeHtml(task)}</option>`).join("") : '<option value="">No tasks</option>'}</select>
    <button class="icon-button" type="button" data-workflow-up="${index}" aria-label="Move member up" title="Move up">↑</button>
    <button class="icon-button" type="button" data-workflow-down="${index}" aria-label="Move member down" title="Move down">↓</button>
    <button class="icon-button" type="button" data-workflow-remove="${index}" aria-label="Remove member" title="Remove">×</button>
  </div>`;
}

function uniqueStrings(values) {
  return [...new Set(values)];
}

function addWorkflowMember() {
  const target = state.workflowTargets.find((candidate) => candidate.tasks?.length) || state.workflowTargets[0];
  if (!target) {
    showWorkflowMessage("No visible workspace targets are available.", "error");
    return;
  }
  state.workflowEditorActive = true;
  state.workflowDraftMembers.push({ node_id: target.node_id, session: target.session, task: target.tasks?.[0] || "" });
  renderWorkflowEditor();
}

function handleWorkflowMemberChange(event) {
  const targetSelect = event.target.closest("[data-workflow-target]");
  const taskSelect = event.target.closest("[data-workflow-task]");
  if (targetSelect) {
    const index = Number(targetSelect.dataset.workflowTarget);
    const target = state.workflowTargets[Number(targetSelect.value)];
    if (target && state.workflowDraftMembers[index]) {
      state.workflowDraftMembers[index].node_id = target.node_id;
      state.workflowDraftMembers[index].session = target.session;
      if (!target.tasks?.includes(state.workflowDraftMembers[index].task)) state.workflowDraftMembers[index].task = target.tasks?.[0] || "";
      renderWorkflowEditor();
    }
  } else if (taskSelect) {
    const index = Number(taskSelect.dataset.workflowTask);
    if (state.workflowDraftMembers[index]) state.workflowDraftMembers[index].task = taskSelect.value;
  }
}

function handleWorkflowMemberButton(event) {
  const button = event.target.closest("button");
  if (!button) return;
  const remove = button.dataset.workflowRemove;
  const up = button.dataset.workflowUp;
  const down = button.dataset.workflowDown;
  if (remove != null) state.workflowDraftMembers.splice(Number(remove), 1);
  if (up != null && Number(up) > 0) {
    const index = Number(up);
    [state.workflowDraftMembers[index - 1], state.workflowDraftMembers[index]] = [state.workflowDraftMembers[index], state.workflowDraftMembers[index - 1]];
  }
  if (down != null && Number(down) < state.workflowDraftMembers.length - 1) {
    const index = Number(down);
    [state.workflowDraftMembers[index + 1], state.workflowDraftMembers[index]] = [state.workflowDraftMembers[index], state.workflowDraftMembers[index + 1]];
  }
  renderWorkflowEditor();
}

function showWorkflowMessage(message, type = "") {
  const element = $("#workflow-message");
  if (!element) return;
  element.textContent = message || "";
  element.classList.remove("error", "success", "warning");
  if (type) element.classList.add(type);
}

async function saveWorkflow() {
  if (!state.workflowEditorActive) return;
  const name = $("#workflow-name").value.trim();
  if (!name) {
    showWorkflowMessage("Workflow name is required.", "error");
    return;
  }
  if (state.workflowDraftMembers.some((member) => !member.node_id || !member.session || !member.task)) {
    showWorkflowMessage("Every member needs a workspace and task.", "error");
    return;
  }
  const body = { name, members: state.workflowDraftMembers };
  const url = state.workflowEditingId ? `/api/workflow-groups/${encodeURIComponent(state.workflowEditingId)}` : "/api/workflow-groups";
  const method = state.workflowEditingId ? "PUT" : "POST";
  showWorkflowMessage("Saving workflow…");
  try {
    const response = await requestJson(url, { method, headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    if (!response.ok) throw new Error(response.message);
    state.workflowEditorActive = false;
    state.workflowEditingId = null;
    state.workflowDraftMembers = [];
    $("#workflow-name").value = "";
    showWorkflowMessage("Workflow saved.", "success");
    await loadWorkflowGroups();
  } catch (error) {
    showWorkflowMessage(error.message || "Unable to save workflow", "error");
  }
}

async function deleteWorkflow(id) {
  const group = state.workflowGroups.find((item) => item.id === id);
  if (!group || !confirm(`Delete workflow '${group.name}'?`)) return;
  try {
    const response = await requestJson(`/api/workflow-groups/${encodeURIComponent(id)}`, { method: "DELETE" });
    if (!response.ok) throw new Error(response.message);
    cancelWorkflowEditor();
    await loadWorkflowGroups();
    showToast("Workflow deleted");
  } catch (error) {
    showWorkflowMessage(error.message || "Unable to delete workflow", "error");
  }
}

async function runWorkflowAction(id, action) {
  const group = state.workflowGroups.find((item) => item.id === id);
  if (!group || !confirm(`${action} ${group.members?.length || 0} workflow member${(group.members?.length || 0) === 1 ? "" : "s"}?`)) return;
  try {
    const response = await requestJson(`/api/workflow-groups/${encodeURIComponent(id)}/actions`, {
      method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ action }),
    });
    if (!response.ok) throw new Error(response.message);
    state.workflowLastResults = response.data;
    renderWorkflowResults(response.data);
    await loadWorkflowGroups();
  } catch (error) {
    showWorkflowMessage(error.message || "Workflow action failed", "error");
  }
}

function renderWorkflowResults(summary) {
  const target = $("#workflow-results");
  if (!target || !summary) return;
  target.innerHTML = `<h3>${escapeHtml(summary.group_name)} · ${escapeHtml(summary.action)}</h3><p>${summary.success_count} succeeded · ${summary.failed_count} failed · ${summary.skipped_count} skipped</p><div class="workflow-result-list">${(summary.results || []).map((item) => `<div class="workflow-result ${escapeAttr(item.status)}"><strong>${escapeHtml(item.workspace_display_name)} / ${escapeHtml(item.task)}</strong><span>${escapeHtml(item.status)} · ${escapeHtml(item.message)}</span></div>`).join("")}</div>`;
}

async function openWorkflowMember(node, session) {
  const nodeSelect = $("#nodes");
  if ([...nodeSelect.options].some((option) => option.value === node)) nodeSelect.value = node;
  setView("tasks");
  await loadWorkspaces();
  await loadSessions();
  const sessionSelect = $("#sessions");
  if ([...sessionSelect.options].some((option) => option.value === session)) {
    sessionSelect.value = session;
    state.snapshot = null;
    state.snapshotNode = null;
    await loadSnapshot();
  }
}

function boardCardKey(card) {
  return `${card.node_id}\u0000${card.session}\u0000${card.task}`;
}

function boardPinnedCards() {
  const seen = new Set();
  const cards = [];
  state.boards.forEach((board) => {
    board.cards.forEach((card) => {
      if (!card.pinned) return;
      const key = boardCardKey(card);
      if (seen.has(key)) return;
      seen.add(key);
      cards.push({ ...card, key });
    });
  });
  return cards;
}

function boardLiveCards() {
  const cards = [];
  state.boards.forEach((board) => {
    board.cards.forEach((card) => cards.push({ ...card, key: boardCardKey(card) }));
  });
  return cards;
}

async function loadBoards() {
  const requestId = ++state.boardRequest;
  try {
    const response = await requestJson("/api/boards");
    if (requestId !== state.boardRequest) return;
    if (!response.ok) throw new Error(response.message);
    state.boards = response.data?.boards || [];
    state.boardTargets = response.data?.targets || [];
    renderBoards();
    setConnection(true);
    updateMeta();
    await refreshBoardLive();
  } catch (error) {
    if (requestId !== state.boardRequest) return;
    state.boards = [];
    state.boardTargets = [];
    const list = $("#board-list");
    if (list) list.innerHTML = `<div class="empty-state compact"><div><h1>Boards unavailable</h1><p>${escapeHtml(error.message || "Leader boards are unavailable")}</p></div></div>`;
    const summary = $("#boards-summary");
    if (summary) summary.textContent = "Leader-only boards";
    renderBoardEditor();
  }
}

function renderBoards() {
  const signature = JSON.stringify([state.boards, state.boardTargets, state.nodes]);
  if (signature === state.boardsSignature) return;
  state.boardsSignature = signature;
  const cardCount = state.boards.reduce((total, board) => total + (board.cards?.length || 0), 0);
  const summary = $("#boards-summary");
  if (summary) summary.textContent = `${state.boards.length} boards · ${cardCount} cards · ${boardPinnedCards().length} pinned`;
  renderBoardOverview();
  renderBoardPinned();
  renderBoardList();
  renderBoardEditor();
}

function renderBoardOverview() {
  const nodes = $("#board-nodes");
  if (nodes) {
    nodes.innerHTML = state.nodes.length
      ? state.nodes.map((node) => `<div class="board-node ${node.online ? "" : "offline"}"><div class="board-node-heading"><span class="status-pill ${node.online ? "" : "error"}">${node.online ? "online" : "offline"}</span><strong>${escapeHtml(node.is_self ? `This device · ${node.name}` : node.name)}</strong></div><span>${escapeHtml(node.role || "node")}${node.mode ? ` · ${escapeHtml(node.mode)}` : ""} · ${node.sessions?.length || 0} workspace${(node.sessions?.length || 0) === 1 ? "" : "s"}</span></div>`).join("")
      : '<div class="muted">No nodes known.</div>';
  }
  const workspaces = $("#board-workspaces");
  if (workspaces) {
    workspaces.innerHTML = state.boardTargets.length
      ? state.boardTargets.map((target) => `<button class="workflow-target" type="button" data-board-open data-node="${escapeAttr(target.node_id)}" data-session="${escapeAttr(target.session)}"><strong>${escapeHtml(target.workspace_display_name)}</strong><span>${escapeHtml(target.node_name)} · ${escapeHtml(target.session)} · ${target.tasks?.length || 0} task${(target.tasks?.length || 0) === 1 ? "" : "s"}</span></button>`).join("")
      : '<div class="muted">No workspaces registered.</div>';
  }
}

function boardCardStatus(card) {
  const snapshot = state.boardSnapshots[`${card.node_id}\u0000${card.session}`];
  if (!snapshot || snapshot.error) return { status: "unknown", dot: "failed", label: snapshot?.error || "Unavailable" };
  const task = snapshot.tasks?.[card.task];
  if (!task) return { status: "unknown", dot: "failed", label: "Task not found" };
  const status = task.status || "unknown";
  const dot = status === "running" ? "running" : status === "failed" ? "failed" : status === "exited" ? "exited" : status === "paused" ? "suspected" : "";
  return { status, dot, label: status, task };
}

function boardCardMarkup(card, { compact = false } = {}) {
  const key = card.key || boardCardKey(card);
  if (compact) {
    return `<article class="board-card compact" data-board-open data-node="${escapeAttr(card.node_id)}" data-session="${escapeAttr(card.session)}" data-pin-tile="${escapeAttr(key)}">
      <div class="board-card-title"><span class="task-state-dot board-dot" data-board-dot aria-hidden="true"></span><div><strong>${escapeHtml(card.task)}</strong><span>${escapeHtml(card.workspace_display_name || card.session)} · ${escapeHtml(card.node_name || card.node_id)}</span></div></div>
      <span class="status-pill" data-pin-status>Loading</span>
    </article>`;
  }
  const modes = `<div class="board-card-modes" role="group" aria-label="Card view">${["status", "logs", "metrics"].map((mode) => `<button type="button" data-board-mode="${mode}" data-board-card="${escapeAttr(card.id)}" class="${card.mode === mode ? "active" : ""}">${mode === "metrics" ? "Perf" : mode[0].toUpperCase() + mode.slice(1)}</button>`).join("")}</div>`;
  const actions = `<div class="board-card-actions"><button class="button compact" type="button" data-board-action="start" data-board-card="${escapeAttr(card.id)}">start</button><button class="button compact" type="button" data-board-action="restart" data-board-card="${escapeAttr(card.id)}">restart</button><button class="button compact" type="button" data-board-action="stop" data-board-card="${escapeAttr(card.id)}">stop</button><button class="icon-button" type="button" data-board-pin data-board-card="${escapeAttr(card.id)}" aria-label="${card.pinned ? "Unpin card" : "Pin card"}" title="${card.pinned ? "Unpin" : "Pin to pinned tasks"}" aria-pressed="${card.pinned}">${icons.pin}</button><button class="icon-button" type="button" data-board-remove data-board-card="${escapeAttr(card.id)}" aria-label="Remove card" title="Remove">&times;</button></div>`;
  const body = card.available === false
    ? `<div class="board-card-note">${escapeHtml(card.skip_reason || "unavailable")}</div>`
    : '<div class="board-card-body" data-board-card-body><div class="board-card-note">Loading&hellip;</div></div>';
  return `<article class="board-card" data-board-card-tile="${escapeAttr(card.id)}" data-card-key="${escapeAttr(key)}">
    <header class="board-card-header"><div class="board-card-title"><span class="task-state-dot board-dot" data-board-dot aria-hidden="true"></span><div><strong>${escapeHtml(card.task)}</strong><span>${escapeHtml(card.workspace_display_name || card.session)} · ${escapeHtml(card.node_name || card.node_id)}</span></div></div>${modes}</header>
    ${body}
    <footer class="board-card-foot">${actions}</footer>
  </article>`;
}

function renderBoardPinned() {
  const pinned = $("#board-pinned");
  if (!pinned) return;
  const cards = boardPinnedCards();
  pinned.innerHTML = cards.length
    ? cards.map((card) => boardCardMarkup(card, { compact: true })).join("")
    : '<div class="muted">Pin cards from any board to watch their status here.</div>';
}

function renderBoardList() {
  const list = $("#board-list");
  if (!list) return;
  list.innerHTML = state.boards.length
    ? state.boards.map((board) => `<article class="workflow-card board-panel">
        <header><div><h2>${escapeHtml(board.name)}</h2><p>${board.cards?.length || 0} card${(board.cards?.length || 0) === 1 ? "" : "s"}</p></div><div class="workflow-card-actions"><button class="button compact" type="button" data-board-edit="${escapeAttr(board.id)}">Edit</button><button class="button compact danger" type="button" data-board-delete="${escapeAttr(board.id)}">Delete</button></div></header>
        <div class="board-card-grid">${board.cards?.length ? board.cards.map((card) => boardCardMarkup({ ...card, key: boardCardKey(card) })).join("") : '<div class="muted">No cards yet. Edit the board to add task cards.</div>'}</div>
      </article>`).join("")
    : '<div class="empty-state compact"><div><h1>No boards</h1><p>Create a board to tile task status, logs, and performance across nodes and workspaces.</p></div></div>';
}

async function refreshBoardLive() {
  if (state.view !== "boards" || state.boardLiveBusy) return;
  state.boardLiveBusy = true;
  try {
    const cards = boardLiveCards();
    const pairs = [...new Set(cards.filter((card) => card.available !== false).map((card) => `${card.node_id}\u0000${card.session}`))];
    await Promise.all(pairs.map((pair) => {
      const [node, session] = pair.split("\u0000");
      return loadBoardSnapshot(node, session);
    }));
    await Promise.all([
      ...cards.filter((card) => card.available !== false && card.mode === "logs").map(loadBoardCardLogs),
      ...cards.filter((card) => card.available !== false && card.mode === "metrics").map(loadBoardCardMetrics),
    ]);
    cards.forEach(updateBoardCardDom);
    updateBoardPinnedDom();
  } finally {
    state.boardLiveBusy = false;
  }
}

async function loadBoardSnapshot(node, session) {
  const key = `${node}\u0000${session}`;
  try {
    const response = await requestJson(`/api/sessions/${encodeURIComponent(session)}?node=${encodeURIComponent(node)}&tail=0`);
    if (!response.ok) throw new Error(response.message);
    state.boardSnapshots[key] = response.data;
  } catch (error) {
    state.boardSnapshots[key] = { error: error.message || "unavailable" };
  }
}

async function loadBoardCardLogs(card) {
  try {
    const response = await requestJson(`/api/sessions/${encodeURIComponent(card.session)}/tasks/${encodeURIComponent(card.task)}/logs?node=${encodeURIComponent(card.node_id)}&limit=80`);
    if (!response.ok) throw new Error(response.message);
    state.boardCardData[card.key] = { ...(state.boardCardData[card.key] || {}), logs: { lines: response.data?.lines || [] } };
  } catch (error) {
    state.boardCardData[card.key] = { ...(state.boardCardData[card.key] || {}), logs: { lines: [], error: error.message || "Logs unavailable" } };
  }
}

async function loadBoardCardMetrics(card) {
  try {
    const response = await requestJson(`/api/sessions/${encodeURIComponent(card.session)}/tasks/${encodeURIComponent(card.task)}/metrics?node=${encodeURIComponent(card.node_id)}&window=600`);
    if (!response.ok) throw new Error(response.message);
    state.boardCardData[card.key] = { ...(state.boardCardData[card.key] || {}), metrics: response.data };
  } catch (error) {
    state.boardCardData[card.key] = { ...(state.boardCardData[card.key] || {}), metrics: null };
  }
}

function updateBoardCardDom(card) {
  const tile = $(`[data-board-card-tile="${CSS.escape(card.id)}"]`);
  if (!tile) return;
  const info = boardCardStatus(card);
  const dot = $(".board-dot", tile);
  if (dot) {
    dot.className = `task-state-dot board-dot ${info.dot || ""}`;
    dot.title = info.label;
  }
  const body = $("[data-board-card-body]", tile);
  if (!body) return;
  if (card.mode === "logs") {
    const logs = state.boardCardData[card.key]?.logs;
    const lines = logs?.lines || [];
    body.innerHTML = lines.length
      ? `<div class="board-logs">${lines.map((line) => `<div class="log-row ${escapeAttr(line.stream)}"><span class="log-text">${escapeHtml(line.text)}</span></div>`).join("")}</div>`
      : `<div class="board-card-note">${escapeHtml(logs?.error || "No output yet")}</div>`;
    body.scrollTop = body.scrollHeight;
  } else if (card.mode === "metrics") {
    const metrics = state.boardCardData[card.key]?.metrics;
    if (!metrics) {
      body.innerHTML = '<div class="board-card-note">No samples</div>';
    } else {
      const current = metrics.current || { cpu_percent: 0, memory_bytes: 0, process_count: 0 };
      const samples = metrics.samples || [];
      const markers = metrics.restart_markers_ms || [];
      body.innerHTML = `<div class="board-metric-summary"><div><span>CPU</span><strong>${Number(current.cpu_percent || 0).toFixed(1)}%</strong></div><div><span>RSS</span><strong>${formatBytes(current.memory_bytes)}</strong></div><div><span>Proc</span><strong>${current.process_count || 0}</strong></div></div>
        <div class="board-metric-chart">${chartMarkup(samples, "cpu_percent", "", (value) => `${value.toFixed(1)}%`, markers)}</div>
        <div class="board-metric-chart">${chartMarkup(samples, "memory_bytes", "memory", formatBytes, markers)}</div>`;
    }
  } else {
    const task = info.task;
    if (!task) {
      body.innerHTML = `<div class="board-card-note">${escapeHtml(info.label)}</div>`;
    } else {
      body.innerHTML = `<div class="board-status-facts"><div><span>Status</span><strong class="status ${escapeAttr(task.status || "unknown")}">${escapeHtml(task.status || "unknown")}</strong></div><div><span>PID</span><strong>${task.pid ?? "-"}</strong></div><div><span>Exit code</span><strong>${task.exit_code ?? "-"}</strong></div><div><span>Command</span><strong title="${escapeAttr(task.command)}">${escapeHtml(task.command)}</strong></div></div>`;
    }
  }
}

function updateBoardPinnedDom() {
  boardPinnedCards().forEach((card) => {
    const tile = $(`[data-pin-tile="${CSS.escape(card.key)}"]`);
    if (!tile) return;
    const info = boardCardStatus(card);
    const dot = $(".board-dot", tile);
    if (dot) {
      dot.className = `task-state-dot board-dot ${info.dot || ""}`;
      dot.title = info.label;
    }
    const status = $("[data-pin-status]", tile);
    if (status) {
      status.className = `status-pill ${info.dot === "failed" ? "error" : ""}`;
      status.textContent = info.label;
    }
  });
}

function findBoardCard(cardId) {
  for (const board of state.boards) {
    const card = board.cards.find((candidate) => candidate.id === cardId);
    if (card) return { board, card };
  }
  return null;
}

async function saveBoardCards(board, cards) {
  try {
    const response = await requestJson(`/api/boards/${encodeURIComponent(board.id)}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        name: board.name,
        cards: cards.map((card) => ({ node_id: card.node_id, session: card.session, task: card.task, mode: card.mode, pinned: card.pinned })),
      }),
    });
    if (!response.ok) throw new Error(response.message);
    await loadBoards();
  } catch (error) {
    showToast(error.message || "Unable to update board");
  }
}

function mutateBoardCard(cardId, transform) {
  const found = findBoardCard(cardId);
  if (!found) return;
  saveBoardCards(
    found.board,
    found.board.cards.map((card) => (card.id === cardId ? transform({ ...card }) : card)),
  );
}

function setBoardCardMode(cardId, mode) {
  mutateBoardCard(cardId, (card) => {
    card.mode = mode;
    return card;
  });
}

function toggleBoardCardPin(cardId) {
  mutateBoardCard(cardId, (card) => {
    card.pinned = !card.pinned;
    return card;
  });
}

function removeBoardCard(cardId) {
  const found = findBoardCard(cardId);
  if (!found) return;
  saveBoardCards(found.board, found.board.cards.filter((card) => card.id !== cardId));
}

async function runBoardCardAction(cardId, action, button) {
  const found = findBoardCard(cardId);
  if (!found) return;
  const card = found.card;
  button.disabled = true;
  try {
    const response = await requestJson("/api/action", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ node: card.node_id, session: card.session, task: card.task, action }),
    });
    if (!response.ok) throw new Error(response.message);
    await refreshBoardLive();
  } catch (error) {
    showToast(error.message || "Action failed");
  } finally {
    button.disabled = false;
  }
}

function beginBoardEditor(board = null) {
  state.boardEditorActive = true;
  state.boardEditingId = board?.id || null;
  state.boardDraftCards = (board?.cards || []).map((card) => ({
    node_id: card.node_id,
    session: card.session,
    task: card.task,
    mode: card.mode || "status",
    pinned: Boolean(card.pinned),
  }));
  $("#board-name").value = board?.name || "";
  renderBoardEditor();
}

function cancelBoardEditor() {
  state.boardEditorActive = false;
  state.boardEditingId = null;
  state.boardDraftCards = [];
  $("#board-name").value = "";
  renderBoardEditor();
}

function renderBoardEditor() {
  const cards = $("#board-cards");
  if (!cards) return;
  $("#board-editor-title").textContent = state.boardEditingId ? "Edit board" : "New board";
  $("#save-board").disabled = !state.boardEditorActive;
  $("#add-board-card").disabled = !state.boardEditorActive;
  $("#cancel-board").disabled = !state.boardEditorActive;
  if (!state.boardEditorActive) {
    cards.innerHTML = '<div class="muted">Select a board to edit, or create a new one.</div>';
    showBoardMessage("", "");
    return;
  }
  cards.innerHTML = state.boardDraftCards.length
    ? state.boardDraftCards.map(renderBoardCardEditor).join("")
    : '<div class="muted">Add at least one task card.</div>';
}

function renderBoardCardEditor(card, index) {
  const targetIndex = state.boardTargets.findIndex((target) => target.node_id === card.node_id && target.session === card.session);
  const missingOption = targetIndex < 0 && card.node_id && card.session
    ? `<option value="-1" selected>${escapeHtml(card.node_id)} / ${escapeHtml(card.session)} (cached or missing)</option>`
    : "";
  const targetOptions = state.boardTargets.map((target, optionIndex) => `<option value="${optionIndex}" ${optionIndex === targetIndex ? "selected" : ""}>${escapeHtml(target.node_name)} / ${escapeHtml(target.workspace_display_name)} (${escapeHtml(target.session)})</option>`).join("");
  const target = targetIndex >= 0 ? state.boardTargets[targetIndex] : null;
  const taskOptions = uniqueStrings([...(target?.tasks || []), card.task].filter(Boolean));
  return `<div class="workflow-member-editor" data-board-card-row="${index}">
    <select data-board-target="${index}" aria-label="Card workspace">${missingOption}${targetOptions}</select>
    <select data-board-task="${index}" aria-label="Card task">${taskOptions.length ? taskOptions.map((task) => `<option value="${escapeAttr(task)}" ${task === card.task ? "selected" : ""}>${escapeHtml(task)}</option>`).join("") : '<option value="">No tasks</option>'}</select>
    <select data-board-mode-select="${index}" aria-label="Card view">${["status", "logs", "metrics"].map((mode) => `<option value="${mode}" ${card.mode === mode ? "selected" : ""}>${mode}</option>`).join("")}</select>
    <label class="board-pin-toggle" title="Pin card"><input type="checkbox" data-board-pin-check="${index}" aria-label="Pin card" ${card.pinned ? "checked" : ""}></label>
    <button class="icon-button" type="button" data-board-remove-row="${index}" aria-label="Remove card" title="Remove">&times;</button>
  </div>`;
}

function showBoardMessage(message, type = "") {
  const element = $("#board-message");
  if (!element) return;
  element.textContent = message || "";
  element.classList.remove("error", "success", "warning");
  if (type) element.classList.add(type);
}

function addBoardCard() {
  const target = state.boardTargets.find((candidate) => candidate.tasks?.length) || state.boardTargets[0];
  if (!target) {
    showBoardMessage("No visible workspace targets are available.", "error");
    return;
  }
  state.boardEditorActive = true;
  state.boardDraftCards.push({ node_id: target.node_id, session: target.session, task: target.tasks?.[0] || "", mode: "status", pinned: false });
  renderBoardEditor();
}

function handleBoardCardChange(event) {
  const target = event.target;
  const index = Number(target.dataset.boardTarget ?? target.dataset.boardTask ?? target.dataset.boardModeSelect ?? target.dataset.boardPinCheck);
  const card = state.boardDraftCards[index];
  if (!card) return;
  if (target.dataset.boardTarget != null) {
    const selected = state.boardTargets[Number(target.value)];
    if (selected) {
      card.node_id = selected.node_id;
      card.session = selected.session;
      if (!selected.tasks?.includes(card.task)) card.task = selected.tasks?.[0] || "";
      renderBoardEditor();
    }
  } else if (target.dataset.boardTask != null) {
    card.task = target.value;
  } else if (target.dataset.boardModeSelect != null) {
    card.mode = target.value;
  } else if (target.dataset.boardPinCheck != null) {
    card.pinned = target.checked;
  }
}

function handleBoardCardEditorButton(event) {
  const button = event.target.closest("[data-board-remove-row]");
  if (!button) return;
  state.boardDraftCards.splice(Number(button.dataset.boardRemoveRow), 1);
  renderBoardEditor();
}

async function saveBoard() {
  if (!state.boardEditorActive) return;
  const name = $("#board-name").value.trim();
  if (!name) {
    showBoardMessage("Board name is required.", "error");
    return;
  }
  if (state.boardDraftCards.some((card) => !card.node_id || !card.session || !card.task)) {
    showBoardMessage("Every card needs a workspace and task.", "error");
    return;
  }
  const body = { name, cards: state.boardDraftCards };
  const url = state.boardEditingId ? `/api/boards/${encodeURIComponent(state.boardEditingId)}` : "/api/boards";
  const method = state.boardEditingId ? "PUT" : "POST";
  showBoardMessage("Saving board…");
  try {
    const response = await requestJson(url, { method, headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    if (!response.ok) throw new Error(response.message);
    state.boardEditorActive = false;
    state.boardEditingId = null;
    state.boardDraftCards = [];
    $("#board-name").value = "";
    showBoardMessage("Board saved.", "success");
    await loadBoards();
  } catch (error) {
    showBoardMessage(error.message || "Unable to save board", "error");
  }
}

async function deleteBoard(id) {
  const board = state.boards.find((item) => item.id === id);
  if (!board || !confirm(`Delete board '${board.name}'?`)) return;
  try {
    const response = await requestJson(`/api/boards/${encodeURIComponent(id)}`, { method: "DELETE" });
    if (!response.ok) throw new Error(response.message);
    if (state.boardEditingId === id) cancelBoardEditor();
    await loadBoards();
    showToast("Board deleted");
  } catch (error) {
    showBoardMessage(error.message || "Unable to delete board", "error");
  }
}

async function loadNodeSettings() {
  const requestId = ++state.nodeSettingsRequest;
  const node = selectedNode();
  if (!node) return;
  const subtitle = $("#node-settings-subtitle");
  try {
    const response = await requestJson(`/api/nodes/${encodeURIComponent(node)}/settings`);
    if (requestId !== state.nodeSettingsRequest || selectedNode() !== node) return;
    if (!response.ok) throw new Error(response.message);
    state.nodeSettings = {
      settings: response.data.settings || response.data,
      environment_overrides: response.data.environment_overrides || [],
    };
    renderNodeSettings();
    subtitle.textContent = `${state.nodeSettings.settings.name} · ${state.nodeSettings.settings.node_id}`;
  } catch (error) {
    if (requestId !== state.nodeSettingsRequest) return;
    state.nodeSettings = null;
    showSettingsMessage("#node-settings-message", error.message || "Unable to load node settings", "error");
    subtitle.textContent = "Node settings unavailable";
  }
}

function overrideFor(field) {
  return state.nodeSettings?.environment_overrides?.find((override) => override.field === field) || null;
}

function renderNodeSettings() {
  const settings = state.nodeSettings?.settings;
  if (!settings) return;
  $("#node-name").value = settings.name || "";
  $("#node-role").value = settings.role || "worker";
  $("#node-leader-mode").value = settings.leader_mode || "standard";
  $("#node-leader-url").value = settings.leader_url || "";
  $("#node-bind-host").value = settings.bind_host || "";
  $("#node-web-port").value = settings.web_port || "";
  $("#node-token-mode").value = "keep";
  $("#node-token-value").value = "";
  const leader = settings.role === "leader";
  $("#leader-mode-field").hidden = !leader;
  $("#leader-url-field").hidden = !leader;
  ["name","role","leader_mode","leader_url","bind_host","web_port","enrollment_token"].forEach((field) => {
    const override = overrideFor(field);
    const selector = field === "enrollment_token" ? "#node-token-mode" : `#node-${field}`;
    const input = $(selector);
    if (input) {
      input.disabled = Boolean(override);
      input.title = override ? `Controlled by ${override.variable}` : "";
    }
  });
  $("#node-settings-overrides").textContent = state.nodeSettings?.environment_overrides?.length
    ? `Environment overrides: ${state.nodeSettings.environment_overrides.map((override) => override.variable).join(", ")}`
    : "";
}

function showSettingsMessage(selector, message, type = "") {
  const element = $(selector);
  element.textContent = message || "";
  element.classList.remove("error", "success", "warning");
  if (type) element.classList.add(type);
}

function renderAliasForm() {
  const select = $("#alias-session");
  const selected = select.value;
  select.innerHTML = state.workspaces.length
    ? state.workspaces.map((workspace) => `<option value="${escapeAttr(workspace.session)}">${escapeHtml(workspace.display_name)}</option>`).join("")
    : '<option value="">No workspaces</option>';
  if (state.workspaces.some((workspace) => workspace.session === selected)) select.value = selected;
  if (document.activeElement?.id !== "alias-value") {
    $("#alias-value").value = state.workspaces.find((workspace) => workspace.session === select.value)?.alias || "";
  }
}

async function loadServiceStatus() {
  try {
    const response = await requestJson(`/api/nodes/self/service?scope=${encodeURIComponent($("#service-scope").value)}`);
    if (!response.ok) throw new Error(response.message);
    state.serviceStatus = response.data;
    renderServiceStatus();
  } catch (error) {
    state.serviceStatus = null;
    $("#service-status").textContent = error.message || "Unable to load service status";
  }
}

function renderServiceStatus() {
  const status = state.serviceStatus;
  if (!status) return;
  const enabled = status.enabled ? "enabled" : "disabled";
  const running = status.running ? "running" : "stopped";
  $("#service-status").textContent = `${status.platform} ${status.scope} service: ${status.installed ? "installed" : "not installed"} · ${enabled} · ${running} · ${status.unit}`;
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
    await loadWorkspaces();
    if (state.view === "settings") await loadNodeSettings();
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
  } else if (state.view === "audit") {
    meta.textContent = `${state.auditPage.total || 0} audit record${state.auditPage.total === 1 ? "" : "s"}`;
  } else if (state.view === "runs") {
    meta.textContent = `${state.runPage.total||0} run${(state.runPage.total||0)===1?"":"s"}`;
  } else if (state.view === "docs") {
    meta.textContent = "Local Streamable HTTP endpoint";
  } else if (state.view === "settings") {
    meta.textContent = "Node, workspace, and daemon settings";
  } else if (state.view === "workflows") {
    const count = state.workflowGroups.length;
    meta.textContent = `${count} workflow group${count === 1 ? "" : "s"}`;
  } else if (state.view === "boards") {
    const pinned = boardPinnedCards().length;
    meta.textContent = `${state.boards.length} board${state.boards.length === 1 ? "" : "s"} · ${pinned} pinned task${pinned === 1 ? "" : "s"}`;
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
      select.innerHTML = renderSessionOptions(sessions);
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
    const labels = orderedTaskLabels(state.snapshot);
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
  const taskStates = labels.map((label) => {
    const task = state.snapshot?.tasks?.[label] || {};
    return [label, task.status, task.run_generation, state.seenExits[exitKey(label)] || 0];
  });
  const signature = JSON.stringify([taskStates, state.currentTask, state.tabOrderSaving]);
  if (signature === state.tabsSignature) return;
  state.tabsSignature = signature;
  $("#tabs").classList.toggle("saving-order", state.tabOrderSaving);
  $("#tabs").innerHTML = labels.map((label) => {
    const task = state.snapshot.tasks[label] || {};
    const dot = taskStateDot(label, task);
    const statusLabel = dot ? `, ${dot.label}` : "";
    return `<button class="tab ${label === state.currentTask ? "active" : ""}" type="button" role="tab" tabindex="${label === state.currentTask ? "0" : "-1"}" aria-selected="${label === state.currentTask}" aria-label="${escapeAttr(label + statusLabel)}" data-task="${escapeAttr(label)}" data-order-key="${escapeAttr(label)}">${dot ? `<span class="task-state-dot ${dot.className}" title="${escapeAttr(dot.label)}"></span>` : ""}<span>${escapeHtml(label)}</span></button>`;
  }).join("");
}

function orderedTaskLabels(snapshot) {
  const tasks = snapshot?.tasks || {};
  const labels = [];
  (snapshot?.task_order || []).forEach((label) => {
    if (Object.hasOwn(tasks, label) && !labels.includes(label)) labels.push(label);
  });
  Object.keys(tasks).forEach((label) => { if (!labels.includes(label)) labels.push(label); });
  return labels;
}

function taskStateDot(label, task) {
  if (task.status === "running") return { className: "running", label: "Running" };
  if (task.status === "failed") return { className: "failed", label: "Exited with error" };
  const generation = Number(task.run_generation || 0);
  if (task.status === "exited" && Number(state.seenExits[exitKey(label)] || 0) !== generation) {
    return { className: "exited", label: "Finished" };
  }
  return null;
}

function editableTaskPayload(task) {
  return {
    label: task.label,
    command: task.command,
    args: [...(task.args || [])],
    cwd: task.cwd || ".",
    env: { ...(task.env || {}) },
    shell: Boolean(task.shell),
    auto_start: Boolean(task.auto_start),
    stop_timeout_ms: Number(task.stop_timeout_ms || 3000),
    clear_logs_on_restart: Boolean(task.clear_logs_on_restart),
    schedule: task.schedule || null,
  };
}

async function persistWorkspaceOrder(order, previousOrder) {
  if (state.tabOrderSaving || !state.snapshot) return;
  const session = state.snapshot.name;
  const node = selectedNode();
  state.tabOrderSaving = true;
  state.snapshot.task_order = [...order];
  state.tabsSignature = "";
  renderTabs(order);
  try {
    const query = new URLSearchParams({ node });
    const current = await requestJson(`/api/sessions/${encodeURIComponent(session)}/config?${query}`);
    if (!current.ok) throw new Error(current.message);
    if (selectedNode() !== node || state.snapshot?.name !== session) throw new Error("The selected workspace changed");
    const byLabel = new Map((current.data.tasks || []).map((task) => [task.label, task]));
    if (order.some((label) => !byLabel.has(label)) || byLabel.size !== order.length) {
      throw new Error("Task configuration changed; reload before reordering");
    }
    const response = await requestJson(`/api/sessions/${encodeURIComponent(session)}/config?${query}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        revision: current.data.revision,
        tasks: order.map((label) => editableTaskPayload(byLabel.get(label))),
      }),
    });
    if (!response.ok) throw new Error(response.message);
    showToast("Task order saved");
  } catch (error) {
    if (state.snapshot?.name === session && selectedNode() === node) {
      state.snapshot.task_order = [...previousOrder];
    }
    showToast(error.message || "Unable to save task order");
  } finally {
    state.tabOrderSaving = false;
    state.tabsSignature = "";
    if (state.snapshot) renderTabs(orderedTaskLabels(state.snapshot));
  }
}

function bindPointerSorter(container, itemSelector, handleSelector, onReorder) {
  let drag = null;
  container.addEventListener("pointerdown", (event) => {
    if (event.button !== 0 || state.tabOrderSaving) return;
    if (handleSelector && !event.target.closest(handleSelector)) return;
    const item = event.target.closest(itemSelector);
    if (!item || !container.contains(item)) return;
    drag = {
      item,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      active: false,
      original: $$(itemSelector, container).map((element) => element.dataset.orderKey),
    };
    item.setPointerCapture?.(event.pointerId);
  });
  container.addEventListener("pointermove", (event) => {
    if (!drag || drag.pointerId !== event.pointerId) return;
    if (!drag.active && Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) < 6) return;
    drag.active = true;
    drag.item.classList.add("dragging");
    event.preventDefault();
    const target = document.elementFromPoint(event.clientX, event.clientY)?.closest(itemSelector);
    if (!target || target === drag.item || !container.contains(target)) return;
    const rect = target.getBoundingClientRect();
    const horizontal = container.id === "tabs";
    const before = horizontal ? event.clientX < rect.left + rect.width / 2 : event.clientY < rect.top + rect.height / 2;
    container.insertBefore(drag.item, before ? target : target.nextSibling);
  });
  const finish = (event) => {
    if (!drag || drag.pointerId !== event.pointerId) return;
    drag.item.releasePointerCapture?.(event.pointerId);
    drag.item.classList.remove("dragging");
    if (drag.active) {
      state.suppressTabClick = container.id === "tabs";
      const order = $$(itemSelector, container).map((element) => element.dataset.orderKey);
      onReorder(order, drag.original);
      setTimeout(() => { state.suppressTabClick = false; }, 80);
    }
    drag = null;
  };
  container.addEventListener("pointerup", finish);
  container.addEventListener("pointercancel", finish);
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
          <button class="icon-button" type="button" data-log="clear-search" aria-label="Clear search" title="Clear search">&#215;</button>
          <button class="icon-button" type="button" data-log="top" aria-label="Go to top" title="Go to top">&#8679;</button>
          <button class="icon-button" type="button" data-log="bottom" aria-label="Go to bottom" title="Go to bottom">&#8681;</button>
          <button class="button compact" id="follow-button" type="button" data-log="follow">Focus</button>
          <button class="icon-button" type="button" data-log="fullscreen" aria-label="Full screen logs" title="Full screen logs">&#x26F6;</button>
          <button class="icon-button danger-icon" type="button" data-log="clear-history" aria-label="Clear logs and performance history" title="Clear logs and performance history">${icons.trash}</button>
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
  if (action === "clear-search") {
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
  if (action === "clear-history") clearTaskHistory();
}

async function clearTaskHistory() {
  const session = state.snapshot?.name;
  const task = state.currentTask;
  const node = selectedNode();
  if (!session || !task || !node || selectedNodeState()?.online === false) return;
  try {
    const response = await requestJson(`/api/sessions/${encodeURIComponent(session)}/tasks/${encodeURIComponent(task)}/history?${addNodeQuery()}`, {
      method: "DELETE",
    });
    if (!response.ok) throw new Error(response.message);
    resetLogCursor();
    state.metrics = null;
    renderLogs([]);
    renderMetrics();
    await Promise.all([loadLogs(), loadMetrics()]);
    showToast("Logs and performance history cleared");
  } catch (error) {
    showToast(error.message || "Unable to clear history");
  }
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

function chartMarkup(samples, key, lineClass, formatter, restartMarkers = []) {
  const values = samples.map((sample) => Number(sample[key] || 0));
  const max = Math.max(...values, 1);
  const points = values.map((value, index) => {
    const x = values.length <= 1 ? 0 : (index / (values.length - 1)) * 300;
    const y = 70 - (value / max) * 64;
    return `${x.toFixed(2)},${y.toFixed(2)}`;
  }).join(" ");
  const firstTimestamp = Number(samples[0]?.timestamp_ms || 0);
  const lastTimestamp = Number(samples.at(-1)?.timestamp_ms || firstTimestamp);
  const span = Math.max(lastTimestamp - firstTimestamp, 1);
  const markers = restartMarkers
    .map(Number)
    .filter((timestamp) => timestamp >= firstTimestamp && timestamp <= lastTimestamp)
    .map((timestamp) => ((timestamp - firstTimestamp) / span) * 300)
    .map((x) => `<path class="chart-restart" d="M${x.toFixed(2)} 4V70"/>`)
    .join("");
  return `<svg viewBox="0 0 300 72" preserveAspectRatio="none" aria-hidden="true"><path class="chart-grid" d="M0 70H300M0 38H300M0 6H300"/>${markers}${points ? `<polyline class="chart-line ${lineClass}" points="${points}"/>` : ""}</svg><div class="chart-title"><span>10 minute history</span><strong>${formatter(max)}</strong></div>`;
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
  const restartMarkers = metrics.restart_markers_ms || [];
  $("#monitor-state").textContent = metrics.running ? "Live - 1s" : "Stopped";
  const processRows = (metrics.processes || []).map((process) => `<tr><td>${process.pid}</td><td>${process.ppid ?? "-"}</td><td class="name" title="${escapeAttr(process.name)}">${escapeHtml(process.name)}</td><td>${Number(process.cpu_percent || 0).toFixed(1)}%</td><td>${formatBytes(process.memory_bytes)}</td><td>${escapeHtml(process.status)}</td><td>${formatRuntime(process.run_time_seconds)}</td></tr>`).join("");
  body.innerHTML = `
    <div class="metric-summary"><div><span>CPU</span><strong>${Number(current.cpu_percent || 0).toFixed(1)}%</strong></div><div><span>RSS</span><strong>${formatBytes(current.memory_bytes)}</strong></div><div><span>Processes</span><strong>${current.process_count || 0}</strong></div></div>
    <div class="metric-charts"><div class="metric-chart"><div class="chart-title"><span>CPU</span><strong>${Number(current.cpu_percent || 0).toFixed(1)}%</strong></div>${chartMarkup(samples, "cpu_percent", "", (value) => `${value.toFixed(1)}%`, restartMarkers)}</div><div class="metric-chart"><div class="chart-title"><span>Memory</span><strong>${formatBytes(current.memory_bytes)}</strong></div>${chartMarkup(samples, "memory_bytes", "memory", formatBytes, restartMarkers)}</div></div>
    <div class="process-table-wrap">${processRows ? `<table class="processes"><thead><tr><th>PID</th><th>PPID</th><th>Name</th><th>CPU</th><th>RSS</th><th>Status</th><th>Runtime</th></tr></thead><tbody>${processRows}</tbody></table>` : '<div class="monitor-empty">No running processes</div>'}</div>`;
  body.scrollTop = previousTop;
  body.scrollLeft = previousLeft;
}

function mcpTarget(input, targetNode) {
  const node = targetNode ? `Node ${targetNode}` : "Cluster";
  if (input?.action === "sessions" && !targetNode) return { primary: "All sessions", secondary: node };
  if (input?.task) return { primary: input.task, secondary: `${node} · ${input.session || "Task"}` };
  if (input?.session) return { primary: input.session, secondary: `${node} · ${input.tail ? `Last ${input.tail} lines` : "Session"}` };
  return { primary: targetNode || "Taskdeck cluster", secondary: targetNode ? "Node target" : "Cluster operation" };
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
      const target = mcpTarget(call.input || {}, call.target_node);
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
    const input = call.request?.params?.arguments || {};
    const target = mcpTarget(input, call.target_node);
    $("#call-dialog-title").textContent = `${titleCase(call.operation || "MCP")} request`;
    $("#call-dialog-subtitle").textContent = `${call.tool} - Call #${call.id}`;
    const icon = $("#detail-status-icon");
    icon.classList.toggle("error", !call.success);
    icon.innerHTML = call.success ? '<svg viewBox="0 0 24 24"><path d="m7.5 12 3 3 6-7"/></svg>' : '<svg viewBox="0 0 24 24"><path d="m8 8 8 8m0-8-8 8"/></svg>';
    $("#call-overview").innerHTML = `<div class="overview-item"><span>Started</span><strong>${escapeHtml(new Date(call.started_at_ms).toLocaleString())}</strong></div><div class="overview-item"><span>Duration</span><strong>${Number(call.duration_ms || 0)} ms</strong></div><div class="overview-item"><span>Request ID</span><strong>${escapeHtml(call.request?.id ?? "-")}</strong></div>`;
    $("#request-fields").innerHTML = requestFields(call, target);
    const result = structuredCallResult(call);
    const message = result && typeof result === "object" && typeof result.message === "string" ? result.message : typeof result === "string" ? result : call.success ? "The request completed successfully." : "The request could not be completed.";
    const data = result && typeof result === "object" && Object.hasOwn(result, "data") ? result.data : result;
    $("#response-summary").innerHTML = `<div class="outcome ${call.success ? "" : "error"}"><span class="outcome-icon">${call.success ? "✓" : "×"}</span><strong>${call.success ? "Completed successfully" : "Request failed"}</strong><p>${escapeHtml(message)}</p></div>`;
    $("#response-data").innerHTML = renderResultData(data);
    $("#call-request").textContent = formatCallValue(call.request);
    $("#call-response").textContent = formatCallValue(call.response);
    setCallDetailMode("result");
    if (!$("#call-dialog").open) $("#call-dialog").showModal();
  } catch (error) {
    if (requestId === state.callDetailRequest && state.callDetailId === targetId) showToast(error.message || "Unable to load call");
  }
}

function requestFields(call, target) {
  const input = call.request?.params?.arguments || {};
  const fields = [["Operation", titleCase(input.action || call.operation || "Unknown"), false], ["Target", target.primary, true]];
  if (call.target_node) fields.push(["Target node", call.target_node, true]);
  if (input.session) fields.push(["Session", input.session, true]);
  if (input.task) fields.push(["Task", input.task, false]);
  else if (["start", "stop", "restart", "pause", "resume"].includes(input.action)) fields.push(["Task", "All tasks in the session", false]);
  if (input.tail != null) fields.push(["Log lines", input.tail, false]);
  Object.entries(input).filter(([key]) => !["action", "session", "task", "tail"].includes(key)).forEach(([key, value]) => fields.push([titleCase(key), displayValue(value), typeof value === "string"]));
  return fields.map(([label, value, mono]) => `<div class="field-row"><span class="field-label">${escapeHtml(label)}</span><strong class="field-value ${mono ? "mono" : ""}">${escapeHtml(value)}</strong></div>`).join("");
}

function displayValue(value) {
  if (value == null) return "Not set";
  if (Array.isArray(value)) return value.map(displayValue).join(", ");
  if (typeof value === "object") return Object.entries(value).map(([key, item]) => `${titleCase(key)}: ${displayValue(item)}`).join(" - ");
  return String(value);
}

function renderResultData(data) {
  if (data == null || data === "") return '<div class="result-empty">No structured result data.</div>';
  if (Array.isArray(data)) return `<div class="result-data">${data.length ? `<div class="value-list">${data.map((item) => `<div class="value-list-item">${escapeHtml(displayValue(item))}</div>`).join("")}</div>` : '<div class="result-empty">No items returned.</div>'}</div>`;
  if (typeof data === "object" && data.tasks) {
    const facts = [["Session", data.name], ["Project", data.project], ["Source", data.source]].filter(([, value]) => value != null).map(([label, value]) => `<div class="field-row"><span class="field-label">${escapeHtml(label)}</span><strong class="field-value ${label !== "Source" ? "mono" : ""}">${escapeHtml(value)}</strong></div>`).join("");
    const tasks = Object.entries(data.tasks).map(([name, task]) => renderTaskResult(name, task)).join("");
    return `<div class="result-data"><div class="session-facts"><div class="field-list">${facts}</div></div>${tasks || '<div class="result-empty">This session has no tasks.</div>'}</div>`;
  }
  if (typeof data === "object") return `<div class="result-data"><div class="field-list">${Object.entries(data).filter(([key]) => !["ok", "message"].includes(key)).map(([key, value]) => `<div class="field-row"><span class="field-label">${escapeHtml(titleCase(key))}</span><strong class="field-value">${escapeHtml(displayValue(value))}</strong></div>`).join("")}</div></div>`;
  return `<div class="result-data"><div class="value-list-item">${escapeHtml(displayValue(data))}</div></div>`;
}

function renderTaskResult(name, task) {
  const logs = Array.isArray(task.logs) ? task.logs : [];
  const status = task.status || "unknown";
  const statusClass = status === "failed" ? "error" : status === "paused" ? "paused" : "success";
  return `<article class="task-result"><div class="task-result-header"><strong>${escapeHtml(name)}</strong><span class="status-pill ${statusClass}">${escapeHtml(titleCase(status))}</span></div><div class="task-result-meta">${task.pid ? `<span>PID ${task.pid}</span>` : ""}${task.command ? `<span>${escapeHtml(task.command)}</span>` : ""}${task.cwd ? `<span>${escapeHtml(task.cwd)}</span>` : ""}${task.last_exit ? `<span>Last exit: ${escapeHtml(task.last_exit)}</span>` : ""}</div>${logs.length ? `<details class="log-disclosure"><summary>${logs.length} recent log line${logs.length === 1 ? "" : "s"}</summary><div class="human-logs">${logs.map((line) => `<div class="human-log"><span>${escapeHtml(line.stream)}</span><span>${escapeHtml(line.text)}</span></div>`).join("")}</div></details>` : ""}</article>`;
}

function structuredCallResult(call) {
  const result = call.response?.result;
  if (result?.structuredContent !== undefined) return result.structuredContent;
  const content = Array.isArray(result?.content) ? result.content : [];
  for (const item of content) {
    if (item?.type !== "text" || typeof item.text !== "string") continue;
    try { return JSON.parse(item.text); } catch (_) { /* plain text fallback */ }
  }
  const text = content.filter((item) => item?.type === "text").map((item) => item.text).join("\n");
  return text || result || call.response;
}

function formatCallValue(value) {
  return typeof value === "string" ? value : JSON.stringify(value ?? null, null, 2);
}

function setCallDetailMode(mode) {
  state.callDetailMode = mode;
  $$("[data-call-mode]", $("#call-dialog")).forEach((button) => {
    const active = button.dataset.callMode === mode;
    button.classList.toggle("active", active);
    button.setAttribute("aria-selected", String(active));
  });
  $$("[data-call-panel]", $("#call-dialog")).forEach((panel) => {
    panel.hidden = panel.dataset.callPanel !== mode;
  });
}

function closeCallDetails() {
  state.callDetailRequest += 1;
  state.callDetailId = null;
  if ($("#call-dialog").open) $("#call-dialog").close();
}

function auditQuery() {
  const filters = state.auditFilters;
  const query = new URLSearchParams({ page: String(filters.page), page_size: String(filters.pageSize) });
  if (filters.q) query.set("q", filters.q);
  if (filters.source && filters.source !== "all") query.set("source", filters.source);
  if (filters.status && filters.status !== "all") query.set("status", filters.status);
  if (filters.node) query.set("node", filters.node);
  if (filters.session) query.set("session", filters.session);
  if (filters.task) query.set("task", filters.task);
  if (filters.operation) query.set("operation", filters.operation);
  return query.toString();
}

async function loadAudit() {
  const query = auditQuery();
  const requestId = ++state.auditRequest;
  if (!state.audits.length) {
    $("#audit-summary").textContent = "Loading audit records...";
    $("#audit-body").innerHTML = '<tr class="empty-row loading-row"><td colspan="9">Loading audit records...</td></tr>';
  }
  try {
    const response = await requestJson(`/api/audit?${query}`);
    if (requestId !== state.auditRequest || query !== auditQuery()) return;
    if (!response.ok) throw new Error(response.message);
    state.auditPage = response.data || state.auditPage;
    state.audits = state.auditPage.items || [];
    renderAudit();
    updateMeta();
  } catch (error) {
    if (requestId === state.auditRequest) {
      const message = error.message || "Audit log unavailable";
      $("#audit-summary").textContent = message;
      $("#audit-body").innerHTML = `<tr class="empty-row error-row"><td colspan="9">${escapeHtml(message)}</td></tr>`;
      $("#audit-page-label").textContent = "Page 0 of 0";
      $("#audit-prev").disabled = true;
      $("#audit-next").disabled = true;
    }
  }
}

function renderAudit() {
  const body = $("#audit-body");
  const signature = JSON.stringify(state.audits.map((record) => [record.audit_id, record.status, record.duration_ms, record.replicated_at_ms]));
  if (signature !== state.auditSignature) {
    state.auditSignature = signature;
    const focused = document.activeElement?.closest?.("[data-audit-id]")?.dataset.auditId;
    body.innerHTML = state.audits.length ? state.audits.map((record) => {
      const target = auditTarget(record);
      const node = auditNode(record);
      return `<tr><td>${escapeHtml(new Date(record.timestamp_ms).toLocaleString())}</td><td><span class="source-pill">${escapeHtml(String(record.source || "unknown").toUpperCase())}</span></td><td><div class="cell-stack"><strong>${escapeHtml(node.primary)}</strong><span>${escapeHtml(node.secondary)}</span></div></td><td><div class="cell-stack"><strong>${escapeHtml(titleCase(record.operation || record.request_kind || "request"))}</strong><span>${escapeHtml(record.request_kind || "request")}</span></div></td><td><div class="cell-stack"><strong>${escapeHtml(target.primary)}</strong><span>${escapeHtml(target.secondary)}</span></div></td><td><span class="status-pill ${auditStatusClass(record.status)}">${escapeHtml(titleCase(record.status || "unknown"))}</span></td><td>${Number(record.duration_ms || 0)} ms</td><td>${escapeHtml(auditSyncLabel(record))}</td><td><button class="button compact" type="button" data-audit-id="${escapeAttr(record.audit_id)}">View</button></td></tr>`;
    }).join("") : '<tr class="empty-row"><td colspan="9">No matching audit records.</td></tr>';
    if (focused) $(`[data-audit-id="${CSS.escape(focused)}"]`, body)?.focus();
  }
  $("#audit-summary").textContent = `${state.auditPage.total || 0} retained audit record${state.auditPage.total === 1 ? "" : "s"}`;
  $("#audit-page-label").textContent = state.auditPage.total_pages ? `Page ${state.auditPage.page} of ${state.auditPage.total_pages}` : "Page 0 of 0";
  $("#audit-prev").disabled = !state.auditPage.has_previous;
  $("#audit-next").disabled = !state.auditPage.has_next;
}

function auditStatusClass(status) {
  if (status === "error" || status === "timeout") return "error";
  if (status === "started") return "paused";
  return "success";
}

function auditNode(record) {
  const origin = record.origin_node_id || "unknown";
  const executor = record.executor_node_id || "unknown";
  if (origin === executor) return { primary: executor, secondary: "origin + executor" };
  return { primary: executor, secondary: `from ${origin}` };
}

function auditTarget(record) {
  if (record.task) return { primary: record.task, secondary: record.session || "Task" };
  if (record.session) return { primary: record.session, secondary: "Session" };
  return { primary: record.operation || record.request_kind || "Request", secondary: record.correlation_id || "No target" };
}

function auditSyncLabel(record) {
  if (record.replicated_at_ms) return `Synced ${new Date(record.replicated_at_ms).toLocaleTimeString()}`;
  if (record.transport === "agent") return "Awaiting ack";
  return "Local pending";
}

function scheduleAuditReload() {
  clearTimeout(state.auditDebounce);
  state.auditDebounce = setTimeout(() => {
    state.auditFilters.page = 1;
    loadAudit();
  }, 200);
}

async function openAuditDetails(id) {
  const targetId = String(id);
  const requestId = ++state.auditDetailRequest;
  state.auditDetailId = targetId;
  try {
    const response = await requestJson(`/api/audit/${encodeURIComponent(id)}`);
    if (requestId !== state.auditDetailRequest || state.auditDetailId !== targetId || state.view !== "audit") return;
    if (!response.ok) throw new Error(response.message);
    const record = response.data;
    if (String(record.audit_id) !== targetId) throw new Error("Audit detail response did not match the requested record");
    const target = auditTarget(record);
    const node = auditNode(record);
    $("#audit-dialog-title").textContent = `${titleCase(record.operation || record.request_kind || "Request")} audit`;
    $("#audit-dialog-subtitle").textContent = `${String(record.source || "unknown").toUpperCase()} · ${record.audit_id}`;
    const icon = $("#audit-detail-status-icon");
    icon.classList.toggle("error", !record.success);
    icon.innerHTML = record.success ? '<svg viewBox="0 0 24 24"><path d="m7.5 12 3 3 6-7"/></svg>' : '<svg viewBox="0 0 24 24"><path d="m8 8 8 8m0-8-8 8"/></svg>';
    $("#audit-overview").innerHTML = `<div class="overview-item"><span>Time</span><strong>${escapeHtml(new Date(record.timestamp_ms).toLocaleString())}</strong></div><div class="overview-item"><span>Duration</span><strong>${Number(record.duration_ms || 0)} ms</strong></div><div class="overview-item"><span>Correlation</span><strong>${escapeHtml(record.correlation_id || "-")}</strong></div><div class="overview-item"><span>Sync</span><strong>${escapeHtml(auditSyncLabel(record))}</strong></div>`;
    $("#audit-context-fields").innerHTML = auditContextFields(record, node, target);
    $("#audit-response-summary").innerHTML = `<div class="outcome ${record.success ? "" : "error"}"><span class="outcome-icon">${record.success ? "✓" : "×"}</span><strong>${record.success ? "Completed successfully" : titleCase(record.status || "Failed")}</strong><p>${escapeHtml(record.error || record.response?.message || "No error summary.")}</p></div>`;
    $("#audit-request").textContent = formatCallValue(record.request);
    $("#audit-response").textContent = formatCallValue(record.response);
    $("#audit-details").textContent = formatCallValue(record.details);
    setAuditDetailMode("summary");
    if (!$("#audit-dialog").open) $("#audit-dialog").showModal();
  } catch (error) {
    if (requestId === state.auditDetailRequest && state.auditDetailId === targetId) showToast(error.message || "Unable to load audit record");
  }
}

function auditContextFields(record, node, target) {
  const fields = [
    ["Source", titleCase(record.source || "unknown"), false],
    ["Transport", titleCase(record.transport || "unknown"), false],
    ["Executor node", node.primary, true],
    ["Origin node", record.origin_node_id || "unknown", true],
    ["Operation", titleCase(record.operation || record.request_kind || "request"), false],
    ["Target", target.primary, true],
  ];
  if (record.session) fields.push(["Session", record.session, true]);
  if (record.task) fields.push(["Task", record.task, false]);
  if (record.error) fields.push(["Error", record.error, false]);
  return fields.map(([label, value, mono]) => `<div class="field-row"><span class="field-label">${escapeHtml(label)}</span><strong class="field-value ${mono ? "mono" : ""}">${escapeHtml(value)}</strong></div>`).join("");
}

function setAuditDetailMode(mode) {
  state.auditDetailMode = mode;
  $$('[data-audit-mode]', $("#audit-dialog")).forEach((button) => {
    const active = button.dataset.auditMode === mode;
    button.classList.toggle("active", active);
    button.setAttribute("aria-selected", String(active));
  });
  $$('[data-audit-panel]', $("#audit-dialog")).forEach((panel) => {
    panel.hidden = panel.dataset.auditPanel !== mode;
  });
}

function closeAuditDetails() {
  state.auditDetailRequest += 1;
  state.auditDetailId = null;
  if ($("#audit-dialog").open) $("#audit-dialog").close();
}

function taskToDraft(task) {
  return {
    _key: globalThis.crypto?.randomUUID?.() || `task-${Date.now()}-${Math.random()}`,
    label: task.label,
    command: task.command,
    args: [...(task.args || [])],
    cwd: task.cwd || ".",
    envRows: Object.entries(task.env || {}).map(([key, value]) => ({ key, value })),
    shell: Boolean(task.shell),
    auto_start: Boolean(task.auto_start),
    stop_timeout_ms: Number(task.stop_timeout_ms || 3000),
    clear_logs_on_restart: Boolean(task.clear_logs_on_restart),
    schedule: task.schedule ?? null,
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
    state.configWorkspaceEnvRows = Object.entries(response.data.workspace_env || {}).map(([key,value])=>({key,value}));
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
  renderConfigWorkspace();
  renderConfigForm();
}

function renderConfigTaskList() {
  $("#config-task-list").innerHTML = state.configTasks.length ? state.configTasks.map((task, index) => `<div class="config-task-item" data-order-key="${escapeAttr(task._key)}"><button class="drag-handle" type="button" data-drag-handle aria-label="Drag ${escapeAttr(task.label || "Untitled task")}" title="Drag to reorder">${icons.grip}</button><button class="config-task-button ${index === state.configTaskIndex ? "active" : ""}" type="button" data-config-task="${index}"><span>${escapeHtml(task.label || "Untitled task")}</span><small>${task.origin.imported ? "VS Code import" : "YAML task"}</small></button></div>`).join("") : '<div class="monitor-empty">No tasks</div>';
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
    <label class="field"><span>Cron schedule (5 or 6 fields; empty disables)</span><input data-field="schedule" value="${escapeAttr(task.schedule || "")}" placeholder="*/10 * * * *"></label>
    <div class="field-grid"><label class="field"><span>Working directory</span><input data-field="cwd" value="${escapeAttr(task.cwd)}"></label><label class="field"><span>Stop timeout (ms)</span><input data-field="stop_timeout_ms" type="number" min="1" max="300000" value="${task.stop_timeout_ms}"></label></div>
    <div class="toggle-row"><label><input data-field="shell" type="checkbox" ${task.shell ? "checked" : ""}> Run through shell</label><label><input data-field="auto_start" type="checkbox" ${task.auto_start ? "checked" : ""}> Auto start</label><label><input data-field="clear_logs_on_restart" type="checkbox" ${task.clear_logs_on_restart ? "checked" : ""}> Clear logs and performance history on restart</label></div>
    <div class="origin-note">${task.origin.imported ? "Imported from .vscode/tasks.json; Taskdeck saves only overrides." : "Defined in taskdeck.yaml."}</div>
    <fieldset class="field"><legend>Arguments</legend><div class="repeater" id="args-rows">${task.args.map((arg, index) => `<div class="repeater-row"><input data-arg="${index}" value="${escapeAttr(arg)}" aria-label="Argument ${index + 1}"><button class="icon-button" type="button" data-remove-arg="${index}" aria-label="Remove argument" title="Remove">&#215;</button></div>`).join("")}</div><button class="button compact" type="button" data-add-arg>Add argument</button></fieldset>
    <fieldset class="field"><legend>Environment</legend><div class="repeater" id="env-rows">${task.envRows.map((row, index) => `<div class="repeater-row env"><input data-env-key="${index}" value="${escapeAttr(row.key)}" placeholder="NAME" aria-label="Environment key"><input data-env-value="${index}" value="${escapeAttr(row.value)}" placeholder="Value" aria-label="Environment value"><button class="icon-button" type="button" data-remove-env="${index}" aria-label="Remove environment variable" title="Remove">&#215;</button></div>`).join("")}</div><button class="button compact" type="button" data-add-env>Add variable</button></fieldset>
    <button class="button danger" type="button" data-delete-task>Delete task</button>`;
}

function renderConfigWorkspace() {
  const rows=state.configWorkspaceEnvRows;
  const container=$("#workspace-env-rows");
  if(container){
    container.innerHTML=rows.map((row,index)=>`<div class="repeater-row env"><input data-workspace-key="${index}" value="${escapeAttr(row.key)}" placeholder="NAME"><input data-workspace-value="${index}" value="${escapeAttr(row.value)}" placeholder="Value"><button class="icon-button" type="button" data-remove-workspace="${index}" aria-label="Remove workspace environment variable" title="Remove">&#215;</button></div>`).join("");
  }
}

async function loadRuns(){
  if(state.view!=="runs")return;
  const params=new URLSearchParams({
    page:String(state.runFilters.page),page_size:String(state.runFilters.pageSize),
    status:state.runFilters.status==="all"?"":state.runFilters.status,
    trigger:state.runFilters.trigger,
    session:state.runFilters.session,task:state.runFilters.task,
  });
  const [runs,eventResponse]=await Promise.all([
    requestJson(`/api/task-runs?${addNodeQuery(params)}`),
    requestJson("/api/events?page=1&page_size=20"),
  ]);
  if(runs.ok){ state.runPage={page:runs.data.page,page_size:runs.data.page_size,total:runs.data.total,total_pages:runs.data.total_pages}; state.runs=runs.data.items||[]; }
  else throw new Error(runs.message);
  if(eventResponse.ok){state.eventPage=eventResponse.data;state.events=eventResponse.data.items||[];} else throw new Error(eventResponse.message);
  renderRuns(); updateMeta();
}

function renderRuns(){
  $("#runs-body").innerHTML=state.runs.length?state.runs.map((run)=>`<tr><td>${escapeHtml(run.task)}</td><td>${escapeHtml(run.session)}</td><td><span class="status ${escapeAttr(run.status)}">${escapeHtml(run.status)}</span></td><td>${escapeHtml(run.trigger)}</td><td>${escapeHtml(new Date(run.started_at_ms).toLocaleString())}</td><td>${run.duration_ms==null?"":escapeHtml(run.duration_ms+" ms")}</td><td>${escapeHtml(run.error_message||run.command||"")}</td></tr>`).join(""):'<tr class="empty-row"><td colspan="7">No task runs recorded.</td></tr>';
  $("#events-body").innerHTML=state.events.length?state.events.map((event)=>`<tr><td>${escapeHtml(new Date(event.timestamp_ms).toLocaleString())}</td><td>${escapeHtml(event.category)}</td><td>${escapeHtml(event.message)}</td></tr>`).join(""):'<tr class="empty-row"><td colspan="3">No events recorded.</td></tr>';
  $("#runs-page-label").textContent=`Page ${state.runPage.page||1} of ${state.runPage.total_pages||0}`;
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
  state.configTasks.push({ _key: globalThis.crypto?.randomUUID?.() || `task-${Date.now()}-${Math.random()}`, label: "new-task", command: "", args: [], cwd: ".", envRows: [], shell: true, auto_start: false, stop_timeout_ms: 3000, clear_logs_on_restart: false, schedule: null, origin: { imported: false, has_yaml_override: false } });
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
    return { label, command, args: [...task.args], cwd: task.cwd.trim() || ".", env, shell: task.shell, auto_start: task.auto_start, stop_timeout_ms: timeout, clear_logs_on_restart: Boolean(task.clear_logs_on_restart), schedule: task.schedule ? String(task.schedule).trim() : null };
  });
}

async function validateWorkspaceEnv(){
  const values={};
  state.configWorkspaceEnvRows.forEach((row)=>{const key=row.key.trim();if(!key)throw new Error("Workspace environment key is required");if(Object.hasOwn(values,key))throw new Error(`Duplicate workspace environment key ${key}`);values[key]=row.value;});
  return values;
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
      body: JSON.stringify({ revision: state.config.revision, workspace_env:Object.fromEntries(validateWorkspaceEnv()), tasks }),
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
    state.configWorkspaceEnvRows = Object.entries(response.data.workspace_env || {}).map(([key,value])=>({key,value}));
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
    if (state.view === "settings") loadNodeSettings().catch(() => {});
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
    if (!button || state.suppressTabClick) return;
    markExitSeen(button.dataset.task);
    state.currentTask = button.dataset.task;
    state.renderedTask = null;
    state.metrics = null;
    state.headerSignature = "";
    resetLogCursor();
    renderTabs(orderedTaskLabels(state.snapshot));
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
    if (event.altKey) {
      const direction = event.key === "ArrowRight" ? 1 : -1;
      const target = index + direction;
      if (target < 0 || target >= tabs.length) return;
      const previous = tabs.map((tab) => tab.dataset.task);
      const order = [...previous];
      [order[index], order[target]] = [order[target], order[index]];
      persistWorkspaceOrder(order, previous);
      requestAnimationFrame(() => $(`[data-task="${CSS.escape(state.currentTask)}"]`, $("#tabs"))?.focus());
      return;
    }
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
  $("#refresh-audit").addEventListener("click", loadAudit);
  $("#audit-body").addEventListener("click", (event) => {
    const button = event.target.closest("[data-audit-id]");
    if (button) openAuditDetails(button.dataset.auditId);
  });
  $("#audit-search").addEventListener("input", (event) => { state.auditFilters.q = event.target.value.trim(); scheduleAuditReload(); });
  $("#audit-source").addEventListener("change", (event) => { state.auditFilters.source = event.target.value; scheduleAuditReload(); });
  $("#audit-status").addEventListener("change", (event) => { state.auditFilters.status = event.target.value; scheduleAuditReload(); });
  $("#audit-node").addEventListener("input", (event) => { state.auditFilters.node = event.target.value.trim(); scheduleAuditReload(); });
  $("#audit-session").addEventListener("input", (event) => { state.auditFilters.session = event.target.value.trim(); scheduleAuditReload(); });
  $("#audit-task").addEventListener("input", (event) => { state.auditFilters.task = event.target.value.trim(); scheduleAuditReload(); });
  $("#audit-operation").addEventListener("input", (event) => { state.auditFilters.operation = event.target.value.trim(); scheduleAuditReload(); });
  $("#audit-page-size").addEventListener("change", (event) => { state.auditFilters.pageSize = Number(event.target.value); scheduleAuditReload(); });
  $("#clear-audit-filters").addEventListener("click", () => {
    state.auditFilters = { q: "", source: "all", status: "all", node: "", session: "", task: "", operation: "", page: 1, pageSize: Number($("#audit-page-size").value) || 20 };
    $("#audit-search").value = ""; $("#audit-source").value = "all"; $("#audit-status").value = "all"; $("#audit-node").value = ""; $("#audit-session").value = ""; $("#audit-task").value = ""; $("#audit-operation").value = "";
    loadAudit();
  });
  $("#audit-prev").addEventListener("click", () => { if (state.auditPage.has_previous) { state.auditFilters.page -= 1; loadAudit(); } });
  $("#audit-next").addEventListener("click", () => { if (state.auditPage.has_next) { state.auditFilters.page += 1; loadAudit(); } });
  $("#close-call-dialog").addEventListener("click", closeCallDetails);
  $("#call-dialog").addEventListener("click", (event) => {
    const mode = event.target.closest("[data-call-mode]");
    if (mode) setCallDetailMode(mode.dataset.callMode);
  });
  $("#close-audit-dialog").addEventListener("click", closeAuditDetails);
  $("#audit-dialog").addEventListener("click", (event) => {
    const mode = event.target.closest("[data-audit-mode]");
    if (mode) setAuditDetailMode(mode.dataset.auditMode);
  });
  $("#close-config").addEventListener("click", requestCloseConfig);
  $("#call-dialog").addEventListener("click", (event) => { if (event.target === $("#call-dialog")) closeCallDetails(); });
  $("#call-dialog").addEventListener("close", () => { state.callDetailRequest += 1; state.callDetailId = null; });
  $("#audit-dialog").addEventListener("click", (event) => { if (event.target === $("#audit-dialog")) closeAuditDetails(); });
  $("#audit-dialog").addEventListener("close", () => { state.auditDetailRequest += 1; state.auditDetailId = null; });
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
  bindPointerSorter($("#tabs"), ".tab", null, (order, previous) => {
    if (order.join("\u0000") !== previous.join("\u0000")) persistWorkspaceOrder(order, previous);
  });
  bindPointerSorter($("#config-task-list"), ".config-task-item", "[data-drag-handle]", (order) => {
    const selectedKey = state.configTasks[state.configTaskIndex]?._key;
    const byKey = new Map(state.configTasks.map((task) => [task._key, task]));
    state.configTasks = order.map((key) => byKey.get(key)).filter(Boolean);
    state.configTaskIndex = Math.max(0, state.configTasks.findIndex((task) => task._key === selectedKey));
    state.configDirty = true;
    renderConfigTaskList();
  });
  $("#reload-node-settings").addEventListener("click", loadNodeSettings);
  $("#node-role").addEventListener("change", () => {
    const leader = $("#node-role").value === "leader";
    $("#leader-mode-field").hidden = !leader;
    $("#leader-url-field").hidden = !leader;
  });
  $("#node-token-mode").addEventListener("change", () => {
    $("#token-value-field").hidden = $("#node-token-mode").value !== "set";
  });
  $("#node-settings-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    if (!state.nodeSettings) return;
    const settings = state.nodeSettings.settings;
    const body = {};
    if (!overrideFor("name")) body.name = $("#node-name").value.trim();
    if (!overrideFor("role")) body.role = $("#node-role").value;
    const roleOverridden = Boolean(overrideFor("role"));
    if ($("#node-role").value === "leader") {
      if (!overrideFor("leader_mode")) body.leader_mode = $("#node-leader-mode").value;
      if (!overrideFor("leader_url")) body.leader_url = null;
    } else {
      if (!roleOverridden && !overrideFor("leader_url")) body.leader_url = $("#node-leader-url").value.trim();
    }
    if (roleOverridden) {
      delete body.leader_mode;
      delete body.leader_url;
    }
    if (!overrideFor("bind_host")) body.bind_host = $("#node-bind-host").value.trim();
    if (!overrideFor("web_port")) {
      const port = Number($("#node-web-port").value);
      if (!Number.isInteger(port) || port < 1 || port > 65535) {
        showSettingsMessage("#node-settings-message", "Web port must be 1-65535", "error");
        return;
      }
      body.web_port = port;
    }
    const tokenMode = $("#node-token-mode").value;
    if (tokenMode === "set") {
      const value = $("#node-token-value").value;
      if (!value) {
        showSettingsMessage("#node-settings-message", "Enter a token value or keep the current token", "error");
        return;
      }
      body.enrollment_token = { mode: "set", value };
    } else if (tokenMode === "clear") {
      body.enrollment_token = { mode: "clear" };
    }
    try {
      const node = selectedNode();
      const response = await requestJson(`/api/nodes/${encodeURIComponent(node)}/settings`, {
        method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify(body),
      });
      if (!response.ok) throw new Error(response.message);
      state.nodeSettings = response.data;
      renderNodeSettings();
      showSettingsMessage(
        "#node-settings-message",
        response.data.restart_required ? "Settings saved. Restart the daemon to apply the new configuration." : "Settings saved.",
        response.data.restart_required ? "warning" : "success",
      );
    } catch (error) {
      showSettingsMessage("#node-settings-message", error.message || "Unable to save node settings", "error");
    }
  });
  $("#alias-session").addEventListener("change", renderAliasForm);
  $("#alias-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const session = $("#alias-session").value;
    if (!session) return;
    try {
      const response = await requestJson(`/api/workspaces/${encodeURIComponent(session)}/alias?${addNodeQuery()}`, {
        method: "PUT", headers: { "content-type": "application/json" },
        body: JSON.stringify({ alias: $("#alias-value").value.trim() }),
      });
      if (!response.ok) throw new Error(response.message);
      await loadWorkspaces();
      showSettingsMessage("#alias-message", "Alias saved", "success");
    } catch (error) {
      showSettingsMessage("#alias-message", error.message || "Unable to save alias", "error");
    }
  });
  $("#clear-alias").addEventListener("click", async () => {
    $("#alias-value").value = "";
    $("#alias-form").requestSubmit();
  });
  $("#new-workflow")?.addEventListener("click", () => beginWorkflowEditor());
  $("#refresh-workflows")?.addEventListener("click", () => loadWorkflowGroups().catch((error) => showToast(error.message || "Unable to load workflows")));
  $("#workflow-groups")?.addEventListener("click", (event) => {
    const open = event.target.closest("[data-workflow-open]");
    if (open) {
      openWorkflowMember(open.dataset.node, open.dataset.session).catch((error) => showToast(error.message || "Unable to open workspace"));
      return;
    }
    const edit = event.target.closest("[data-workflow-edit]");
    if (edit) {
      const group = state.workflowGroups.find((item) => item.id === edit.dataset.workflowEdit);
      if (group) beginWorkflowEditor(group);
      return;
    }
    const remove = event.target.closest("[data-workflow-delete]");
    if (remove) {
      deleteWorkflow(remove.dataset.workflowDelete);
      return;
    }
    const action = event.target.closest("[data-workflow-action]");
    if (action) runWorkflowAction(action.dataset.workflowId, action.dataset.workflowAction);
  });
  $("#ungrouped-workspaces")?.addEventListener("click", (event) => {
    const open = event.target.closest("[data-workflow-open]");
    if (open) openWorkflowMember(open.dataset.node, open.dataset.session).catch((error) => showToast(error.message || "Unable to open workspace"));
  });
  $("#add-workflow-member")?.addEventListener("click", addWorkflowMember);
  $("#workflow-members")?.addEventListener("change", handleWorkflowMemberChange);
  $("#workflow-members")?.addEventListener("click", handleWorkflowMemberButton);
  $("#save-workflow")?.addEventListener("click", saveWorkflow);
  $("#cancel-workflow")?.addEventListener("click", cancelWorkflowEditor);
  $("#new-board")?.addEventListener("click", () => beginBoardEditor());
  $("#refresh-boards")?.addEventListener("click", () => loadBoards().catch((error) => showToast(error.message || "Unable to load boards")));
  $("#boards-view")?.addEventListener("click", (event) => {
    const remove = event.target.closest("[data-board-remove]");
    if (remove) {
      removeBoardCard(remove.dataset.boardCard);
      return;
    }
    const pin = event.target.closest("[data-board-pin]");
    if (pin) {
      toggleBoardCardPin(pin.dataset.boardCard);
      return;
    }
    const action = event.target.closest("[data-board-action]");
    if (action) {
      runBoardCardAction(action.dataset.boardCard, action.dataset.boardAction, action);
      return;
    }
    const mode = event.target.closest("[data-board-mode]");
    if (mode) {
      setBoardCardMode(mode.dataset.boardCard, mode.dataset.boardMode);
      return;
    }
    const edit = event.target.closest("[data-board-edit]");
    if (edit) {
      const board = state.boards.find((item) => item.id === edit.dataset.boardEdit);
      if (board) beginBoardEditor(board);
      return;
    }
    const removeBoardButton = event.target.closest("[data-board-delete]");
    if (removeBoardButton) {
      deleteBoard(removeBoardButton.dataset.boardDelete);
      return;
    }
    const open = event.target.closest("[data-board-open]");
    if (open) openWorkflowMember(open.dataset.node, open.dataset.session).catch((error) => showToast(error.message || "Unable to open workspace"));
  });
  $("#add-board-card")?.addEventListener("click", addBoardCard);
  $("#board-cards")?.addEventListener("change", handleBoardCardChange);
  $("#board-cards")?.addEventListener("click", handleBoardCardEditorButton);
  $("#save-board")?.addEventListener("click", saveBoard);
  $("#cancel-board")?.addEventListener("click", cancelBoardEditor);
  $("#reload-service").addEventListener("click", loadServiceStatus);
  $("#service-scope").addEventListener("change", () => {
    $("#service-home-field").hidden = $("#service-scope").value !== "system";
    loadServiceStatus();
  });
  $("#service-form").addEventListener("submit", (event) => event.preventDefault());
  $$("[data-service]").forEach((button) => {
    button.addEventListener("click", async () => {
      const scope = $("#service-scope").value;
      const body = { action: button.dataset.service, scope };
      if (scope === "system") body.home = $("#service-home").value.trim() || null;
      showSettingsMessage("#service-message", "Working…");
      try {
        const response = await requestJson("/api/nodes/self/service", {
          method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body),
        });
        if (!response.ok) throw new Error(response.message);
        state.serviceStatus = response.data;
        renderServiceStatus();
        showSettingsMessage("#service-message", "Service operation completed", "success");
      } catch (error) {
        showSettingsMessage("#service-message", error.message || "Service operation failed", "error");
      }
    });
  });
  $("#config-form").addEventListener("submit", (event) => { event.preventDefault(); saveConfig(); });
  $("#refresh-runs")?.addEventListener("click", () => loadRuns().catch((e)=>showToast(e.message||"Unable to load runs")));
  ["run-session","run-task","run-status","run-trigger","run-page-size"].forEach((id)=>{
    $(`#${id}`)?.addEventListener("change",(event)=>{
      const target=event.target;
      if(id==="run-session")state.runFilters.session=target.value;
      else if(id==="run-task")state.runFilters.task=target.value;
      else if(id==="run-status")state.runFilters.status=target.value;
      else if(id==="run-trigger")state.runFilters.trigger=target.value;
      else if(id==="run-page-size")state.runFilters.pageSize=Number(target.value);
      state.runFilters.page=1;loadRuns().catch(()=>{});
    });
  });
  $("#runs-prev")?.addEventListener("click",()=>{if(state.runPage.page>1){state.runFilters.page-=1;loadRuns().catch(()=>{});}});
  $("#runs-next")?.addEventListener("click",()=>{if(state.runPage.has_next??(state.runFilters.page<state.runPage.total_pages)){state.runFilters.page+=1;loadRuns().catch(()=>{});}});
  $("#config-form").addEventListener("input", handleConfigInput);
  $("#config-form").addEventListener("change", handleConfigInput);
  $("#config-form").addEventListener("click", handleConfigButton);
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
  const target = event.target;
  if(target.dataset.workspaceKey!=null){state.configWorkspaceEnvRows[Number(target.dataset.workspaceKey)].key=target.value;}
  else if(target.dataset.workspaceValue!=null){state.configWorkspaceEnvRows[Number(target.dataset.workspaceValue)].value=target.value;}
  else { const task=state.configTasks[state.configTaskIndex]; if(!task)return;
  if (target.dataset.field) {
    const field = target.dataset.field;
    task[field] = target.type === "checkbox" ? target.checked : field === "stop_timeout_ms" ? Number(target.value) : target.value;
    if (field === "label") renderConfigTaskList();
  }
  if (target.dataset.arg != null) task.args[Number(target.dataset.arg)] = target.value;
  if (target.dataset.envKey != null) task.envRows[Number(target.dataset.envKey)].key = target.value;
  if (target.dataset.envValue != null) task.envRows[Number(target.dataset.envValue)].value = target.value;
  }
  state.configDirty = true;
}

function handleConfigButton(event) {
  const task = state.configTasks[state.configTaskIndex];
  const button = event.target.closest("button");
  if (!button) return;
  if(button.dataset.addWorkspace!=null){state.configWorkspaceEnvRows.push({key:"",value:""});renderConfigWorkspace();}
  else if(button.dataset.removeWorkspace!=null){state.configWorkspaceEnvRows.splice(Number(button.dataset.removeWorkspace),1);renderConfigWorkspace();}
  else if (!task) return;
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
  } else if (state.view === "audit") {
    loadAudit();
  } else if (state.view === "runs") {
    loadRuns().catch(() => {});
  } else if (state.view === "settings") {
    loadWorkspaces().catch(() => {});
  } else if (state.view === "workflows") {
    loadWorkflowGroups().catch(() => {});
  } else if (state.view === "boards") {
    loadBoards().catch(() => {});
  }
}

applySavedPreferences();
bindEvents();
updateEndpoint();
setView("tasks");
loadNodes();
setInterval(tick, 1000);
setInterval(loadNodes, 5000);
