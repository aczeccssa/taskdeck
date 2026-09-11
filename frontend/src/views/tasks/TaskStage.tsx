import type { KeyboardEvent } from "react";
import type { LogLine, NodeSummary, SessionSnapshot, TaskMetricsSnapshot } from "../../domain/models";
import { LogsPanel } from "./LogsPanel";
import { MetricsPanel } from "./MetricsPanel";
import { ConfigDialog } from "./ConfigDialog";
import { TaskHeader } from "./TaskHeader";
import { endpointElement } from "./utils";
import { taskStateDot } from "./helpers";
import { TaskTabs } from "./TaskTabs";
import type { SeenExits, WorkspaceMode } from "./types";

export type TaskStageContext = {
    task: SessionSnapshot["tasks"][string] | null;
    online: boolean;
    status: string;
    technology: { framework?: string; runtime?: string; evidence?: string[] } | null | undefined;
    technologyLabel: string;
    workspaceMode: WorkspaceMode;
    splitPosition: number;
    busyAction: "start" | "pause" | "resume" | "restart" | "stop" | null;
    canStart: boolean;
    canPause: boolean;
    canResume: boolean;
    canStop: boolean;
    labels: string[];
    currentTask: string | null;
    seenExits: SeenExits;
    tabOrderSaving: boolean;
    snapshot: SessionSnapshot | null;
    selectedNode: string;
    selectedSession: string;
    selectedNodeState: NodeSummary | null;
    workspaceLoading: boolean;
    metaText: string;
    logLines: LogLine[];
    metrics: TaskMetricsSnapshot | null;
    search: string;
    matchIndex: number;
    follow: boolean;
    scrollToMatch: boolean;
    tail: number;
    logGeneration: { current: number | null };
    logContext: { current: string };
    stageRef: React.RefObject<HTMLDivElement | null>;
    tabsRef: React.RefObject<HTMLDivElement | null>;
    configOpen: boolean;
    setConfigOpen: (open: boolean) => void;
    setWorkspaceMode: (mode: WorkspaceMode) => void;
    setSplitPosition: React.Dispatch<React.SetStateAction<number>>;
    setFollow: (follow: boolean) => void;
    setScrollToMatch: (scroll: boolean) => void;
    setSearch: (search: string) => void;
    setMatchIndex: (index: number) => void;
    setTail: (tail: number) => void;
    selectTask: (label: string) => void;
    persistWorkspaceOrder: (order: string[], previousOrder: string[]) => Promise<void>;
    moveMatch: (direction: -1 | 1) => void;
    handleLogAction: (action: string) => void;
    toggleLogFullscreen: () => Promise<void>;
    resetLogCursor: () => void;
    loadLogs: (nodeId?: string, sessionId?: string, taskId?: string | null, tailValue?: number) => Promise<void>;
    loadSnapshot: (nodeId?: string, sessionId?: string) => Promise<void>;
    act: (action: "start" | "pause" | "resume" | "restart" | "stop", button: HTMLButtonElement) => Promise<void>;
    updateSplitLayout: (position?: number) => void;
    configGuard: { current: () => boolean };
    nodes: NodeSummary[];
};

