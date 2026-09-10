import { useEffect } from "react";
import { LegacyApiAdapter } from "../../compat/api";
import {
    decodeLogs,
    decodeMetrics,
    decodeNodes,
    decodeSnapshot,
    decodeStrings,
    decodeWorkspaces,
} from "../../api/session";
import { orderedTaskLabels, reconcileLogs } from "./helpers";
import { requireData } from "./utils";
import { setConnectionState, sessionOptions, setMetaText, setSelectOptions } from "./dom";
import type {
    LogLine,
    NodeSummary,
    SessionSnapshot,
    TaskMetricsSnapshot,
    WorkspaceSummary,
} from "../../domain/models";
import { showToast } from "../../lib/toast";
import type { SortDrag } from "./types";

const api = new LegacyApiAdapter();

export function useWorkspaceLoaders(ctx: {
    nodes: NodeSummary[];
    setNodes: React.Dispatch<React.SetStateAction<NodeSummary[]>>;
    workspaces: WorkspaceSummary[];
    setWorkspaces: React.Dispatch<React.SetStateAction<WorkspaceSummary[]>>;
    sessions: string[];
    setSessions: React.Dispatch<React.SetStateAction<string[]>>;
    selectedNode: string;
    setSelectedNode: React.Dispatch<React.SetStateAction<string>>;
    selectedSession: string;
    setSelectedSession: React.Dispatch<React.SetStateAction<string>>;
    snapshot: SessionSnapshot | null;
    setSnapshot: React.Dispatch<React.SetStateAction<SessionSnapshot | null>>;
    snapshotNode: string | null;
    setSnapshotNode: React.Dispatch<React.SetStateAction<string | null>>;
    currentTask: string | null;
    setCurrentTask: React.Dispatch<React.SetStateAction<string | null>>;
    logLines: LogLine[];
    setLogLines: React.Dispatch<React.SetStateAction<LogLine[]>>;
    metrics: TaskMetricsSnapshot | null;
    setMetrics: React.Dispatch<React.SetStateAction<TaskMetricsSnapshot | null>>;
    tail: number;
    workspaceLoading: boolean;
    setWorkspaceLoading: React.Dispatch<React.SetStateAction<boolean>>;
    nodesRequest: React.RefObject<number>;
    snapshotRequest: React.RefObject<number>;
    logsRequest: React.RefObject<number>;
    metricsRequest: React.RefObject<number>;
    logGeneration: React.RefObject<number | null>;
    lastLogSeq: React.RefObject<number | null>;
    logContext: React.RefObject<string>;
    previousNode: React.RefObject<string>;
    previousSession: React.RefObject<string>;
    tabDrag: React.RefObject<SortDrag | null>;
    clearWorkspace: (loading?: boolean) => void;
    resetLogCursor: () => void;
    metaText: string;
    setFollow: React.Dispatch<React.SetStateAction<boolean>>;
    setSearch: React.Dispatch<React.SetStateAction<string>>;
    setMatchIndex: React.Dispatch<React.SetStateAction<number>>;
}) {
    const {
        nodes, setNodes, workspaces, setWorkspaces, sessions, setSessions,
        selectedNode, setSelectedNode, selectedSession, setSelectedSession,
        snapshot, setSnapshot, snapshotNode, setSnapshotNode,
        currentTask, setCurrentTask,
        logLines, setLogLines, metrics, setMetrics, tail,
        workspaceLoading, setWorkspaceLoading,
        nodesRequest, snapshotRequest, logsRequest, metricsRequest,
        logGeneration, lastLogSeq, logContext,
        previousNode, previousSession, tabDrag,
        clearWorkspace, resetLogCursor, metaText,
        setFollow, setSearch, setMatchIndex,
    } = ctx;

    const loadNodes = async (): Promise<void> => {
        const requestId = nodesRequest.current + 1;
        nodesRequest.current = requestId;
        try {
            const response = await api.request("/api/nodes", decodeNodes);
            if (requestId !== nodesRequest.current) return;
            setNodes(requireData(response));
            setConnectionState(true);
            if (!selectedNode || !response.data?.some((node) => node.id === selectedNode))
                setSelectedNode(response.data?.[0]?.id ?? "");
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
            setSelectedSession(nodeSessions.includes(sessionOverride) ? sessionOverride : (nodeSessions[0] ?? ""));
            setConnectionState(true);
            if (!snapshot || snapshotNode !== nodeId) clearWorkspace();
            return;
        }
        try {
            const workspaceResponse = await api.request(
                `/api/workspaces?${new URLSearchParams({ node: nodeId })}`,
                decodeWorkspaces,
            );
            if (workspaceResponse.ok && workspaceResponse.data) {
                setWorkspaces(workspaceResponse.data);
                setSessions(workspaceResponse.data.map((workspace) => workspace.session));
            }
            const sessionResponse = await api.request(
                `/api/sessions?${new URLSearchParams({ node: nodeId })}`,
                decodeStrings,
            );
            if (!sessionResponse.ok) throw new Error(sessionResponse.message);
            const sessionData = requireData(sessionResponse);
            const workspaceData = workspaceResponse.ok && workspaceResponse.data ? workspaceResponse.data : [];
            const nextSessions = sessionData.length ? sessionData : workspaceData.map((workspace) => workspace.session);
            setSessions(nextSessions);
            setConnectionState(true);
            const nextSession = nextSessions.includes(sessionOverride) ? sessionOverride : (nextSessions[0] ?? "");
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
            const query = new URLSearchParams({ tail: "0", node: nodeId });
            const response = await api.request(
                `/api/sessions/${encodeURIComponent(sessionId)}?${query}`,
                decodeSnapshot,
            );
            if (requestId !== snapshotRequest.current || selectedSession !== sessionId || selectedNode !== nodeId)
                return;
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
            if (requestId === snapshotRequest.current)
                setMetaText(error instanceof Error ? error.message : "Snapshot unavailable");
        }
    };
    const loadLogs = async (
        nodeId = selectedNode,
        sessionId = selectedSession,
        taskId = currentTask,
        tailValue = tail,
    ): Promise<void> => {
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
        const query = new URLSearchParams({ node: nodeId, limit: String(tailValue) });
        if (lastLogSeq.current != null) query.set("after", String(lastLogSeq.current));
        try {
            const response = await api.request(
                `/api/sessions/${encodeURIComponent(sessionId)}/tasks/${encodeURIComponent(taskId)}/logs?${query}`,
                decodeLogs,
            );
            if (
                requestId !== logsRequest.current ||
                selectedNode !== nodeId ||
                selectedSession !== sessionId ||
                currentTask !== taskId ||
                logContext.current !== context
            )
                return;
            if (!response.ok) throw new Error(response.message);
            const reconciled = reconcileLogs(
                logLines,
                requireData(response),
                tailValue,
                logGeneration.current,
                lastLogSeq.current,
            );
            setLogLines(reconciled.lines);
            logGeneration.current = reconciled.generation;
            lastLogSeq.current = reconciled.lastSeq;
        } catch (error) {
            if (requestId === logsRequest.current)
                showToast(error instanceof Error ? error.message : "Logs unavailable");
        }
    };
    const loadMetrics = async (
        nodeId = selectedNode,
        sessionId = selectedSession,
        taskId = currentTask,
    ): Promise<void> => {
        if (!nodeId || !sessionId || !taskId) return;
        const requestId = metricsRequest.current + 1;
        metricsRequest.current = requestId;
        try {
            const response = await api.request(
                `/api/sessions/${encodeURIComponent(sessionId)}/tasks/${encodeURIComponent(taskId)}/metrics?${new URLSearchParams({ node: nodeId, window: "600" })}`,
                decodeMetrics,
            );
            if (
                requestId !== metricsRequest.current ||
                selectedNode !== nodeId ||
                selectedSession !== sessionId ||
                currentTask !== taskId
            )
                return;
            setMetrics(requireData(response));
        } catch {
            if (requestId === metricsRequest.current) {
                const label = document.getElementById("monitor-state");
                if (label) label.textContent = "Unavailable";
            }
        }
    };

    return { loadNodes, loadWorkspaceData, loadSnapshot, loadLogs, loadMetrics };
}
