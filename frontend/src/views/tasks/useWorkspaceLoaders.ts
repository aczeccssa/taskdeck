import {LegacyApiAdapter} from "../../compat/api";
import {decodeLogs, decodeMetrics, decodeSnapshot} from "../../api/session";
import {orderedTaskLabels, reconcileLogs} from "./helpers";
import {requireData} from "./utils";
import type {LogLine, SessionSnapshot, TaskMetricsSnapshot} from "../../domain/models";
import {showToast} from "../../lib/toast";
import type {SortDrag} from "./types";

const api = new LegacyApiAdapter();

export function useWorkspaceLoaders(ctx: {
    selectedNode: string;
    selectedSession: string;
    snapshot: SessionSnapshot | null;
    setSnapshot: React.Dispatch<React.SetStateAction<SessionSnapshot | null>>;
    currentTask: string | null;
    setCurrentTask: React.Dispatch<React.SetStateAction<string | null>>;
    logLines: LogLine[];
    setLogLines: React.Dispatch<React.SetStateAction<LogLine[]>>;
    setMetrics: React.Dispatch<React.SetStateAction<TaskMetricsSnapshot | null>>;
    tail: number;
    setWorkspaceLoading: React.Dispatch<React.SetStateAction<boolean>>;
    snapshotRequest: React.RefObject<number>;
    logsRequest: React.RefObject<number>;
    metricsRequest: React.RefObject<number>;
    logGeneration: React.RefObject<number | null>;
    lastLogSeq: React.RefObject<number | null>;
    logContext: React.RefObject<string>;
    tabDrag: React.RefObject<SortDrag | null>;
    resetLogCursor: () => void;
    setMeta: (value: string) => void;
    setFollow: React.Dispatch<React.SetStateAction<boolean>>;
    setSearch: React.Dispatch<React.SetStateAction<string>>;
    setMatchIndex: React.Dispatch<React.SetStateAction<number>>;
}) {
    const {
        selectedNode, selectedSession, snapshot, setSnapshot, currentTask, setCurrentTask,
        logLines, setLogLines, setMetrics, tail, setWorkspaceLoading,
        snapshotRequest, logsRequest, metricsRequest, logGeneration, lastLogSeq, logContext,
        tabDrag, resetLogCursor, setMeta, setFollow, setSearch, setMatchIndex,
    } = ctx;

    const loadSnapshot = async (nodeId = selectedNode, sessionId = selectedSession): Promise<void> => {
        if (!sessionId || !nodeId || tabDrag.current?.active) return;
        const requestId = snapshotRequest.current + 1;
        snapshotRequest.current = requestId;
        try {
            const query = new URLSearchParams({tail: "0", node: nodeId});
            const response = await api.request(`/api/sessions/${encodeURIComponent(sessionId)}?${query}`, decodeSnapshot);
            if (requestId !== snapshotRequest.current) return;
            const data = requireData(response);
            setWorkspaceLoading(false);
            setSnapshot(data);
            setMeta(`${data.project} · ${data.source}`);
            const nextLabels = orderedTaskLabels(data);
            const nextTask = currentTask && nextLabels.includes(currentTask) ? currentTask : nextLabels[0] ?? null;
            if (nextTask !== currentTask) {
                resetLogCursor();
                setMetrics(null);
                setFollow(true);
                setSearch("");
                setMatchIndex(0);
                setCurrentTask(nextTask);
            }
            if (nextTask && !logContext.current) void loadLogs(nodeId, sessionId, nextTask);
        } catch (cause) {
            if (requestId === snapshotRequest.current) {
                setWorkspaceLoading(false);
                setMeta(cause instanceof Error ? cause.message : "Snapshot unavailable");
            }
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
            if (requestId !== logsRequest.current || logContext.current !== context) return;
            if (!response.ok) throw new Error(response.message);
            const reconciled = reconcileLogs(logLines, requireData(response), tailValue, logGeneration.current, lastLogSeq.current);
            setLogLines(reconciled.lines);
            logGeneration.current = reconciled.generation;
            lastLogSeq.current = reconciled.lastSeq;
        } catch (cause) {
            if (requestId === logsRequest.current) showToast(cause instanceof Error ? cause.message : "Logs unavailable");
        }
    };

    const loadMetrics = async (nodeId = selectedNode, sessionId = selectedSession, taskId = currentTask): Promise<void> => {
        if (!nodeId || !sessionId || !taskId) return;
        const requestId = metricsRequest.current + 1;
        metricsRequest.current = requestId;
        try {
            const response = await api.request(`/api/sessions/${encodeURIComponent(sessionId)}/tasks/${encodeURIComponent(taskId)}/metrics?${new URLSearchParams({node: nodeId, window: "600"})}`, decodeMetrics);
            if (requestId !== metricsRequest.current) return;
            setMetrics(requireData(response));
        } catch {
            if (requestId === metricsRequest.current) {
                const label = document.getElementById("monitor-state");
                if (label) label.textContent = "Unavailable";
            }
        }
    };

    return {loadSnapshot, loadLogs, loadMetrics};
}
