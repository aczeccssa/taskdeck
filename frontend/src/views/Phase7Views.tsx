import {useEffect, useLayoutEffect, useRef, useState, type FormEvent, type KeyboardEvent, type ReactNode} from "react";
import {LegacyApiAdapter} from "../compat/api";
import {showToast} from "../lib/toast";
import type {EditableTask, LogLine, NodeSummary, SessionSnapshot, TaskMetricsSnapshot, WorkspaceSummary} from "../domain/models";
import {
    chartGeometry, formatBytes, formatRuntime, highlightSegments, logMatches, matchOffsets, normalizeMatchIndex,
    orderedTaskLabels, reconcileLogs, taskStateDot, taskStatusAllowsAction, taskToDraft,
    validateConfigTasks, validateWorkspaceEnv, type ConfigTaskDraft,
} from "./phase7Helpers";
import {
    configSaveFailure, decodeConfig, decodeConfigOrFailure, decodeLogs, decodeMetrics, decodeNodes, decodeSnapshot, decodeStrings,
    decodeWorkspaces,
} from "./phase7Api";
import "../legacy/styles.css";

const api = new LegacyApiAdapter();
const LOG_TAILS = [100, 500, 1000, 5000];
type WorkspaceMode = "log" | "split" | "monitor";
type SeenExits = Record<string, number>;

function exitKey(node: string, project: string, task: string): string {
    return `${node}\u0000${project}\u0000${task}`;
}

function setConnectionState(connected: boolean): void {
    const connection = document.getElementById("connection-state");
    if (!connection) return;
    connection.classList.toggle("offline", !connected);
    const label = connection.querySelector("span");
    if (label) label.textContent = connected ? "Daemon connected" : "Daemon unavailable";
}

function setMetaText(text: string): void {
    const meta = document.getElementById("meta");
    if (meta) meta.textContent = text;
}

function setSelectOptions(select: HTMLSelectElement | null, options: Array<{value: string; label: string}>, selected: string, emptyLabel: string): void {
    if (!select) return;
    select.replaceChildren(...options.map((option) => {
        const element = document.createElement("option");
        element.value = option.value;
        element.textContent = option.label;
        return element;
    }));
    select.value = options.some((option) => option.value === selected) ? selected : options[0]?.value ?? "";
}

function sessionLabel(workspaces: readonly WorkspaceSummary[], session: string): string {
    const workspace = workspaces.find((item) => item.session === session);
    return workspace?.alias ? `${workspace.alias} · ${session}` : session;
}

function sessionOptions(workspaces: readonly WorkspaceSummary[], sessions: readonly string[], emptyLabel: string): Array<{value: string; label: string}> {
    return sessions.length ? sessions.map((session) => ({value: session, label: sessionLabel(workspaces, session)})) : [{value: "", label: emptyLabel}];
}

function actionIcon(action: "start" | "pause" | "restart" | "stop"): ReactNode {
    const paths: Record<typeof action, string> = {
        start: "m8 5 11 7-11 7z",
        pause: "M8 5v14M16 5v14",
        restart: "M19 8V4m0 0h-4m4 0-3 3a7 7 0 1 0 2 8",
        stop: "",
    };
    if (action === "stop") return <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>;
    return <svg viewBox="0 0 24 24" aria-hidden="true"><path d={paths[action]}/></svg>;
}

const settingsIcon = <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M19 12a7 7 0 0 0-.1-1l2-1.5-2-3.4-2.4 1A7 7 0 0 0 15 6l-.3-2.6h-4L10.4 6A7 7 0 0 0 9 7.1l-2.4-1-2 3.4 2 1.5a7 7 0 0 0 0 2l-2 1.5 2 3.4 2.4-1A7 7 0 0 0 10.4 18l.3 2.6h4L15 18a7 7 0 0 0 1.5-1.1l2.4 1 2-3.4-2-1.5a7 7 0 0 0 .1-1z"/></svg>;
const trashIcon = <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 7h16M9 7V4h6v3m3 0-1 13H7L6 7m4 4v5m4-5v5"/></svg>;
const gripIcon = <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="9" cy="7" r="1"/><circle cx="15" cy="7" r="1"/><circle cx="9" cy="12" r="1"/><circle cx="15" cy="12" r="1"/><circle cx="9" cy="17" r="1"/><circle cx="15" cy="17" r="1"/></svg>;

function requireData<T>(response: {ok: boolean; message: string; data?: T}): T {
    if (!response.ok) throw new Error(response.message);
    if (response.data === undefined) throw new Error("Response data is missing");
    return response.data;
}

function decodeAccepted(value: unknown): {accepted?: boolean} {
    return typeof value === "object" && value !== null && !Array.isArray(value) && (value as Record<string, unknown>).accepted === true ? {accepted: true} : {};
}

type SortDrag = {
    pointerId: number;
    key: string;
    element: HTMLElement;
    startX: number;
    startY: number;
    active: boolean;
    original: string[];
};

function endpointElement(endpoint: {bind_host: string; port: number; protocol: string; source: string; state: string}, index: number): ReactNode {
    const label = `${endpoint.bind_host}:${endpoint.port}`;
    const isLink = endpoint.state === "listening" && ["http", "https"].includes(endpoint.protocol);
    if (isLink) return <a className="endpoint-chip listening" key={`${label}-${index}`} href={`${endpoint.protocol}://${endpoint.bind_host}:${endpoint.port}`} target="_blank" rel="noreferrer">{label}</a>;
    return <span className={`endpoint-chip ${endpoint.state || ""}`} key={`${label}-${index}`} title={`${endpoint.source || "unknown"} · ${endpoint.state || "unknown"}`}>{label}</span>;
}

function MetricChart({samples, metric, className, formatter, restartMarkers, title}: {
    samples: TaskMetricsSnapshot["samples"]; metric: "cpu_percent" | "memory_bytes"; className: string;
    formatter: (value: number) => string; restartMarkers: number[]; title: string;
}): React.JSX.Element {
    const geometry = chartGeometry(samples, metric, restartMarkers);
    const current = Number(samples.at(-1)?.[metric] || 0);
    return <div className="metric-chart">
        <div className="chart-title"><span>{title}</span><strong>{formatter(current)}</strong></div>
        <svg viewBox="0 0 300 72" preserveAspectRatio="none" aria-hidden="true">
            <path className="chart-grid" d="M0 70H300M0 38H300M0 6H300"/>
            {geometry.markers.map((x, index) => <path className="chart-restart" key={index} d={`M${x} 4V70`}/>)}
            {geometry.points ? <polyline className={`chart-line ${className}`} points={geometry.points}/> : null}
        </svg>
    </div>;
}

