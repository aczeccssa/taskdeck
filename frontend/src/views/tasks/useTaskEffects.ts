import { useEffect } from "react";
import type { NodeSummary, SessionSnapshot, WorkspaceSummary } from "../../domain/models";
import { setConnectionState, setMetaText, setSelectOptions } from "./dom";
import type { WorkspaceMode } from "./types";

export function useTaskEffects({
    metaText,
    selectedNode,
    selectedSession,
    nodes,
    workspaces,
    sessions,
    selectedNodeState,
    snapshot,
    currentTask,
    workspaceMode,
    splitPosition,
    stageRef,
    setWorkspaceMode,
    setSplitPosition,
    act,
    handleLogAction,
    setConfigOpen,
    toggleLogFullscreen,
    updateSplitLayout,
    loadNodes,
    loadWorkspaceData,
    loadSnapshot,
    loadLogs,
    loadMetrics,
    persistWorkspaceOrder,
    configGuard,
    labels,
    tabOrderSaving,
    tabsRef,
    tabDrag,
    suppressTabClick,
    tabDragActive,
}: {
    metaText: string;
    selectedNode: string;
    selectedSession: string;
    nodes: NodeSummary[];
    workspaces: WorkspaceSummary[];
    sessions: string[];
    selectedNodeState: NodeSummary | null;
    snapshot: SessionSnapshot | null;
    currentTask: string | null;
    workspaceMode: WorkspaceMode;
    splitPosition: number;
    stageRef: React.RefObject<HTMLDivElement | null>;
    setWorkspaceMode: (mode: WorkspaceMode) => void;
    setSplitPosition: React.Dispatch<React.SetStateAction<number>>;
    act: (action: "start" | "pause" | "resume" | "restart" | "stop", button: HTMLButtonElement) => Promise<void>;
    handleLogAction: (action: string) => void;
    setConfigOpen: (open: boolean) => void;
    toggleLogFullscreen: () => Promise<void>;
    updateSplitLayout: () => void;
    loadNodes: () => Promise<void>;
    loadWorkspaceData: (nodeId: string, sessionOverride?: string) => Promise<void>;
    loadSnapshot: (nodeId?: string, sessionId?: string) => Promise<void>;
    loadLogs: (nodeId?: string, sessionId?: string, taskId?: string | null, tailValue?: number) => Promise<void>;
    loadMetrics: (nodeId?: string, sessionId?: string, taskId?: string | null) => Promise<void>;
    persistWorkspaceOrder: (order: string[], previousOrder: string[]) => Promise<void>;
    configGuard: { current: () => boolean };
    labels: string[];
    tabOrderSaving: boolean;
    tabsRef: React.RefObject<HTMLDivElement | null>;
    tabDrag: { current: { active: boolean } | null };
    suppressTabClick: { current: boolean };
    tabDragActive: boolean;
}) {
    useEffect(() => {
        const onFullscreenChange = (): void => {
            const panel = document.getElementById("log-panel");
            if (!document.fullscreenElement && panel?.classList.contains("fallback-fullscreen"))
                panel.classList.remove("fallback-fullscreen");
            const button = document.querySelector<HTMLButtonElement>('[data-log="fullscreen"]');
            const active =
                document.fullscreenElement === panel || Boolean(panel?.classList.contains("fallback-fullscreen"));
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
            if (action?.dataset.action)
                void act(action.dataset.action as "start" | "pause" | "resume" | "restart" | "stop", action);
            const logAction = target.closest<HTMLButtonElement>("[data-log]");
            if (logAction?.dataset.log) handleLogAction(logAction.dataset.log);
            const mode = target.closest<HTMLButtonElement>("[data-mode]");
            if (mode?.dataset.mode === "log" || mode?.dataset.mode === "split" || mode?.dataset.mode === "monitor")
                setWorkspaceMode(mode.dataset.mode);
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
        if (workspaceMode !== "split" || window.matchMedia("(max-width: 1180px)").matches)
            stage.style.removeProperty("grid-template-columns");
        else updateSplitLayout();
        const narrow = window.matchMedia("(max-width: 820px)").matches;
        document.querySelectorAll<HTMLButtonElement>("#task-header [data-mode]").forEach((button) => {
            const active =
                button.dataset.mode === workspaceMode ||
                (narrow && button.dataset.mode === "log" && workspaceMode === "split");
            button.classList.toggle("active", active);
            button.setAttribute("aria-pressed", String(active));
        });
    }, [workspaceMode, splitPosition, snapshot, currentTask]);

}
