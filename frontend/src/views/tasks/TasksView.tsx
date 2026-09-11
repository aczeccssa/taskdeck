import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import type { EditableTask, LogLine, SessionSnapshot, TaskMetricsSnapshot } from "../../domain/models";
import { showToast } from "../../lib/toast";
import { LegacyApiAdapter } from "../../compat/api";
import { decodeConfig } from "../../api/session";
import {
    matchOffsets,
    normalizeMatchIndex,
    orderedTaskLabels,
    reconcileLogs,
    taskStateDot,
    taskStatusAllowsAction,
    taskToDraft,
    validateConfigTasks,
} from "./helpers";
import { decodeAccepted, endpointElement, requireData } from "./utils";
import type { SeenExits, SortDrag, WorkspaceMode } from "./types";
import { useTaskActions } from "./useTaskActions";
import { useWorkspaceLoaders } from "./useWorkspaceLoaders";
import { useTaskEffects } from "./useTaskEffects";
import { TaskStage } from "./TaskStage";
import { useSelection } from "../../shell/SelectionContext";

const api = new LegacyApiAdapter();

function exitKey(node: string, project: string, task: string): string {
    return `${node}\u0000${project}\u0000${task}`;
}

export function TasksView(): React.JSX.Element {
    const selection = useSelection();
    const { nodes, selectedNode, selectedSession } = selection;
    const [snapshot, setSnapshot] = useState<SessionSnapshot | null>(null);
    const [currentTask, setCurrentTask] = useState<string | null>(null);
    const [seenExits, setSeenExits] = useState<SeenExits>(() => {
        try {
            const value = JSON.parse(localStorage.getItem("taskdeck-seen-exits") || "{}");
            return typeof value === "object" && value !== null && !Array.isArray(value) ? (value as SeenExits) : {};
        } catch {
            return {};
        }
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
    const snapshotRequest = useRef(0);
    const logsRequest = useRef(0);
    const metricsRequest = useRef(0);
    const logGeneration = useRef<number | null>(null);
    const lastLogSeq = useRef<number | null>(null);
    const logContext = useRef("");
    const suppressTabClick = useRef(false);
    const configGuard = useRef<() => boolean>(() => true);
    const stageRef = useRef<HTMLDivElement>(null);
    const tabsRef = useRef<HTMLDivElement>(null);

    const task = snapshot && currentTask ? snapshot.tasks[currentTask] : null;
    const labels = snapshot ? orderedTaskLabels(snapshot) : [];
    const selectedNodeState = nodes.find((node) => node.id === selectedNode) ?? null;
    const online = selectedNodeState?.online !== false;
    const metaText =
        selectedNodeState?.online === false
            ? `${selectedNodeState.name} is offline`
            : snapshot
              ? `${snapshot.project} - ${snapshot.source}`
              : "No sessions registered";

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
        setCurrentTask(null);
        setMetrics(null);
        resetLogCursor();
        setWorkspaceLoading(loading);
    };

    const { loadSnapshot, loadLogs, loadMetrics } = useWorkspaceLoaders({
        selectedNode, selectedSession, snapshot, setSnapshot,
        currentTask, setCurrentTask,
        logLines, setLogLines, setMetrics, tail, setWorkspaceLoading,
        snapshotRequest, logsRequest, metricsRequest,
        logGeneration, lastLogSeq, logContext,
        tabDrag, resetLogCursor, setMeta: selection.setMeta,
        setFollow, setSearch, setMatchIndex,
    });

    useEffect(() => {
        selection.registerGuard(() => configGuard.current());
        return () => selection.registerGuard(() => true);
    }, [selection.registerGuard]);
    useEffect(() => {
        clearWorkspace(Boolean(selectedNode && selectedSession));
        if (selectedNode && selectedSession) void loadSnapshot(selectedNode, selectedSession);
    }, [selectedNode, selectedSession]);
    useEffect(() => {
        const container = tabsRef.current;
        if (!container) return;
        const finish = (event: PointerEvent): void => {
            const drag = tabDrag.current;
            if (!drag || drag.pointerId !== event.pointerId) return;
            drag.element.classList.remove("dragging");
            if (drag.active) {
                const order = Array.from(container.querySelectorAll<HTMLElement>("[data-task]")).map(
                    (element) => element.dataset.task ?? "",
                );
                if (order.join("\u0000") !== drag.original.join("\u0000")) {
                    suppressTabClick.current = true;
                    void persistWorkspaceOrder(order, drag.original);
                    window.setTimeout(() => {
                        suppressTabClick.current = false;
                    }, 80);
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
                pointerId: event.pointerId,
                key,
                element: item,
                startX: event.clientX,
                startY: event.clientY,
                active: false,
                original: Array.from(container.querySelectorAll<HTMLElement>("[data-task]")).map(
                    (element) => element.dataset.task ?? "",
                ),
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

    const pollTick = useRef<() => void>(() => {});
    pollTick.current = () => {
        void loadSnapshot();
        void loadLogs();
        void loadMetrics();
    };
    useEffect(() => {
        if (selectedNodeState?.online === false) return;
        const id = window.setInterval(() => pollTick.current(), 1000);
        return () => window.clearInterval(id);
    }, [selectedNode, selectedSession, currentTask, selectedNodeState?.online]);

    const markExitSeen = (label: string): void => {
        const generation = Number(snapshot?.tasks[label]?.run_generation || 0);
        if (!generation) return;
        const key = exitKey(selectedNode, snapshot?.project || "", label);
        setSeenExits((previous) => {
            const next = { ...previous, [key]: generation };
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
        setSnapshot({ ...snapshot, task_order: [...order] });
        try {
            const query = new URLSearchParams({ node });
            const current = await api.request(
                `/api/sessions/${encodeURIComponent(session)}/config?${query}`,
                decodeConfig,
            );
            const currentData = requireData(current);
            if (selectedNode !== node || snapshot?.name !== session) throw new Error("The selected workspace changed");
            const byLabel = new Map(currentData.tasks.map((task: EditableTask) => [task.label, task]));
            if (order.some((label) => !byLabel.has(label)) || byLabel.size !== order.length)
                throw new Error("Task configuration changed; reload before reordering");
            const response = await api.request(
                `/api/sessions/${encodeURIComponent(session)}/config?${query}`,
                decodeConfig,
                {
                    method: "PUT",
                    headers: { "content-type": "application/json" },
                    body: JSON.stringify({
                        revision: currentData.revision,
                        tasks: validateConfigTasks(
                            order.map((label) => taskToDraft(byLabel.get(label) ?? { label, command: "" })),
                        ),
                    }),
                },
            );
            if (!response.ok) throw new Error(response.message);
            showToast("Task order saved");
        } catch (error) {
            if (snapshot?.name === session && selectedNode === node)
                setSnapshot({ ...snapshot, task_order: [...previousOrder] });
            showToast(error instanceof Error ? error.message : "Unable to save task order");
        } finally {
            setTabOrderSaving(false);
        }
    };

    const selectedNodeOnline = online;
    const { act, clearHistory, moveMatch, handleLogAction, toggleLogFullscreen } = useTaskActions({
        snapshot, currentTask, selectedNode, selectedSession, selectedNodeOnline,
        logLines, search, matchIndex, follow, busyAction, tail,
        setBusyAction, setFollow, setMatchIndex, setScrollToMatch, setSearch,
        setLogLines, setMetrics, setTail,
        resetLogCursor, loadLogs, loadMetrics, loadSnapshot,
    });
    const updateSplitLayout = (position = splitPosition): void => {
        const stage = stageRef.current;
        if (!stage || workspaceMode !== "split" || window.matchMedia("(max-width: 1180px)").matches) return;
        const width = stage.getBoundingClientRect().width;
        if (!width) return;
        const gap = 12;
        const logWidth = Math.round((width - gap) * position);
        stage.style.gridTemplateColumns = `minmax(0, ${logWidth}px) minmax(0, 1fr)`;
        stage.style.setProperty("--split-left", `${logWidth + gap / 2}px`);
    };

    const canStart = task ? taskStatusAllowsAction(task.status, "start") : false;
    const canPause = task ? taskStatusAllowsAction(task.status, "pause") : false;
    const canResume = task ? taskStatusAllowsAction(task.status, "resume") : false;
    const canStop = task ? taskStatusAllowsAction(task.status, "stop") : false;
    const technology = task?.service?.technology;
    const technologyLabel = technology?.framework || technology?.runtime || "";
    const status = task?.status || "unknown";
    useTaskEffects({
        snapshot, currentTask, workspaceMode, splitPosition,
        stageRef, setWorkspaceMode, setSplitPosition,
        act, handleLogAction, setConfigOpen, toggleLogFullscreen, updateSplitLayout,
    });

    return (
        <TaskStage
            task={task}
            online={online}
            status={status}
            technology={technology as { framework?: string; runtime?: string; evidence?: string[] } | null | undefined}
            technologyLabel={technologyLabel}
            workspaceMode={workspaceMode}
            splitPosition={splitPosition}
            busyAction={busyAction}
            canStart={canStart}
            canPause={canPause}
            canResume={canResume}
            canStop={canStop}
            labels={labels}
            currentTask={currentTask}
            seenExits={seenExits}
            tabOrderSaving={tabOrderSaving}
            snapshot={snapshot}
            selectedNode={selectedNode}
            selectedSession={selectedSession}
            selectedNodeState={selectedNodeState}
            workspaceLoading={workspaceLoading}
            metaText={metaText}
            logLines={logLines}
            metrics={metrics}
            search={search}
            matchIndex={matchIndex}
            follow={follow}
            scrollToMatch={scrollToMatch}
            tail={tail}
            logGeneration={logGeneration}
            logContext={logContext}
            stageRef={stageRef}
            tabsRef={tabsRef}
            configOpen={configOpen}
            setConfigOpen={setConfigOpen}
            setWorkspaceMode={setWorkspaceMode}
            setSplitPosition={setSplitPosition}
            setFollow={setFollow}
            setScrollToMatch={setScrollToMatch}
            setSearch={setSearch}
            setMatchIndex={setMatchIndex}
            setTail={setTail}
            selectTask={selectTask}
            persistWorkspaceOrder={persistWorkspaceOrder}
            moveMatch={moveMatch}
            handleLogAction={handleLogAction}
            toggleLogFullscreen={toggleLogFullscreen}
            resetLogCursor={resetLogCursor}
            loadLogs={loadLogs}
            loadSnapshot={loadSnapshot}
            act={act}
            updateSplitLayout={updateSplitLayout}
            configGuard={configGuard}
            nodes={nodes}
        />
    );
}