function LogsPanel({lines, query, matchIndex, follow, scrollToMatch, logContext, logGeneration, tail, onFollowChange, onScrolledToMatch, onSearchChange, onSearchSubmit, onTailChange}: {
    lines: LogLine[]; query: string; matchIndex: number; follow: boolean; scrollToMatch: boolean;
    logContext: string; logGeneration: number | null; tail: number;
    onFollowChange: (follow: boolean) => void; onScrolledToMatch: () => void;
    onSearchChange: (value: string) => void; onSearchSubmit: (direction: -1 | 1) => void; onTailChange: (value: number) => void;
}): React.JSX.Element {
    const containerRef = useRef<HTMLDivElement>(null);
    const countRef = useRef<HTMLSpanElement>(null);
    const matchCountRef = useRef<HTMLSpanElement>(null);
    const previousTop = useRef(0);
    const suppressScroll = useRef(false);
    const trimmed = query.trim();
    const {matches, offsetsByLine} = logMatches(lines, trimmed);
    const normalizedIndex = normalizeMatchIndex(matchIndex, matches.length);
    const currentMatch = matches[normalizedIndex];

    const buildRow = (line: LogLine, lineIndex: number): HTMLDivElement => {
        const offsets = offsetsByLine[lineIndex] ?? [];
        const match = offsets.length > 0;
        const current = match && currentMatch?.lineIndex === lineIndex;
        const row = document.createElement("div");
        row.className = `log-row ${line.stream}${match ? " match" : ""}${current ? " current-match" : ""}`;
        row.dataset.lineIndex = String(lineIndex);
        row.dataset.seq = String(line.seq);
        const number = document.createElement("span");
        number.className = "log-number";
        number.textContent = String(line.seq);
        const text = document.createElement("span");
        text.className = "log-text";
        for (const segment of highlightSegments(line.text, trimmed, current ? currentMatch.occurrenceIndex : -1)) {
            const node = document.createElement(segment.highlighted ? "mark" : "span");
            if (segment.highlighted && segment.current) node.className = "current-hit";
            node.textContent = segment.text;
            text.appendChild(node);
        }
        row.append(number, text);
        return row;
    };

    useLayoutEffect(() => {
        const container = containerRef.current;
        if (!container) return;
        previousTop.current = container.scrollTop;
        const rows = Array.from(container.querySelectorAll<HTMLDivElement>(".log-row"));
        const existingLast = Number(rows.at(-1)?.dataset.seq);
        const firstSeq = lines[0]?.seq;
        const lastSeq = lines.at(-1)?.seq;
        const canPatch = !trimmed
            && container.dataset.logContext === logContext
            && container.dataset.logGeneration === String(logGeneration ?? "")
            && container.dataset.search === ""
            && rows.length > 0
            && Number.isFinite(existingLast)
            && lines.some((line) => line.seq === existingLast);
        let removedHeight = 0;
        if (canPatch) {
            rows.forEach((row) => {
                if (Number(row.dataset.seq) < (firstSeq ?? 0)) {
                    removedHeight += row.getBoundingClientRect().height;
                    row.remove();
                }
            });
            lines.forEach((line, lineIndex) => {
                if (line.seq > existingLast) container.appendChild(buildRow(line, lineIndex));
            });
        } else {
            container.replaceChildren(...(lines.length ? lines.map((line, lineIndex) => buildRow(line, lineIndex)) : (() => {
                const empty = document.createElement("div");
                empty.className = "log-empty";
                empty.textContent = "No output yet.";
                return [empty];
            })()));
        }
        container.dataset.logContext = logContext;
        container.dataset.logGeneration = String(logGeneration ?? "");
        container.dataset.search = trimmed;
        container.dataset.firstSeq = firstSeq == null ? "" : String(firstSeq);
        container.dataset.lastSeq = lastSeq == null ? "" : String(lastSeq);
        if (countRef.current) countRef.current.textContent = `${lines.length} line${lines.length === 1 ? "" : "s"}`;
        if (matchCountRef.current) matchCountRef.current.textContent = matches.length ? `${normalizedIndex + 1} / ${matches.length}` : "0 / 0";
        suppressScroll.current = true;
        if (scrollToMatch && currentMatch) {
            const target = container.querySelector<HTMLDivElement>(`[data-line-index="${currentMatch.lineIndex}"]`);
            target?.scrollIntoView({block: "center"});
            onScrolledToMatch();
        } else if (follow) {
            container.scrollTop = container.scrollHeight;
        } else {
            container.scrollTop = Math.max(0, previousTop.current - removedHeight);
        }
        window.requestAnimationFrame(() => { suppressScroll.current = false; });
    });

    return <section className="log-panel" id="log-panel" aria-label="Task logs">
        <div className="log-toolbar">
            <select id="log-tail" aria-label="Maximum log lines" title="Maximum log lines" value={String(tail)} onChange={(event) => onTailChange(Number(event.target.value))}>
                {LOG_TAILS.map((value) => <option key={value} value={value}>{value} lines</option>)}
            </select>
            <div className="log-search">
                <input id="log-search" type="search" placeholder="Search output" aria-label="Search output" autoComplete="off" value={query} onChange={(event) => onSearchChange(event.target.value)} onKeyDown={(event) => { if (event.key !== "Enter") return; event.preventDefault(); onSearchSubmit(event.shiftKey ? -1 : 1); }}/>
                <span id="log-match-count" ref={matchCountRef}>0 / 0</span>
            </div>
            <button className="icon-button" type="button" data-log="previous" aria-label="Previous match" title="Previous match">↑</button>
            <button className="icon-button" type="button" data-log="next" aria-label="Next match" title="Next match">↓</button>
            <button className="icon-button" type="button" data-log="clear-search" aria-label="Clear search" title="Clear search">×</button>
            <button className="icon-button" type="button" data-log="top" aria-label="Go to top" title="Go to top">⇧</button>
            <button className="icon-button" type="button" data-log="bottom" aria-label="Go to bottom" title="Go to bottom">⇩</button>
            <button className="button compact" id="follow-button" type="button" data-log="follow">{follow ? "Unfocus" : "Focus"}</button>
            <button className="icon-button" type="button" data-log="fullscreen" aria-label="Full screen logs" title="Full screen logs">⛶</button>
            <button className="icon-button danger-icon" type="button" data-log="clear-history" aria-label="Clear logs and performance history" title="Clear logs and performance history">{trashIcon}</button>
            <span className="log-line-count" id="log-line-count" ref={countRef}>0 lines</span>
        </div>
        <div className="logs" id="logs" tabIndex={0} ref={containerRef} onScroll={(event) => {
            if (suppressScroll.current) return;
            const container = event.currentTarget;
            if (container.scrollHeight - container.scrollTop - container.clientHeight >= 8) onFollowChange(false);
        }}/>
    </section>;
}

function MetricsPanel({metrics}: {metrics: TaskMetricsSnapshot | null}): React.JSX.Element {
    const bodyRef = useRef<HTMLDivElement>(null);
    const scroll = useRef({top: 0, left: 0});
    useLayoutEffect(() => {
        const body = bodyRef.current;
        if (!body) return;
        body.scrollTop = scroll.current.top;
        body.scrollLeft = scroll.current.left;
    });
    if (!metrics) return <aside className="monitor-panel" id="monitor-panel" aria-label="Task performance">
        <header className="monitor-header"><strong>Performance</strong><span id="monitor-state">Waiting</span></header>
        <div className="monitor-body" id="monitor-body" ref={bodyRef}><div className="monitor-empty">No samples</div></div>
    </aside>;
    const current = metrics.current;
    const samples = metrics.samples;
    const restartMarkers = metrics.restart_markers_ms;
    return <aside className="monitor-panel" id="monitor-panel" aria-label="Task performance">
        <header className="monitor-header"><strong>Performance</strong><span id="monitor-state">{metrics.running ? "Live - 1s" : "Stopped"}</span></header>
        <div className="monitor-body" id="monitor-body" ref={bodyRef} onScroll={(event) => { scroll.current = {top: event.currentTarget.scrollTop, left: event.currentTarget.scrollLeft}; }}>
            <div className="metric-summary">
                <div><span>CPU</span><strong>{Number(current.cpu_percent || 0).toFixed(1)}%</strong></div>
                <div><span>RSS</span><strong>{formatBytes(current.memory_bytes)}</strong></div>
                <div><span>Processes</span><strong>{current.process_count || 0}</strong></div>
            </div>
            <div className="metric-charts">
                <MetricChart samples={samples} metric="cpu_percent" className="" formatter={(value) => `${value.toFixed(1)}%`} restartMarkers={restartMarkers} title="CPU"/>
                <MetricChart samples={samples} metric="memory_bytes" className="memory" formatter={formatBytes} restartMarkers={restartMarkers} title="Memory"/>
            </div>
            <div className="process-table-wrap">
                {metrics.processes.length ? <table className="processes">
                    <thead><tr><th>PID</th><th>PPID</th><th>Name</th><th>CPU</th><th>RSS</th><th>Status</th><th>Runtime</th></tr></thead>
                    <tbody>{metrics.processes.map((process) => <tr key={`${process.pid}-${process.name}`}>
                        <td>{process.pid}</td><td>{process.ppid ?? "-"}</td>
                        <td className="name" title={process.name}>{process.name}</td>
                        <td>{Number(process.cpu_percent || 0).toFixed(1)}%</td>
                        <td>{formatBytes(process.memory_bytes)}</td>
                        <td>{process.status}</td><td>{formatRuntime(process.run_time_seconds)}</td>
                    </tr>)}</tbody>
                </table> : <div className="monitor-empty">No running processes</div>}
            </div>
        </div>
    </aside>;
}

type ConfigSaveState = {kind?: "stale_revision" | "reconciliation_error"; saved?: boolean; current_revision?: string};

