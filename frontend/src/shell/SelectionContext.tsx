import {createContext, useCallback, useContext, useEffect, useMemo, useRef, useState} from "react";
import type {NodeSummary, WorkspaceSummary} from "../domain/models";
import {decodeNodes, decodeStrings, decodeWorkspaces} from "../api/session";
import {LegacyApiAdapter} from "../compat/api";
import {showToast} from "../lib/toast";
import {keepAvailable} from "./selection";

const api = new LegacyApiAdapter();
const NODE_KEY = "taskdeck-selected-node";
const WORKSPACE_KEY = "taskdeck-selected-workspaces";

type SelectionGuard = () => boolean;

export interface SelectionContextValue {
    nodes: NodeSummary[];
    workspaces: WorkspaceSummary[];
    sessions: string[];
    selectedNode: string;
    selectedSession: string;
    loading: boolean;
    error: string;
    meta: string;
    setMeta: (value: string) => void;
    selectNode: (node: string) => boolean;
    selectSession: (session: string) => boolean;
    refresh: () => Promise<void>;
    renameNode: (name: string) => Promise<void>;
    renameWorkspace: (alias: string | null) => Promise<void>;
    registerGuard: (guard: SelectionGuard) => void;
}

const SelectionContext = createContext<SelectionContextValue | null>(null);

function readWorkspaceSelections(): Record<string, string> {
    try {
        const value = JSON.parse(localStorage.getItem(WORKSPACE_KEY) || "{}");
        return typeof value === "object" && value !== null && !Array.isArray(value)
            ? value as Record<string, string>
            : {};
    } catch {
        return {};
    }
}