export function TaskStage({
    task,
    online,
    status,
    technology,
    technologyLabel,
    workspaceMode,
    splitPosition,
    busyAction,
    canStart,
    canPause,
    canResume,
    canStop,
    labels,
    currentTask,
    seenExits,
    tabOrderSaving,
    snapshot,
    selectedNode,
    selectedSession,
    selectedNodeState,
    workspaceLoading,
    metaText,
    logLines,
    metrics,
    search,
    matchIndex,
    follow,
    scrollToMatch,
    tail,
    logGeneration,
    logContext,
    stageRef,
    tabsRef,
    configOpen,
    setConfigOpen,
    setWorkspaceMode,
    setSplitPosition,
    setFollow,
    setScrollToMatch,
    setSearch,
    setMatchIndex,
    setTail,
    selectTask,
    persistWorkspaceOrder,
    moveMatch,
    handleLogAction,
    toggleLogFullscreen,
    resetLogCursor,
    loadLogs,
    loadSnapshot,
    act,
    updateSplitLayout,
    configGuard,
    nodes,
}: TaskStageContext): React.JSX.Element {
    return (
        <>
            <section className="view workspace-view active" id="tasks-view" data-react-owned="true">
                <TaskTabs
                    tabOrderSaving={tabOrderSaving}
                    tabsRef={tabsRef}
                    labels={labels}
                    snapshot={snapshot}
                    seenExits={seenExits}
                    currentTask={currentTask}
                    selectTask={selectTask}
                    persistWorkspaceOrder={persistWorkspaceOrder}
                />
                <div className="task-pane" id="task-pane">
                    {!task ? (
                        <div className="empty-state">
                            <div>
                                <h1>
                                    {workspaceLoading
                                        ? "Loading workspace"
                                        : metaText.includes("offline")
                                          ? "Worker offline"
                                          : "No active workspace"}
                                </h1>
                                {workspaceLoading ? null : (
                                    <p>
                                        {metaText.includes("offline")
                                            ? `Last seen ${selectedNodeState?.last_seen_ms ? new Date(selectedNodeState.last_seen_ms).toLocaleString() : "Unknown"}`
                                            : "Register a project from the CLI or TUI."}
                                    </p>
                                )}
                            </div>
                        </div>
                    ) : (
                        <>
                    <TaskHeader
                        task={task}
                        online={online}
                        status={status}
                        technology={technology}
                        technologyLabel={technologyLabel}
                        workspaceMode={workspaceMode}
                        setWorkspaceMode={setWorkspaceMode}
                        busyAction={busyAction}
                        canStart={canStart}
                        canPause={canPause}
                        canResume={canResume}
                        canStop={canStop}
                        act={act}
                        setConfigOpen={setConfigOpen}
                    />
                            <div
                                className={`worker-stage mode-${workspaceMode}`}
                                id="worker-stage"
                                ref={stageRef}
                                key={currentTask ?? ""}
                                style={{ "--split-position": splitPosition } as React.CSSProperties}>
                                <LogsPanel
                                    lines={logLines}
                                    query={search}
                                    matchIndex={matchIndex}
                                    follow={follow}
                                    scrollToMatch={scrollToMatch}
                                    logContext={logContext.current}
                                    logGeneration={logGeneration.current}
                                    tail={tail}
                                    onFollowChange={setFollow}
                                    onScrolledToMatch={() => setScrollToMatch(false)}
                                    onSearchChange={(value) => {
                                        setSearch(value);
                                        setMatchIndex(0);
                                        setScrollToMatch(Boolean(value));
                                        if (value) setFollow(false);
                                    }}
                                    onSearchSubmit={moveMatch}
                                    onTailChange={(value) => {
                                        setTail(value);
                                        localStorage.setItem("taskdeck-log-tail", String(value));
                                        resetLogCursor();
                                        void loadLogs(selectedNode, selectedSession, currentTask, value);
                                    }}
                                />
                                <MetricsPanel metrics={metrics} />
                                <button
                                    className="split-divider"
                                    id="split-divider"
                                    type="button"
                                    aria-label="Resize logs and performance panels"
                                    title="Resize panels"
                                    onPointerDown={(event) => {
                                        if (
                                            workspaceMode !== "split" ||
                                            window.matchMedia("(max-width: 1180px)").matches
                                        )
                                            return;
                                        event.currentTarget.setPointerCapture(event.pointerId);
                                        event.currentTarget.classList.add("dragging");
                                        stageRef.current?.classList.add("resizing");
                                    }}
                                    onPointerMove={(event) => {
                                        if (
                                            !event.currentTarget.hasPointerCapture(event.pointerId) ||
                                            !stageRef.current
                                        )
                                            return;
                                        const bounds = stageRef.current.getBoundingClientRect();
                                        const position = (event.clientX - bounds.left) / bounds.width;
                                        updateSplitLayout(Math.max(0.25, Math.min(0.75, position)));
                                    }}
                                    onPointerUp={(event) => {
                                        const bounds = stageRef.current?.getBoundingClientRect();
                                        const position = bounds ? (event.clientX - bounds.left) / bounds.width : 0.62;
                                        event.currentTarget.classList.remove("dragging");
                                        stageRef.current?.classList.remove("resizing");
                                        if (position <= 0.25) setWorkspaceMode("monitor");
                                        else if (position >= 0.75) setWorkspaceMode("log");
                                        else setSplitPosition(position);
                                    }}
                                    onPointerCancel={(event) => {
                                        event.currentTarget.classList.remove("dragging");
                                        stageRef.current?.classList.remove("resizing");
                                        updateSplitLayout();
                                    }}
                                    onKeyDown={(event: KeyboardEvent<HTMLButtonElement>) => {
                                        if (workspaceMode !== "split") return;
                                        if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
                                            setSplitPosition((position) => {
                                                const next = Math.max(
                                                    0.25,
                                                    Math.min(
                                                        0.75,
                                                        position + (event.key === "ArrowLeft" ? -0.04 : 0.04),
                                                    ),
                                                );
                                                updateSplitLayout(next);
                                                return next;
                                            });
                                            event.preventDefault();
                                        } else if (event.key === "Home") {
                                            setWorkspaceMode("monitor");
                                            event.preventDefault();
                                        } else if (event.key === "End") {
                                            setWorkspaceMode("log");
                                            event.preventDefault();
                                        }
                                    }}>
                                    <span aria-hidden="true" />
                                </button>
                            </div>
                        </>
                    )}
                </div>
            </section>
            <ConfigDialog
                open={configOpen}
                setOpen={setConfigOpen}
                node={selectedNode}
                session={selectedSession}
                currentTask={currentTask}
                nodes={nodes}
                onSnapshotRefresh={() => void loadSnapshot()}
                registerGuard={(guard) => {
                    configGuard.current = guard;
                }}
            />
        </>
    );
}