function ConfigDialog({open, setOpen, node, session, currentTask, nodes, onSnapshotRefresh, registerGuard}: {
    open: boolean; setOpen: (open: boolean) => void; node: string; session: string; currentTask: string | null; nodes: NodeSummary[];
    onSnapshotRefresh: () => void; registerGuard: (guard: () => boolean) => void;
}): React.JSX.Element {
    const dialogRef = useRef<HTMLDialogElement>(null);
    const requestRef = useRef(0);
    const [config, setConfig] = useState<{session: string; project: string; source: string; revision: string; node: string} | null>(null);
    const [tasks, setTasks] = useState<ConfigTaskDraft[]>([]);
    const [workspaceEnvRows, setWorkspaceEnvRows] = useState<Array<{key: string; value: string}>>([]);
    const [taskIndex, setTaskIndex] = useState(0);
    const [dirty, setDirty] = useState(false);
    const [saving, setSaving] = useState(false);
    const [message, setMessage] = useState("");
    const [messageError, setMessageError] = useState(false);
    const [reloadVisible, setReloadVisible] = useState(false);
    const sortDrag = useRef<SortDrag | null>(null);
    const configListRef = useRef<HTMLElement>(null);

    const showMessage = (value: string, error = false, reload = false): void => {
        setMessage(value);
        setMessageError(error);
        setReloadVisible(reload);
    };
    const requestClose = (): boolean => {
        const dialog = dialogRef.current;
        if (!dialog?.open) return true;
        if (saving) {
            showMessage("Wait for the current save to finish.", true);
            return false;
        }
        if (dirty && !window.confirm("Discard unsaved configuration changes?")) return false;
        requestRef.current += 1;
        setDirty(false);
        setOpen(false);
        dialog.close();
        return true;
    };
    useEffect(() => { registerGuard(requestClose); });
    useEffect(() => {
        const container = configListRef.current;
        if (!container) return;
        const readOrder = (): string[] => Array.from(container.querySelectorAll<HTMLElement>(".config-task-item")).map((element) => element.dataset.orderKey ?? "");
        const finish = (event: PointerEvent): void => {
            const drag = sortDrag.current;
            if (!drag || drag.pointerId !== event.pointerId) return;
            drag.element.classList.remove("dragging");
            if (drag.active) {
                const order = readOrder();
                if (order.join("\u0000") !== drag.original.join("\u0000")) {
                    const selectedKey = tasks[taskIndex]?._key;
                    const byKey = new Map(tasks.map((task) => [task._key, task]));
                    const next = order.map((key) => byKey.get(key)).filter((task): task is ConfigTaskDraft => Boolean(task));
                    setTasks(next);
                    setDirty(true);
                    if (selectedKey) setTaskIndex(Math.max(0, next.findIndex((task) => task._key === selectedKey)));
                }
            }
            sortDrag.current = null;
        };
        const onPointerDown = (event: PointerEvent): void => {
            if (event.button !== 0) return;
            const target = event.target instanceof Element ? event.target : null;
            const handle = target?.closest<HTMLElement>("[data-drag-handle]");
            if (!handle) return;
            const item = target?.closest<HTMLElement>(".config-task-item");
            const key = item?.dataset.orderKey;
            if (!item || !key) return;
            item.setPointerCapture?.(event.pointerId);
            item.classList.add("dragging");
            sortDrag.current = {pointerId: event.pointerId, key, element: item, startX: event.clientX, startY: event.clientY, active: false, original: readOrder()};
        };
        const onPointerMove = (event: PointerEvent): void => {
            const drag = sortDrag.current;
            if (!drag || drag.pointerId !== event.pointerId) return;
            if (!drag.active && Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) < 6) return;
            event.preventDefault();
            drag.active = true;
            drag.element.classList.add("dragging");
            const target = document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>(".config-task-item");
            const targetKey = target?.dataset.orderKey;
            if (!targetKey || targetKey === drag.key) return;
            const rect = target?.getBoundingClientRect();
            if (!rect) return;
            const before = event.clientY < rect.top + rect.height / 2;
            container.insertBefore(drag.element, before ? target : target.nextSibling);
        };
        container.addEventListener("pointerdown", onPointerDown);
        container.addEventListener("pointermove", onPointerMove);
        container.addEventListener("pointerup", finish);
        container.addEventListener("pointercancel", finish);
        return () => {
            container.removeEventListener("pointerdown", onPointerDown);
            container.removeEventListener("pointermove", onPointerMove);
            container.removeEventListener("pointerup", finish);
            container.removeEventListener("pointercancel", finish);
        };
    }, [tasks, taskIndex]);

    useEffect(() => {
        const dialog = dialogRef.current;
        if (!dialog) return;
        if (open && !dialog.open) dialog.showModal();
        if (!open && dialog.open) dialog.close();
    }, [open]);
    const loadConfig = async (confirmDiscard = false): Promise<void> => {
        if (saving) return;
        if (confirmDiscard && dirty && !window.confirm("Discard unsaved configuration changes and reload?")) return;
        const requestId = requestRef.current + 1;
        requestRef.current = requestId;
        showMessage("Loading configuration...");
        try {
            const response = await api.request(`/api/sessions/${encodeURIComponent(session)}/config?${new URLSearchParams({node})}`, decodeConfig);
            if (requestId !== requestRef.current || !dialogRef.current?.open || session !== dialogRef.current.dataset.session) return;
            const data = requireData(response);
            setConfig({session: data.session, project: data.project, source: data.source, revision: data.revision, node});
            setTasks(data.tasks.map((task) => taskToDraft(task)));
            setWorkspaceEnvRows(Object.entries(data.workspace_env).map(([key, value]) => ({key, value})));
            const currentTask = dialogRef.current.dataset.currentTask;
            const index = data.tasks.findIndex((task: EditableTask) => task.label === currentTask);
            setTaskIndex(Math.max(0, index));
            setDirty(false);
            showMessage("");
            setMessageError(false);
            setReloadVisible(false);
        } catch (error) {
            if (requestId === requestRef.current) showMessage(error instanceof Error ? error.message : "Unable to load configuration", true);
        }
    };
    useEffect(() => {
        const dialog = dialogRef.current;
        if (!dialog) return;
        dialog.dataset.session = session;
        dialog.dataset.currentTask = currentTask ?? "";
        if (open && session) void loadConfig(false);
    }, [open, session, node]);

    const addTask = (): void => {
        const nextTasks = [...tasks, taskToDraft({label: "new-task", command: "", shell: true, auto_start: false, stop_timeout_ms: 3000, origin: {imported: false, has_yaml_override: false}})];
        setTasks(nextTasks);
        setTaskIndex(nextTasks.length - 1);
        setDirty(true);
    };
    const updateTask = (index: number, update: (task: ConfigTaskDraft) => ConfigTaskDraft): void => {
        setTasks((current) => current.map((task, taskPosition) => taskPosition === index ? update(task) : task));
        setDirty(true);
    };
    const save = async (event: FormEvent<HTMLFormElement>): Promise<void> => {
        event.preventDefault();
        if (!config || saving) return;
        let validated;
        let workspaceEnv;
        try {
            validated = validateConfigTasks(tasks);
            workspaceEnv = validateWorkspaceEnv(workspaceEnvRows);
        } catch (error) {
            showMessage(error instanceof Error ? error.message : "Unable to save configuration", true);
            return;
        }
        setSaving(true);
        showMessage("");
        setMessageError(false);
        setReloadVisible(false);
        try {
            if (session !== config.session || node !== config.node) throw new Error("The selected node or session changed. Reopen configuration before saving.");
            const response = await api.request(`/api/sessions/${encodeURIComponent(session)}/config?${new URLSearchParams({node})}`, decodeConfigOrFailure, {
                method: "PUT",
                headers: {"content-type": "application/json"},
                body: JSON.stringify({revision: config.revision, workspace_env: workspaceEnv, tasks: validated}),
            });
            if (!response.ok) {
                const failure = configSaveFailure(response.data);
                if (failure?.kind === "stale_revision") {
                    showMessage("The file changed outside Taskdeck. Reload before applying again.", true, true);
                    return;
                }
                if (failure?.kind === "reconciliation_error" && failure.saved) {
                    setConfig({...config, revision: failure.current_revision || config.revision});
                    setDirty(false);
                    showMessage("Saved to taskdeck.yaml, but one or more live sessions could not reconcile. Retry after checking the affected tasks.", true, true);
                    onSnapshotRefresh();
                    return;
                }
                throw new Error(response.message);
            }
            const data = requireData(response);
            if (!data || !("session" in data)) throw new Error(response.message);
            setConfig({...config, revision: data.revision});
            setTasks(data.tasks.map((task) => taskToDraft(task)));
            setWorkspaceEnvRows(Object.entries(data.workspace_env).map(([key, value]) => ({key, value})));
            setDirty(false);
            showToast("Configuration applied");
            setOpen(false);
            dialogRef.current?.close();
            onSnapshotRefresh();
        } catch (error) {
            showMessage(error instanceof Error ? error.message : "Unable to save configuration", true);
        } finally {
            setSaving(false);
        }
    };
    const nodeSummary = nodes.find((candidate) => candidate.id === config?.node);
    const selectedTask = tasks[taskIndex];
    return <dialog className="drawer config-drawer" id="config-dialog" aria-labelledby="config-dialog-title" ref={dialogRef}
        onCancel={(event) => { event.preventDefault(); requestClose(); }}
        onClick={(event) => { if (event.target === dialogRef.current) requestClose(); }}
        onClose={() => setOpen(false)}>
        <div className="drawer-shell">
            <header className="drawer-header">
                <div><span>Registered configuration</span><h2 id="config-dialog-title">Edit tasks</h2></div>
                <button className="icon-button" id="close-config" type="button" aria-label="Close configuration" title="Close" onClick={() => requestClose()}>
                    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m7 7 10 10M17 7 7 17"/></svg>
                </button>
            </header>
            <div className="config-context" id="config-context">
                <div><span>Node</span><strong>{nodeSummary?.name || config?.node || "Unknown"}</strong></div>
                <div><span>Session</span><strong>{config?.session || ""}</strong></div>
                <div><span>Project</span><strong title={config?.project}>{config?.project || ""}</strong></div>
                <div><span>Revision</span><strong>{(config?.revision || "").slice(0, 12)}</strong></div>
            </div>
            <div className={`config-message${messageError ? " error" : ""}`} id="config-message" role="alert" aria-live="assertive" hidden={!message}>{message}</div>
            <div className="config-layout">
                <aside className="config-task-list" id="config-task-list" ref={configListRef}>
                    <div>
                        {tasks.length ? tasks.map((task, index) => <div className="config-task-item" data-order-key={task._key} key={task._key}>
                            <button className="drag-handle" type="button" data-drag-handle aria-label={`Drag ${task.label || "Untitled task"}`} title="Drag to reorder">
                                {gripIcon}
                            </button>
                            <button className={`config-task-button${index === taskIndex ? " active" : ""}`} type="button" data-config-task={index} onClick={() => setTaskIndex(index)}>
                                <span>{task.label || "Untitled task"}</span>
                                <small>{task.origin.imported ? "VS Code import" : "YAML task"}</small>
                            </button>
                        </div>) : <div className="monitor-empty">No tasks</div>}
                    </div>
                    <button className="button" id="add-config-task" type="button" onClick={addTask}>Add task</button>
                </aside>
                <form className="config-form" id="config-form" onSubmit={save}>
                    <div className="config-form-body">
                        <fieldset className="field">
                            <legend>Workspace environment</legend>
                            <div className="repeater" id="workspace-env-rows">
                                {workspaceEnvRows.map((row, index) => <div className="repeater-row env" key={index}>
                                    <input data-workspace-key={index} value={row.key} placeholder="NAME" onChange={(event) => setWorkspaceEnvRows(workspaceEnvRows.map((item, position) => position === index ? {...item, key: event.target.value} : item))}/>
                                    <input data-workspace-value={index} value={row.value} placeholder="Value" onChange={(event) => setWorkspaceEnvRows(workspaceEnvRows.map((item, position) => position === index ? {...item, value: event.target.value} : item))}/>
                                    <button className="icon-button" type="button" data-remove-workspace={index} aria-label="Remove workspace environment variable" title="Remove" onClick={() => { setWorkspaceEnvRows(workspaceEnvRows.filter((_, position) => position !== index)); setDirty(true); }}>×</button>
                                </div>)}
                            </div>
                            <button className="button compact" type="button" data-add-workspace onClick={() => { setWorkspaceEnvRows([...workspaceEnvRows, {key: "", value: ""}]); setDirty(true); }}>Add workspace variable</button>
                        </fieldset>
                    </div>
                    <div className="config-form-body" id="config-form-body">
                        {selectedTask ? <>
                            <label className="field"><span>Label</span><input data-field="label" value={selectedTask.label} required onChange={(event) => updateTask(taskIndex, (task) => ({...task, label: event.target.value}))}/></label>
                            <label className="field"><span>Command</span><input data-field="command" value={selectedTask.command} required onChange={(event) => updateTask(taskIndex, (task) => ({...task, command: event.target.value}))}/></label>
                            <label className="field"><span>Cron schedule (5 or 6 fields; empty disables)</span><input data-field="schedule" value={selectedTask.schedule || ""} placeholder="*/10 * * * *" onChange={(event) => updateTask(taskIndex, (task) => ({...task, schedule: event.target.value}))}/></label>
                            <div className="field-grid">
                                <label className="field"><span>Working directory</span><input data-field="cwd" value={selectedTask.cwd} onChange={(event) => updateTask(taskIndex, (task) => ({...task, cwd: event.target.value}))}/></label>
                                <label className="field"><span>Stop timeout (ms)</span><input data-field="stop_timeout_ms" type="number" min={1} max={300000} value={selectedTask.stop_timeout_ms} onChange={(event) => updateTask(taskIndex, (task) => ({...task, stop_timeout_ms: Number(event.target.value)}))}/></label>
                            </div>
                            <div className="toggle-row">
                                <label><input data-field="shell" type="checkbox" checked={selectedTask.shell} onChange={(event) => updateTask(taskIndex, (task) => ({...task, shell: event.target.checked}))}/> Run through shell</label>
                                <label><input data-field="auto_start" type="checkbox" checked={selectedTask.auto_start} onChange={(event) => updateTask(taskIndex, (task) => ({...task, auto_start: event.target.checked}))}/> Auto start</label>
                                <label><input data-field="clear_logs_on_restart" type="checkbox" checked={selectedTask.clear_logs_on_restart} onChange={(event) => updateTask(taskIndex, (task) => ({...task, clear_logs_on_restart: event.target.checked}))}/> Clear logs and performance history on restart</label>
                            </div>
                            <div className="origin-note">{selectedTask.origin.imported ? "Imported from .vscode/tasks.json; Taskdeck saves only overrides." : "Defined in taskdeck.yaml."}</div>
                            <fieldset className="field">
                                <legend>Arguments</legend>
                                <div className="repeater" id="args-rows">
                                    {selectedTask.args.map((argument, index) => <div className="repeater-row" key={index}>
                                        <input data-arg={index} value={argument} aria-label={`Argument ${index + 1}`} onChange={(event) => updateTask(taskIndex, (task) => ({...task, args: task.args.map((item, position) => position === index ? event.target.value : item)}))}/>
                                        <button className="icon-button" type="button" data-remove-arg={index} aria-label="Remove argument" title="Remove" onClick={() => updateTask(taskIndex, (task) => ({...task, args: task.args.filter((_, position) => position !== index)}))}>×</button>
                                    </div>)}
                                </div>
                                <button className="button compact" type="button" data-add-arg onClick={() => updateTask(taskIndex, (task) => ({...task, args: [...task.args, ""]}))}>Add argument</button>
                            </fieldset>
                            <fieldset className="field">
                                <legend>Environment</legend>
                                <div className="repeater" id="env-rows">
                                    {selectedTask.envRows.map((row, index) => <div className="repeater-row env" key={index}>
                                        <input data-env-key={index} value={row.key} placeholder="NAME" aria-label="Environment key" onChange={(event) => updateTask(taskIndex, (task) => ({...task, envRows: task.envRows.map((item, position) => position === index ? {...item, key: event.target.value} : item)}))}/>
                                        <input data-env-value={index} value={row.value} placeholder="Value" aria-label="Environment value" onChange={(event) => updateTask(taskIndex, (task) => ({...task, envRows: task.envRows.map((item, position) => position === index ? {...item, value: event.target.value} : item)}))}/>
                                        <button className="icon-button" type="button" data-remove-env={index} aria-label="Remove environment variable" title="Remove" onClick={() => updateTask(taskIndex, (task) => ({...task, envRows: task.envRows.filter((_, position) => position !== index)}))}>×</button>
                                    </div>)}
                                </div>
                                <button className="button compact" type="button" data-add-env onClick={() => updateTask(taskIndex, (task) => ({...task, envRows: [...task.envRows, {key: "", value: ""}]}))}>Add variable</button>
                            </fieldset>
                            <button className="button danger" type="button" data-delete-task onClick={() => {
                                const next = tasks.filter((_, index) => index !== taskIndex);
                                setTasks(next);
                                setTaskIndex(Math.max(0, Math.min(taskIndex, next.length - 1)));
                                setDirty(true);
                            }}>Delete task</button>
                        </> : <div className="monitor-empty">Add a task to begin.</div>}
                    </div>
                    <footer>
                        <button className="button" id="reload-config" type="button" hidden={!reloadVisible} onClick={() => void loadConfig(true)}>Reload</button>
                        <button className="button primary" id="save-config" type="submit" disabled={saving}>Apply</button>
                    </footer>
                </form>
            </div>
        </div>
    </dialog>;
}

