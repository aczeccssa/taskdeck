import { LegacyApiAdapter } from "../../compat/api";
import { showToast } from "../../lib/toast";
import { decodeAccepted } from "./utils";
import { matchOffsets, normalizeMatchIndex, taskStatusAllowsAction } from "./helpers";
import type { LogLine, SessionSnapshot, TaskMetricsSnapshot } from "../../domain/models";

const api = new LegacyApiAdapter();

export function useTaskActions({
    snapshot,
    currentTask,
    selectedNode,
    selectedSession,
    selectedNodeOnline,
    logLines,
    search,
    matchIndex,
    follow,
    busyAction,
    tail,
    setBusyAction,
    setFollow,
    setMatchIndex,
    setScrollToMatch,
    setSearch,
    setLogLines,
    setMetrics,
    setTail,
    resetLogCursor,
    loadLogs,
    loadMetrics,
    loadSnapshot,
}: {
    snapshot: SessionSnapshot | null;
    currentTask: string | null;
    selectedNode: string;
    selectedSession: string;
    selectedNodeOnline: boolean;
    logLines: LogLine[];
    search: string;
    matchIndex: number;
    follow: boolean;
    busyAction: "start" | "pause" | "resume" | "restart" | "stop" | null;
    tail: number;
    setBusyAction: (action: "start" | "pause" | "resume" | "restart" | "stop" | null) => void;
    setFollow: (follow: boolean) => void;
    setMatchIndex: (index: number) => void;
    setScrollToMatch: (scroll: boolean) => void;
    setSearch: (search: string) => void;
    setLogLines: (lines: LogLine[]) => void;
    setMetrics: (metrics: TaskMetricsSnapshot | null) => void;
    setTail: (tail: number) => void;
    resetLogCursor: () => void;
    loadLogs: (nodeId?: string, sessionId?: string, taskId?: string | null, tailValue?: number) => Promise<void>;
    loadMetrics: (nodeId?: string, sessionId?: string, taskId?: string | null) => Promise<void>;
    loadSnapshot: (nodeId?: string, sessionId?: string) => Promise<void>;
}) {
    const act = async (
        action: "start" | "pause" | "resume" | "restart" | "stop",
        button: HTMLButtonElement,
    ): Promise<void> => {
        if (
            !selectedNode ||
            !snapshot ||
            !currentTask ||
            selectedSession !== snapshot.name ||
            button.disabled ||
            busyAction
        )
            return;
        setBusyAction(action);
        button.disabled = true;
        try {
            const response = await api.request("/api/action", decodeAccepted, {
                method: "POST",
                headers: { "content-type": "application/json" },
                body: JSON.stringify({ node: selectedNode, session: snapshot.name, task: currentTask, action }),
            });
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
        if (!snapshot || !currentTask || !selectedNode || !selectedNodeOnline) return;
        try {
            const response = await api.request(
                `/api/sessions/${encodeURIComponent(snapshot.name)}/tasks/${encodeURIComponent(currentTask)}/history?${new URLSearchParams({ node: selectedNode })}`,
                decodeAccepted,
                { method: "DELETE" },
            );
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
            if (input instanceof HTMLInputElement) {
                input.value = "";
                input.focus();
            }
        }
        if (action === "top") {
            setFollow(false);
            logs.scrollTop = 0;
        }
        if (action === "bottom") {
            setFollow(true);
            logs.scrollTop = logs.scrollHeight;
        }
        if (action === "follow") {
            setFollow(!follow);
            if (!follow) logs.scrollTop = logs.scrollHeight;
        }
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
            } catch {
                /* fallback below */
            }
        }
        panel?.classList.toggle("fallback-fullscreen");
    };

    return { act, clearHistory, moveMatch, handleLogAction, toggleLogFullscreen };
}