export function SelectionProvider({children}: {children: React.ReactNode}): React.JSX.Element {
    const [nodes, setNodes] = useState<NodeSummary[]>([]);
    const [workspaces, setWorkspaces] = useState<WorkspaceSummary[]>([]);
    const [sessions, setSessions] = useState<string[]>([]);
    const [selectedNode, setSelectedNode] = useState(() => localStorage.getItem(NODE_KEY) || "");
    const [selectedSession, setSelectedSession] = useState("");
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState("");
    const [meta, setMeta] = useState("Loading workspace context…");
    const request = useRef(0);
    const selectedNodeRef = useRef(selectedNode);
    const selectedSessionRef = useRef(selectedSession);
    const workspaceSelections = useRef(readWorkspaceSelections());
    const guard = useRef<SelectionGuard>(() => true);

    useEffect(() => { selectedNodeRef.current = selectedNode; }, [selectedNode]);
    useEffect(() => { selectedSessionRef.current = selectedSession; }, [selectedSession]);

    const persistSession = useCallback((node: string, session: string): void => {
        if (!node) return;
        workspaceSelections.current = {...workspaceSelections.current, [node]: session};
        localStorage.setItem(WORKSPACE_KEY, JSON.stringify(workspaceSelections.current));
    }, []);

    const loadWorkspaces = useCallback(async (node: NodeSummary, preferred?: string, quiet = false): Promise<void> => {
        const requestId = ++request.current;
        if (!quiet) setLoading(true);
        if (!quiet) setError("");
        try {
            const workspaceResponse = await api.request(
                `/api/workspaces?${new URLSearchParams({node: node.id})}`,
                decodeWorkspaces,
            );
            const sessionResponse = await api.request(
                `/api/sessions?${new URLSearchParams({node: node.id})}`,
                decodeStrings,
            );
            if (requestId !== request.current || selectedNodeRef.current !== node.id) return;
            if (!sessionResponse.ok) throw new Error(sessionResponse.message);
            const nextWorkspaces = workspaceResponse.ok ? workspaceResponse.data ?? [] : [];
            const nextSessions = sessionResponse.data?.length
                ? sessionResponse.data
                : nextWorkspaces.map((workspace) => workspace.session);
            const remembered = preferred ?? workspaceSelections.current[node.id] ?? "";
            const nextSession = keepAvailable(selectedSessionRef.current, nextSessions, remembered);
            setWorkspaces(nextWorkspaces);
            setSessions(nextSessions);
            setSelectedSession(nextSession);
            selectedSessionRef.current = nextSession;
            persistSession(node.id, nextSession);
            if (remembered && remembered !== nextSession) showToast(`Workspace '${remembered}' is no longer available. Switched to ${nextSession || "no workspace"}.`);
            if (!quiet) setMeta(node.online === false ? `${node.name} is offline` : nextSessions.length ? `${nextSessions.length} workspace${nextSessions.length === 1 ? "" : "s"}` : "No sessions registered");
        } catch (cause) {
            if (requestId !== request.current) return;
            const message = cause instanceof Error ? cause.message : "Workspace context unavailable";
            setError(message);
            setMeta(message);
        } finally {
            if (requestId === request.current && !quiet) setLoading(false);
        }
    }, [persistSession]);

    const refresh = useCallback(async (): Promise<void> => {
        try {
            const response = await api.request("/api/nodes", decodeNodes);
            if (!response.ok) throw new Error(response.message);
            const nextNodes = response.data ?? [];
            setNodes(nextNodes);
            const current = selectedNodeRef.current;
            const nextNode = keepAvailable(current, nextNodes.map((node) => node.id));
            if (nextNode !== current) {
                setSelectedNode(nextNode);
                selectedNodeRef.current = nextNode;
                if (nextNode) localStorage.setItem(NODE_KEY, nextNode);
            }
            const node = nextNodes.find((item) => item.id === nextNode);
            if (current && nextNode !== current) showToast(`Node '${current}' is no longer available. Switched to ${node?.name ?? "no node"}.`);
            if (node) await loadWorkspaces(node, nextNode === current ? selectedSessionRef.current : undefined, nextNode === current && nodes.length > 0);
            else {
                setWorkspaces([]);
                setSessions([]);
                setSelectedSession("");
                setLoading(false);
                setMeta("No nodes registered");
            }
        } catch (cause) {
            const message = cause instanceof Error ? cause.message : "Daemon unavailable";
            setError(message);
            setMeta(message);
            setLoading(false);
        }
    }, [loadWorkspaces, nodes.length]);

    useEffect(() => {
        void refresh();
        const timer = window.setInterval(() => void refresh(), 5000);
        return () => window.clearInterval(timer);
    }, [refresh]);

    const selectNode = useCallback((nodeId: string): boolean => {
        if (nodeId === selectedNodeRef.current) return true;
        if (!guard.current()) return false;
        const node = nodes.find((item) => item.id === nodeId);
        if (!node) return false;
        request.current += 1;
        setSelectedNode(nodeId);
        selectedNodeRef.current = nodeId;
        localStorage.setItem(NODE_KEY, nodeId);
        setSelectedSession("");
        selectedSessionRef.current = "";
        setWorkspaces([]);
        setSessions([]);
        void loadWorkspaces(node);
        return true;
    }, [loadWorkspaces, nodes]);

    const selectSession = useCallback((session: string): boolean => {
        if (session === selectedSessionRef.current) return true;
        if (!guard.current() || !sessions.includes(session)) return false;
        setSelectedSession(session);
        selectedSessionRef.current = session;
        persistSession(selectedNodeRef.current, session);
        return true;
    }, [persistSession, sessions]);

    const renameNode = useCallback(async (name: string): Promise<void> => {
        const trimmed = name.trim();
        if (!trimmed || !selectedNodeRef.current) throw new Error("Display name is required.");
        const response = await api.request(
            `/api/nodes/${encodeURIComponent(selectedNodeRef.current)}/settings`,
            (value) => value,
            {method: "PUT", headers: {"content-type": "application/json"}, body: JSON.stringify({name: trimmed})},
        );
        if (!response.ok) throw new Error(response.message);
        await refresh();
    }, [refresh]);

    const renameWorkspace = useCallback(async (alias: string | null): Promise<void> => {
        const node = selectedNodeRef.current;
        const session = selectedSessionRef.current;
        if (!node || !session) throw new Error("Select a workspace first.");
        const response = await api.request(
            `/api/workspaces/${encodeURIComponent(session)}/alias?${new URLSearchParams({node})}`,
            (value) => value,
            {method: "PUT", headers: {"content-type": "application/json"}, body: JSON.stringify({alias: alias?.trim() || null})},
        );
        if (!response.ok) throw new Error(response.message);
        const selectedNodeState = nodes.find((item) => item.id === node);
        if (selectedNodeState) await loadWorkspaces(selectedNodeState, session);
    }, [loadWorkspaces, nodes]);

    const value = useMemo<SelectionContextValue>(() => ({
        nodes, workspaces, sessions, selectedNode, selectedSession, loading, error, meta, setMeta,
        selectNode, selectSession, refresh, renameNode, renameWorkspace,
        registerGuard: (nextGuard) => { guard.current = nextGuard; },
    }), [nodes, workspaces, sessions, selectedNode, selectedSession, loading, error, meta, selectNode, selectSession, refresh, renameNode, renameWorkspace]);

    return <SelectionContext.Provider value={value}>{children}</SelectionContext.Provider>;
}

export function useSelection(): SelectionContextValue {
    const value = useContext(SelectionContext);
    if (!value) throw new Error("useSelection must be used inside SelectionProvider");
    return value;
}