export function TasksView(): React.JSX.Element {
    const [nodes, setNodes] = useState<NodeSummary[]>([]);
    const [workspaces, setWorkspaces] = useState<WorkspaceSummary[]>([]);
    const [sessions, setSessions] = useState<string[]>([]);
    const [selectedNode, setSelectedNode] = useState("");
    const [selectedSession, setSelectedSession] = useState("");
    const [snapshot, setSnapshot] = useState<SessionSnapshot | null>(null);
    const [snapshotNode, setSnapshotNode] = useState<string | null>(null);
    const [currentTask, setCurrentTask] = useState<string | null>(null);
    const [seenExits, setSeenExits] = useState<SeenExits>(() => {
        try {
            const value = JSON.parse(localStorage.getItem("taskdeck-seen-exits") || "{}");
            return typeof value === "object" && value !== null && !Array.isArray(value) ? value as SeenExits : {};
        } catch { return {}; }
    });
    const [logLines, setLogLines] = useState<LogLine[]>([]);
    const [metrics, setMetrics] = useState<TaskMetricsSnapshot | null>(null);
    const [tail, setTail] = useState(() => Number(localStorage.getItem("taskdeck-log-tail")) || 1000);
    const [search, setSearch] = useState("");
    const [matchIndex, setMatchIndex] = useState(0);
    const [follow, setFollow] = useState(true);
    const [scrollToMatch, setScrollToMatch] = useState(false);
    const [workspaceMode, setWorkspaceMode] = useState<WorkspaceMode>(() => {
        const saved = localStorage.getItem("taskdeck-worker-mode");
        return saved === "log" || saved === "monitor" ? saved : "split";
    });
    const [splitPosition, setSplitPosition] = useState(0.62);
    const [tabOrderSaving, setTabOrderSaving] = useState(false);
    const tabDrag = useRef<SortDrag | null>(null);
    const [configOpen, setConfigOpen] = useState(false);
    const [busyAction, setBusyAction] = useState<"start" | "pause" | "resume" | "restart" | "stop" | null>(null);
    const [workspaceLoading, setWorkspaceLoading] = useState(false);
    const nodesRequest = useRef(0);
    const snapshotRequest = useRef(0);
    const logsRequest = useRef(0);
    const metricsRequest = useRef(0);
    const logGeneration = useRef<number | null>(null);
    const lastLogSeq = useRef<number | null>(null);
    const logContext = useRef("");
    const suppressTabClick = useRef(false);
    const previousNode = useRef("");
    const previousSession = useRef("");
    const configGuard = useRef<() => boolean>(() => true);
    const stageRef = useRef<HTMLDivElement>(null);
    const tabsRef = useRef<HTMLDivElement>(null);

    const task = snapshot && currentTask ? snapshot.tasks[currentTask] : null;
    const labels = snapshot ? orderedTaskLabels(snapshot) : [];
    const selectedNodeState = nodes.find((node) => node.id === selectedNode) ?? null;
    const online = selectedNodeState?.online !== false;
    const metaText = selectedNodeState?.online === false
        ? `${selectedNodeState.name} is offline`
        : snapshot ? `${snapshot.project} - ${snapshot.source}` : "No sessions registered";

    const resetLogCursor = (): void => {
        setLogLines([]);
        logGeneration.current = null;
        lastLogSeq.current = null;
        logContext.current = "";
    };
    const clearWorkspace = (loading = false): void => {
        snapshotRequest.current += 1;
        metricsRequest.current += 1;
        logsRequest.current += 1;
        setSnapshot(null);
        setSnapshotNode(null);
        setCurrentTask(null);
        setMetrics(null);
        resetLogCursor();
        setWorkspaceLoading(loading);
    };

    useEffect(() => {
        setMetaText(metaText);
    }, [metaText]);
    useEffect(() => { previousNode.current = selectedNode; }, [selectedNode]);
    useEffect(() => { previousSession.current = selectedSession; }, [selectedSession]);
    useEffect(() => {
        setSelectOptions(document.getElementById("nodes") as HTMLSelectElement | null,
            nodes.length ? nodes.map((node) => ({value: node.id, label: node.is_self ? `This device · ${node.name}` : `${node.name}${node.online ? "" : " · offline"}`})) : [{value: "", label: "No nodes"}], selectedNode, "No nodes");
        setSelectOptions(document.getElementById("sessions") as HTMLSelectElement | null,
            sessionOptions(workspaces, sessions, "No sessions"), selectedSession, "No sessions");
    }, [nodes, workspaces, sessions, selectedNode, selectedSession]);
    useEffect(() => {
        const nodeSelect = document.getElementById("nodes");
        const sessionSelect = document.getElementById("sessions");
        if (!(nodeSelect instanceof HTMLSelectElement) || !(sessionSelect instanceof HTMLSelectElement)) return;
        const onNodeChange = (event: Event): void => {
            const next = (event.target as HTMLSelectElement).value;
            if (!configGuard.current()) {
                event.target && ((event.target as HTMLSelectElement).value = previousNode.current);
                setSelectedNode(previousNode.current);
                return;
            }
            previousNode.current = next;
            setSelectedNode(next);
            clearWorkspace();
            setSessions([]);
            setWorkspaces([]);
            setSelectedSession("");
        };
        const onSessionChange = (event: Event): void => {
            const next = (event.target as HTMLSelectElement).value;
            if (!configGuard.current()) {
                (event.target as HTMLSelectElement).value = previousSession.current;
                setSelectedSession(previousSession.current);
                return;
            }
            previousSession.current = next;
            setSelectedSession(next);
            setWorkspaceLoading(true);
            snapshotRequest.current += 1;
            metricsRequest.current += 1;
            logsRequest.current += 1;
            setSnapshot(null);
            setCurrentTask(null);
            setMetrics(null);
            resetLogCursor();
            void loadWorkspaceData(selectedNode, next);
        };
        nodeSelect.addEventListener("change", onNodeChange);
        sessionSelect.addEventListener("change", onSessionChange);
        return () => { nodeSelect.removeEventListener("change", onNodeChange); sessionSelect.removeEventListener("change", onSessionChange); };
    }, []);
    useEffect(() => {
        const container = tabsRef.current;
        if (!container) return;
        const finish = (event: PointerEvent): void => {
            const drag = tabDrag.current;
            if (!drag || drag.pointerId !== event.pointerId) return;
            drag.element.classList.remove("dragging");
            if (drag.active) {
                const order = Array.from(container.querySelectorAll<HTMLElement>("[data-task]")).map((element) => element.dataset.task ?? "");
                if (order.join("\u0000") !== drag.original.join("\u0000")) {
                    suppressTabClick.current = true;
                    void persistWorkspaceOrder(order, drag.original);
                    window.setTimeout(() => { suppressTabClick.current = false; }, 80);
                }
            }
            tabDrag.current = null;
        };
        const onPointerDown = (event: PointerEvent): void => {
            if (event.button !== 0 || tabOrderSaving) return;
            const target = event.target instanceof Element ? event.target : null;
            const item = target?.closest<HTMLElement>(".tab");
            const key = item?.dataset.task;
            if (!item || !key) return;
            item.setPointerCapture?.(event.pointerId);
            item.classList.add("dragging");
            tabDrag.current = {
                pointerId: event.pointerId, key, element: item, startX: event.clientX, startY: event.clientY,
                active: false, original: Array.from(container.querySelectorAll<HTMLElement>("[data-task]")).map((element) => element.dataset.task ?? ""),
            };
        };
        const onPointerMove = (event: PointerEvent): void => {
            const drag = tabDrag.current;
            if (!drag || drag.pointerId !== event.pointerId) return;
            if (!drag.active && Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) < 6) return;
            event.preventDefault();
            drag.active = true;
            drag.element.classList.add("dragging");
            const target = document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>(".tab");
            const targetKey = target?.dataset.task;
            if (!targetKey || targetKey === drag.key) return;
            const from = labels.indexOf(drag.key);
            const to = labels.indexOf(targetKey);
            const rect = target?.getBoundingClientRect();
            if (!rect) return;
            const before = event.clientX < rect.left + rect.width / 2;
            container.insertBefore(drag.element, before ? target : target.nextSibling);
        };
        container.addEventListener("pointerdown", onPointerDown);
        container.addEventListener("pointermove", onPointerMove);
        container.addEventListener("pointerup", finish);
        container.addEventListener("pointercancel", finish);
        return () => {
            container.removeEventListener("pointerdown", onPointerDown);
            container.removeEventListener("pointermove", onPointerMove);
            container.removeEventListener("pointerup", finish);
            container.removeEventListener("pointercancel", finish);
        };
    }, [labels, snapshot, selectedNode, tabOrderSaving]);

    useEffect(() => { void loadNodes(); }, []);
    const pollTick = useRef<() => void>(() => {});
    pollTick.current = () => { void loadSnapshot(); void loadLogs(); void loadMetrics(); };
    useEffect(() => {
        if (selectedNodeState?.online === false) return;
        const id = window.setInterval(() => pollTick.current(), 1000);
        return () => window.clearInterval(id);
    }, [selectedNode, selectedSession, currentTask, selectedNodeState?.online]);
    useEffect(() => {
        const nodeTimer = window.setInterval(() => { void loadNodes(); }, 5000);
        return () => window.clearInterval(nodeTimer);
    }, [selectedNode]);

    const loadNodes = async (): Promise<void> => {
        const requestId = nodesRequest.current + 1;
        nodesRequest.current = requestId;
        try {
            const response = await api.request("/api/nodes", decodeNodes);
            if (requestId !== nodesRequest.current) return;
            setNodes(requireData(response));
            setConnectionState(true);
            if (!selectedNode || !response.data?.some((node) => node.id === selectedNode)) setSelectedNode(response.data?.[0]?.id ?? "");
            else void loadWorkspaceData(selectedNode);
        } catch {
            if (requestId === nodesRequest.current) setConnectionState(false);
        }
    };
    const loadWorkspaceData = async (nodeId: string, sessionOverride = selectedSession): Promise<void> => {
        const nodeState = nodes.find((node) => node.id === nodeId) ?? null;
        if (!nodeState) return;
        if (nodeState.online === false) {
            const nodeSessions = nodeState.sessions || [];
            setWorkspaces([]);
            setSessions(nodeSessions);
            setSelectedSession(nodeSessions.includes(sessionOverride) ? sessionOverride : nodeSessions[0] ?? "");
            setConnectionState(true);
            if (!snapshot || snapshotNode !== nodeId) clearWorkspace();
            return;
        }
        try {
            const workspaceResponse = await api.request(`/api/workspaces?${new URLSearchParams({node: nodeId})}`, decodeWorkspaces);
            if (workspaceResponse.ok && workspaceResponse.data) {
                setWorkspaces(workspaceResponse.data);
                setSessions(workspaceResponse.data.map((workspace) => workspace.session));
            }
            const sessionResponse = await api.request(`/api/sessions?${new URLSearchParams({node: nodeId})}`, decodeStrings);
            if (!sessionResponse.ok) throw new Error(sessionResponse.message);
            const sessionData = requireData(sessionResponse);
            const workspaceData = workspaceResponse.ok && workspaceResponse.data ? workspaceResponse.data : [];
            const nextSessions = sessionData.length ? sessionData : workspaceData.map((workspace) => workspace.session);
            setSessions(nextSessions);
            setConnectionState(true);
            const nextSession = nextSessions.includes(sessionOverride) ? sessionOverride : nextSessions[0] ?? "";
            setSelectedSession(nextSession);
            if (nextSession) await loadSnapshot(nodeId, nextSession);
            else clearWorkspace();
        } catch (error) {
            setConnectionState(false);
            setMetaText(error instanceof Error ? error.message : "Daemon unavailable");
        }
    };
    useEffect(() => {
        if (!selectedNode) return;
        void loadWorkspaceData(selectedNode);
    }, [selectedNode]);

    const loadSnapshot = async (nodeId = selectedNode, sessionId = selectedSession): Promise<void> => {
        if (!sessionId || !nodeId || tabDrag.current?.active) return;
        const requestId = snapshotRequest.current + 1;
        snapshotRequest.current = requestId;
        try {
            const query = new URLSearchParams({tail: "0", node: nodeId});
            const response = await api.request(`/api/sessions/${encodeURIComponent(sessionId)}?${query}`, decodeSnapshot);
            if (requestId !== snapshotRequest.current || selectedSession !== sessionId || selectedNode !== nodeId) return;
            const data = requireData(response);
            setWorkspaceLoading(false);
            setSnapshot(data);
            setSnapshotNode(nodeId);
            const nextLabels = orderedTaskLabels(data);
            if (!currentTask || !nextLabels.includes(currentTask)) {
                resetLogCursor();
                setMetrics(null);
                setFollow(true);
                setSearch("");
                setMatchIndex(0);
                setCurrentTask(nextLabels[0] ?? null);
            } else {
                setCurrentTask(currentTask);
            }
            if (!logContext.current) void loadLogs(nodeId, sessionId, nextLabels[0] ?? null);
            setConnectionState(true);
        } catch (error) {
            if (requestId === snapshotRequest.current) setMetaText(error instanceof Error ? error.message : "Snapshot unavailable");
        }
    };
    const loadLogs = async (nodeId = selectedNode, sessionId = selectedSession, taskId = currentTask, tailValue = tail): Promise<void> => {
        if (!nodeId || !sessionId || !taskId) return;
        const context = `${nodeId}\u0000${sessionId}\u0000${taskId}\u0000${tailValue}`;
        if (logContext.current !== context) {
            logContext.current = context;
            logGeneration.current = null;
            lastLogSeq.current = null;
            setLogLines([]);
        }
        const requestId = logsRequest.current + 1;
        logsRequest.current = requestId;
        const query = new URLSearchParams({node: nodeId, limit: String(tailValue)});
        if (lastLogSeq.current != null) query.set("after", String(lastLogSeq.current));
        try {
            const response = await api.request(`/api/sessions/${encodeURIComponent(sessionId)}/tasks/${encodeURIComponent(taskId)}/logs?${query}`, decodeLogs);
            if (requestId !== logsRequest.current || selectedNode !== nodeId || selectedSession !== sessionId || currentTask !== taskId || logContext.current !== context) return;
            if (!response.ok) throw new Error(response.message);
            const reconciled = reconcileLogs(logLines, requireData(response), tailValue, logGeneration.current, lastLogSeq.current);
            setLogLines(reconciled.lines);
            logGeneration.current = reconciled.generation;
            lastLogSeq.current = reconciled.lastSeq;
        } catch (error) {
            if (requestId === logsRequest.current) showToast(error instanceof Error ? error.message : "Logs unavailable");
        }
    };
    const loadMetrics = async (nodeId = selectedNode, sessionId = selectedSession, taskId = currentTask): Promise<void> => {
        if (!nodeId || !sessionId || !taskId) return;
        const requestId = metricsRequest.current + 1;
        metricsRequest.current = requestId;
        try {
            const response = await api.request(`/api/sessions/${encodeURIComponent(sessionId)}/tasks/${encodeURIComponent(taskId)}/metrics?${new URLSearchParams({node: nodeId, window: "600"})}`, decodeMetrics);
            if (requestId !== metricsRequest.current || selectedNode !== nodeId || selectedSession !== sessionId || currentTask !== taskId) return;
            setMetrics(requireData(response));
        } catch {
            if (requestId === metricsRequest.current) {
                const label = document.getElementById("monitor-state");
                if (label) label.textContent = "Unavailable";
            }
        }
    };

    const markExitSeen = (label: string): void => {
        const generation = Number(snapshot?.tasks[label]?.run_generation || 0);
        if (!generation) return;
        const key = exitKey(selectedNode, snapshot?.project || "", label);
        setSeenExits((previous) => {
            const next = {...previous, [key]: generation};
            localStorage.setItem("taskdeck-seen-exits", JSON.stringify(next));
            return next;
        });
    };
    const selectTask = (label: string): void => {
        if (suppressTabClick.current) return;
        markExitSeen(label);
        setCurrentTask(label);
        setMetrics(null);
        resetLogCursor();
        setFollow(true);
        setSearch("");
        setMatchIndex(0);
        void loadLogs(selectedNode, selectedSession, label);
        void loadMetrics(selectedNode, selectedSession, label);
    };
    const persistWorkspaceOrder = async (order: string[], previousOrder: string[]): Promise<void> => {
        if (tabOrderSaving || !snapshot) return;
        const session = snapshot.name;
        const node = selectedNode;
        setTabOrderSaving(true);
        setSnapshot({...snapshot, task_order: [...order]});
        try {
            const query = new URLSearchParams({node});
            const current = await api.request(`/api/sessions/${encodeURIComponent(session)}/config?${query}`, decodeConfig);
            const currentData = requireData(current);
            if (selectedNode !== node || snapshot?.name !== session) throw new Error("The selected workspace changed");
            const byLabel = new Map(currentData.tasks.map((task: EditableTask) => [task.label, task]));
            if (order.some((label) => !byLabel.has(label)) || byLabel.size !== order.length) throw new Error("Task configuration changed; reload before reordering");
            const response = await api.request(`/api/sessions/${encodeURIComponent(session)}/config?${query}`, decodeConfig, {
                method: "PUT",
                headers: {"content-type": "application/json"},
                body: JSON.stringify({revision: currentData.revision, tasks: validateConfigTasks(order.map((label) => taskToDraft(byLabel.get(label) ?? {label, command: ""})))}),
            });
            if (!response.ok) throw new Error(response.message);
            showToast("Task order saved");
        } catch (error) {
            if (snapshot?.name === session && selectedNode === node) setSnapshot({...snapshot, task_order: [...previousOrder]});
            showToast(error instanceof Error ? error.message : "Unable to save task order");
        } finally {
            setTabOrderSaving(false);
        }
    };
    const act = async (action: "start" | "pause" | "resume" | "restart" | "stop", button: HTMLButtonElement): Promise<void> => {
        if (!selectedNode || !snapshot || !currentTask || selectedSession !== snapshot.name || button.disabled || busyAction) return;
        setBusyAction(action);
        button.disabled = true;
        try {
            const response = await api.request("/api/action", decodeAccepted, {method: "POST", headers: {"content-type": "application/json"}, body: JSON.stringify({node: selectedNode, session: snapshot.name, task: currentTask, action})});
            if (!response.ok) throw new Error(response.message);
            if (selectedSession === snapshot.name && currentTask) await loadSnapshot();
        } catch (error) {
            showToast(error instanceof Error ? error.message : "Action failed");
        } finally {
            button.disabled = false;
            setBusyAction(null);
        }
    };
    const clearHistory = async (): Promise<void> => {
        if (!snapshot || !currentTask || !selectedNode || selectedNodeState?.online === false) return;
        try {
            const response = await api.request(`/api/sessions/${encodeURIComponent(snapshot.name)}/tasks/${encodeURIComponent(currentTask)}/history?${new URLSearchParams({node: selectedNode})}`, decodeAccepted, {method: "DELETE"});
            if (!response.ok) throw new Error(response.message);
            resetLogCursor();
            setMetrics(null);
            setLogLines([]);
            await Promise.all([loadLogs(), loadMetrics()]);
            showToast("Logs and performance history cleared");
        } catch (error) {
            showToast(error instanceof Error ? error.message : "Unable to clear history");
        }
    };
    const moveMatch = (direction: -1 | 1): void => {
        if (!search.trim()) return;
        const total = logLines.reduce((count, line) => count + matchOffsets(line.text, search.trim()).length, 0);
        if (!total) return;
        setFollow(false);
        setMatchIndex(normalizeMatchIndex(matchIndex + direction, total));
        setScrollToMatch(true);
    };
    const handleLogAction = (action: string): void => {
        const logs = document.getElementById("logs");
        if (!logs) return;
        if (action === "previous") moveMatch(-1);
        if (action === "next") moveMatch(1);
        if (action === "clear-search") {
            setSearch("");
            setMatchIndex(0);
            const input = document.getElementById("log-search");
            if (input instanceof HTMLInputElement) { input.value = ""; input.focus(); }
        }
        if (action === "top") { setFollow(false); logs.scrollTop = 0; }
        if (action === "bottom") { setFollow(true); logs.scrollTop = logs.scrollHeight; }
        if (action === "follow") { setFollow(!follow); if (!follow) logs.scrollTop = logs.scrollHeight; }
        if (action === "fullscreen") void toggleLogFullscreen();
        if (action === "clear-history") void clearHistory();
    };
    const toggleLogFullscreen = async (): Promise<void> => {
        const panel = document.getElementById("log-panel");
        if (document.fullscreenElement === panel) {
            await document.exitFullscreen();
            return;
        }
        if (panel?.requestFullscreen) {
            try {
                await panel.requestFullscreen();
                return;
            } catch { /* fallback below */ }
        }
        panel?.classList.toggle("fallback-fullscreen");
    };
    useEffect(() => {
        const onFullscreenChange = (): void => {
            const panel = document.getElementById("log-panel");
            if (!document.fullscreenElement && panel?.classList.contains("fallback-fullscreen")) panel.classList.remove("fallback-fullscreen");
            const button = document.querySelector<HTMLButtonElement>('[data-log="fullscreen"]');
            const active = document.fullscreenElement === panel || Boolean(panel?.classList.contains("fallback-fullscreen"));
            button?.setAttribute("aria-pressed", String(active));
            if (button) button.title = active ? "Exit full screen" : "Full screen logs";
        };
        document.addEventListener("fullscreenchange", onFullscreenChange);
        return () => document.removeEventListener("fullscreenchange", onFullscreenChange);
    }, []);
    useEffect(() => {
        const onClick = (event: MouseEvent): void => {
            const target = event.target as Element;
            const action = target.closest<HTMLButtonElement>("[data-action]");
            if (action?.dataset.action) void act(action.dataset.action as "start" | "pause" | "resume" | "restart" | "stop", action);
            const logAction = target.closest<HTMLButtonElement>("[data-log]");
            if (logAction?.dataset.log) handleLogAction(logAction.dataset.log);
            const mode = target.closest<HTMLButtonElement>("[data-mode]");
            if (mode?.dataset.mode === "log" || mode?.dataset.mode === "split" || mode?.dataset.mode === "monitor") setWorkspaceMode(mode.dataset.mode);
            if (target.closest("[data-config]")) setConfigOpen(true);
        };
        const pane = document.getElementById("task-pane");
        pane?.addEventListener("click", onClick);
        return () => pane?.removeEventListener("click", onClick);
    });
    useEffect(() => {
        const stage = stageRef.current;
        if (!stage) return;
        stage.className = `worker-stage mode-${workspaceMode}`;
        stage.style.setProperty("--split-position", String(splitPosition));
        if (workspaceMode !== "split" || window.matchMedia("(max-width: 1180px)").matches) stage.style.removeProperty("grid-template-columns");
        else updateSplitLayout();
        const narrow = window.matchMedia("(max-width: 820px)").matches;
        document.querySelectorAll<HTMLButtonElement>("#task-header [data-mode]").forEach((button) => {
            const active = button.dataset.mode === workspaceMode || (narrow && button.dataset.mode === "log" && workspaceMode === "split");
            button.classList.toggle("active", active);
            button.setAttribute("aria-pressed", String(active));
        });
    }, [workspaceMode, splitPosition, snapshot, currentTask]);
    const updateSplitLayout = (): void => {
        const stage = stageRef.current;
        if (!stage || workspaceMode !== "split" || window.matchMedia("(max-width: 1180px)").matches) return;
        const width = stage.getBoundingClientRect().width;
        if (!width) return;
        const gap = 12;
        const logWidth = Math.round((width - gap) * splitPosition);
        stage.style.gridTemplateColumns = `minmax(0, ${logWidth}px) minmax(0, 1fr)`;
        const divider = document.getElementById("split-divider");
        divider?.style.setProperty("--split-left", `${logWidth + gap / 2}px`);
    };

    const canStart = task ? taskStatusAllowsAction(task.status, "start") : false;
    const canPause = task ? taskStatusAllowsAction(task.status, "pause") : false;
    const canResume = task ? taskStatusAllowsAction(task.status, "resume") : false;
    const canStop = task ? taskStatusAllowsAction(task.status, "stop") : false;
    const technology = task?.service?.technology;
    const technologyLabel = technology?.framework || technology?.runtime || "";
    const status = task?.status || "unknown";
    return <>
        <section className="view workspace-view active" id="tasks-view" data-react-owned="true">
            <div className={`task-tabs${tabOrderSaving ? " saving-order" : ""}`} id="tabs" role="tablist" aria-label="Tasks" ref={tabsRef}
                onClick={(event) => { const button = (event.target as Element).closest<HTMLElement>("[data-task]"); if (button?.dataset.task) selectTask(button.dataset.task); }}
                onKeyDown={(event: KeyboardEvent<HTMLDivElement>) => {
                    if (!["ArrowLeft", "ArrowRight"].includes(event.key)) return;
                    const tabs = Array.from(document.querySelectorAll<HTMLButtonElement>("#tabs [data-task]"));
                    const index = tabs.indexOf(document.activeElement as HTMLButtonElement);
                    if (index < 0 || !tabs.length) return;
                    event.preventDefault();
                    if (event.altKey) {
                        const target = index + (event.key === "ArrowRight" ? 1 : -1);
                        if (target < 0 || target >= tabs.length) return;
                        const previous = tabs.map((tab) => tab.dataset.task ?? "");
                        const order = [...previous];
                        [order[index], order[target]] = [order[target], order[index]];
                        const activeTab = tabs[index];
                        void persistWorkspaceOrder(order, previous);
                        window.requestAnimationFrame(() => activeTab.focus());
                        return;
                    }
                    const next = tabs[(index + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length];
                    next.click();
                    next.focus();
                }}
            >
                {labels.map((label) => {
                    const tabTask = snapshot?.tasks[label];
                    const dot = tabTask ? taskStateDot(label, tabTask, seenExits) : null;
                    return <button className={`tab${label === currentTask ? " active" : ""}`} key={label} type="button" role="tab"
                        tabIndex={label === currentTask ? 0 : -1} aria-selected={label === currentTask}
                        aria-label={`${label}${dot ? `, ${dot.label}` : ""}`} data-task={label} data-order-key={label}>
                        {dot ? <span className={`task-state-dot ${dot.className}`} title={dot.label}/> : null}
                        <span>{label}</span>
                    </button>;
                })}
            </div>
            <div className="task-pane" id="task-pane">
                {!task ? <div className="empty-state"><div><h1>{workspaceLoading ? "Loading workspace" : metaText.includes("offline") ? "Worker offline" : "No active workspace"}</h1>{workspaceLoading ? null : <p>{metaText.includes("offline") ? `Last seen ${selectedNodeState?.last_seen_ms ? new Date(selectedNodeState.last_seen_ms).toLocaleString() : "Unknown"}` : "Register a project from the CLI or TUI."}</p>}</div></div> : <>
                    <div className="task-header" id="task-header">
                        <div className="task-heading">
                            <div className="eyebrow">{online ? "Selected task" : "Offline snapshot"}</div>
                            <h1>{task.label}</h1>
                            <div className="task-detail">
                                <span className={`status ${status}`}>{status}</span>
                                {task.pid ? <span>PID {task.pid}</span> : null}
                                {technologyLabel ? <span className="technology-chip" title={(technology?.evidence || []).join(" · ")}>{technologyLabel}</span> : null}
                                {(task.service?.endpoints || []).map(endpointElement)}
                                <span className="cwd">{task.cwd}</span>
                            </div>
                        </div>
                        <div className="view-modes" aria-label="Workspace layout">
                            {(["log", "split", "monitor"] as const).map((mode) => { const active = mode === workspaceMode || (window.matchMedia("(max-width: 820px)").matches && mode === "log" && workspaceMode === "split"); return <button key={mode} type="button" data-mode={mode} className={`${mode === "split" ? "desktop-split-mode" : ""}${active ? " active" : ""}`} aria-pressed={active}>{mode === "log" ? "Logs" : mode === "split" ? "Split" : "Monitor"}</button>; })}
                        </div>
                        <div className="actions">
                            <button className="button primary" type="button" data-action="start" disabled={!online || !canStart || busyAction === "start"}>{actionIcon("start")}Start</button>
                            <button className="button" type="button" data-action="pause" disabled={!online || !canPause || busyAction === "pause"}>{actionIcon("pause")}Pause</button>
                            <button className="button" type="button" data-action="resume" disabled={!online || !canResume || busyAction === "resume"}>{actionIcon("restart")}Resume</button>
                            <button className="button" type="button" data-action="restart" disabled={!online || busyAction === "restart"}>{actionIcon("restart")}Restart</button>
                            <button className="button danger" type="button" data-action="stop" disabled={!online || !canStop || busyAction === "stop"}>{actionIcon("stop")}Stop</button>
                            <button className="icon-button" type="button" data-config aria-label="Edit configuration" title="Edit configuration" disabled={!online}>{settingsIcon}</button>
                        </div>
                    </div>
                    <div className={`worker-stage mode-${workspaceMode}`} id="worker-stage" ref={stageRef} key={currentTask ?? ""}
                        style={{"--split-position": splitPosition} as React.CSSProperties}>
                        <LogsPanel lines={logLines} query={search} matchIndex={matchIndex} follow={follow} scrollToMatch={scrollToMatch}
                            logContext={logContext.current} logGeneration={logGeneration.current} tail={tail}
                            onFollowChange={setFollow} onScrolledToMatch={() => setScrollToMatch(false)} onSearchChange={(value) => { setSearch(value); setMatchIndex(0); setScrollToMatch(Boolean(value)); if (value) setFollow(false); }} onSearchSubmit={moveMatch} onTailChange={(value) => { setTail(value); localStorage.setItem("taskdeck-log-tail", String(value)); resetLogCursor(); void loadLogs(selectedNode, selectedSession, currentTask, value); }}/>
                        <MetricsPanel metrics={metrics}/>
                        <button className="split-divider" id="split-divider" type="button" aria-label="Resize logs and performance panels" title="Resize panels"
                            onPointerDown={(event) => {
                                if (workspaceMode !== "split" || window.matchMedia("(max-width: 1180px)").matches) return;
                                event.currentTarget.setPointerCapture(event.pointerId);
                                event.currentTarget.classList.add("dragging");
                                stageRef.current?.classList.add("resizing");
                            }}
                            onPointerMove={(event) => {
                                if (!event.currentTarget.hasPointerCapture(event.pointerId) || !stageRef.current) return;
                                const bounds = stageRef.current.getBoundingClientRect();
                                const position = (event.clientX - bounds.left) / bounds.width;
                                if (position <= 0.25) setWorkspaceMode("monitor");
                                else if (position >= 0.75) setWorkspaceMode("log");
                                else { setSplitPosition(Math.max(0.25, Math.min(0.75, position))); updateSplitLayout(); }
                            }}
                            onPointerUp={(event) => { event.currentTarget.classList.remove("dragging"); stageRef.current?.classList.remove("resizing"); }}
                            onPointerCancel={(event) => { event.currentTarget.classList.remove("dragging"); stageRef.current?.classList.remove("resizing"); }}
                            onKeyDown={(event: KeyboardEvent<HTMLButtonElement>) => {
                                if (workspaceMode !== "split") return;
                                if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
                                    setSplitPosition((position) => Math.max(0.25, Math.min(0.75, position + (event.key === "ArrowLeft" ? -0.04 : 0.04))));
                                    event.preventDefault();
                                } else if (event.key === "Home") { setWorkspaceMode("log"); event.preventDefault(); }
                                else if (event.key === "End") { setWorkspaceMode("monitor"); event.preventDefault(); }
                            }}
                        ><span aria-hidden="true"/></button>
                    </div>
                </>}
            </div>
        </section>
        <ConfigDialog open={configOpen} setOpen={setConfigOpen} node={selectedNode} session={selectedSession} currentTask={currentTask} nodes={nodes} onSnapshotRefresh={() => void loadSnapshot()} registerGuard={(guard) => { configGuard.current = guard; }}/>
    </>;
}
