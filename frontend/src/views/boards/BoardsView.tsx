import { useCallback, useEffect, useMemo, useState } from "react";
import type {
    BoardCard,
    BoardCardInput,
    BoardTemplate,
    BoardView,
    BoardsView as BoardsPayload,
    NodeSummary,
    SessionSnapshot,
    WorkflowTargetView,
} from "../../domain/models";
import { showToast } from "../../lib/toast";
import { api, decode } from "../shared/api";
import { isRecord } from "../../lib/narrow";
import { BoardPanel } from "./BoardPanel";
import { BoardEditor } from "./BoardEditor";
import { as, decodeExport, keyOf, quiet, statusClass, type TemplateMessage } from "./helpers";

export function BoardsView(): React.JSX.Element {
    const [boards, setBoards] = useState<BoardView[]>([]);
    const [targets, setTargets] = useState<WorkflowTargetView[]>([]);
    const [nodes, setNodes] = useState<NodeSummary[]>([]);
    const [templates, setTemplates] = useState<BoardTemplate[]>([]);
    const [selected, setSelected] = useState<string | null>(null);
    const [editorActive, setEditorActive] = useState(false);
    const [draft, setDraft] = useState<{ name: string; cards: BoardCard[] }>({ name: "", cards: [] });
    const [snapshots, setSnapshots] = useState<Record<string, SessionSnapshot>>({});
    const [message, setMessage] = useState("");
    const [boardMessage, setBoardMessage] = useState<TemplateMessage>(quiet);
    const [busy, setBusy] = useState(false);
    const [templateName, setTemplateName] = useState("");
    const [templateId, setTemplateId] = useState<string | null>(null);
    const [sourceBoardId, setSourceBoardId] = useState("");
    const [templateMessage, setTemplateMessage] = useState<TemplateMessage>(quiet);

    const load = useCallback(async () => {
        setBusy(true);
        setMessage("");
        try {
            const [result, nodeResult, templateResult] = await Promise.all([
                api.request("/api/boards", decode),
                api.request("/api/nodes", decode),
                api.request("/api/board-templates", decode),
            ]);
            if (!result.ok) throw new Error(result.message || "Boards unavailable");
            const value = as<Partial<BoardsPayload>>(result.data, {});
            setBoards(as(value.boards, []));
            setTargets(as(value.targets, []));
            if (nodeResult.ok) setNodes(as(nodeResult.data, []));
            if (templateResult.ok)
                setTemplates(as((templateResult.data as { templates?: BoardTemplate[] })?.templates, []));
        } catch (error) {
            setMessage(error instanceof Error ? error.message : "Boards unavailable");
        } finally {
            setBusy(false);
        }
    }, []);
    useEffect(() => {
        void load();
        const timer = window.setInterval(() => void load(), 5000);
        return () => window.clearInterval(timer);
    }, [load]);

    const liveCards = useMemo(() => boards.flatMap((board) => board.cards ?? []), [boards]);
    useEffect(() => {
        let active = true;
        const pairs = [...new Set(liveCards.map((card) => `${card.node_id}\u0000${card.session}`))];
        void Promise.all(
            pairs.map(async (pair) => {
                const [node, session] = pair.split("\u0000");
                const result = await api.request(
                    `/api/sessions/${encodeURIComponent(session)}?node=${encodeURIComponent(node)}&tail=0`,
                    decode,
                );
                return [
                    pair,
                    result.ok
                        ? as<SessionSnapshot>(result.data, { name: session, project: "", source: "", tasks: {} })
                        : null,
                ] as const;
            }),
        ).then((values) => {
            if (!active) return;
            setSnapshots((previous) => {
                const next = { ...previous };
                values.forEach(([key, value]) => {
                    if (value) next[key] = value;
                });
                return next;
            });
        });
        return () => {
            active = false;
        };
    }, [liveCards]);

    const openEditor = (board?: BoardView) => {
        setSelected(board?.id ?? null);
        setEditorActive(true);
        setDraft({
            name: board?.name ?? "",
            cards: (board?.cards ?? []).map((card) => ({
                id: card.id,
                node_id: card.node_id,
                session: card.session,
                task: card.task,
                mode: card.mode,
                pinned: card.pinned,
            })),
        });
        setBoardMessage(quiet);
        setMessage("");
    };
    const cancelEditor = () => {
        setSelected(null);
        setEditorActive(false);
        setDraft({ name: "", cards: [] });
        setBoardMessage(quiet);
    };
    const save = async () => {
        if (!editorActive) return;
        if (!draft.name.trim()) {
            setBoardMessage({ text: "Board name is required.", kind: "error" });
            return;
        }
        if (draft.cards.some((card) => !card.node_id || !card.session || !card.task)) {
            setBoardMessage({ text: "Every card needs a workspace and task.", kind: "error" });
            return;
        }
        setBusy(true);
        setBoardMessage({ text: "Saving board…", kind: "" });
        const url = selected ? `/api/boards/${encodeURIComponent(selected)}` : "/api/boards";
        const result = await api.request(url, decode, {
            method: selected ? "PUT" : "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ name: draft.name.trim(), cards: draft.cards }),
        });
        setBusy(false);
        if (!result.ok) {
            setBoardMessage({ text: result.message || "Unable to save board", kind: "error" });
            return;
        }
        cancelEditor();
        setBoardMessage({ text: "Board saved.", kind: "success" });
        await load();
    };
    const removeBoard = async (board: BoardView) => {
        if (!window.confirm(`Delete board '${board.name}'?`)) return;
        const result = await api.request(`/api/boards/${encodeURIComponent(board.id)}`, decode, { method: "DELETE" });
        if (!result.ok) {
            setBoardMessage({ text: result.message || "Unable to delete board", kind: "error" });
            return;
        }
        if (selected === board.id) cancelEditor();
        await load();
        showToast("Board deleted");
    };
    const removeCard = async (card: BoardCard) => {
        const board = boards.find((item) => item.cards.some((candidate) => candidate.id === card.id));
        if (!board) return;
        const result = await api.request(`/api/boards/${encodeURIComponent(board.id)}`, decode, {
            method: "PUT",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ name: board.name, cards: board.cards.filter((item) => item.id !== card.id) }),
        });
        if (!result.ok) setMessage(result.message || "Unable to remove card");
        else await load();
    };
    const mutateCard = async (card: BoardCard, patch: Partial<BoardCard>) => {
        const board = boards.find((item) => item.cards.some((candidate) => candidate.id === card.id));
        if (!board) return;
        const result = await api.request(`/api/boards/${encodeURIComponent(board.id)}`, decode, {
            method: "PUT",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({
                name: board.name,
                cards: board.cards.map((item) => (item.id === card.id ? { ...item, ...patch } : item)),
            }),
        });
        if (!result.ok) setMessage(result.message || "Unable to update card");
        else await load();
    };
    const action = async (card: BoardCard, taskAction: string) => {
        const result = await api.request("/api/action", decode, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ node: card.node_id, session: card.session, task: card.task, action: taskAction }),
        });
        if (!result.ok) setMessage(result.message || "Action failed");
        else await load();
    };
    const pinned = liveCards.filter((card) => card.pinned);
    const statusFor = (card: BoardCard): string => snapshots[keyOf(card)]?.tasks?.[card.task]?.status ?? "unknown";
    const addCard = (target: WorkflowTargetView) =>
        setDraft((value) => ({
            ...value,
            cards: [
                ...value.cards,
                {
                    id: `draft-${Date.now()}`,
                    node_id: target.node_id,
                    session: target.session,
                    task: target.tasks?.[0] ?? "",
                    mode: "status",
                    pinned: false,
                },
            ],
        }));

    const saveTemplate = async () => {
        if (!templateName.trim()) {
            setTemplateMessage({ text: "Template name is required.", kind: "error" });
            return;
        }
        const body: Record<string, unknown> = {
            name: templateName.trim(),
            description: null,
            cards: [] as BoardCardInput[],
            source_board_id: sourceBoardId || null,
        };
        if (!sourceBoardId) {
            if (!draft.cards.length) {
                setTemplateMessage({ text: "Pick a source board or add cards in the editor first.", kind: "error" });
                return;
            }
            body.cards = draft.cards.map((card) => ({
                node_id: card.node_id,
                session: card.session,
                task: card.task,
                mode: card.mode,
                pinned: card.pinned,
            }));
        }
        const result = await api.request("/api/board-templates", decode, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify(body),
        });
        if (!result.ok) {
            setTemplateMessage({ text: result.message || "Unable to save template", kind: "error" });
            return;
        }
        setTemplateMessage({ text: "Template saved.", kind: "success" });
        setTemplateId(isRecord(result.data) && typeof result.data.id === "string" ? result.data.id : null);
        await load();
    };
    const applyTemplate = async () => {
        if (!templateId) {
            setTemplateMessage({ text: "Select a template first.", kind: "error" });
            return;
        }
        const name = draft.name.trim() || `Board ${new Date().toISOString().slice(0, 10)}`;
        const result = await api.request(`/api/board-templates/${encodeURIComponent(templateId)}/apply`, decode, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ name }),
        });
        if (!result.ok) {
            setTemplateMessage({ text: result.message || "Unable to apply template", kind: "error" });
            return;
        }
        setTemplateMessage({ text: "Board created from template.", kind: "success" });
        await load();
    };
    const exportTemplate = async () => {
        if (!templateId) {
            setTemplateMessage({ text: "Select a template first.", kind: "error" });
            return;
        }
        const result = await api.request(`/api/board-templates/${encodeURIComponent(templateId)}/export`, decodeExport);
        if (!result.ok) {
            setTemplateMessage({ text: result.message || "Unable to export template", kind: "error" });
            return;
        }
        const blob = new Blob([JSON.stringify(result.data, null, 2)], { type: "application/json" });
        const url = URL.createObjectURL(blob);
        const link = document.createElement("a");
        link.href = url;
        link.download = `taskdeck-board-template-${result.data?.name || "export"}.json`;
        link.click();
        URL.revokeObjectURL(url);
        setTemplateMessage({ text: "Template exported.", kind: "success" });
    };
    const importTemplate = async (file: File) => {
        try {
            const exportData: unknown = JSON.parse(await file.text());
            const result = await api.request("/api/board-templates/import", decode, {
                method: "POST",
                headers: { "content-type": "application/json" },
                body: JSON.stringify(exportData),
            });
            if (!result.ok) throw new Error(result.message || "Unable to import template");
            setTemplateMessage({ text: "Template imported.", kind: "success" });
            setTemplateId(isRecord(result.data) && typeof result.data.id === "string" ? result.data.id : null);
            await load();
        } catch (error) {
            setTemplateMessage({
                text: error instanceof Error ? error.message : "Unable to import template",
                kind: "error",
            });
        }
    };
    const deleteTemplate = async () => {
        if (!templateId) {
            setTemplateMessage({ text: "Select a template first.", kind: "error" });
            return;
        }
        if (!window.confirm("Delete template?")) return;
        const result = await api.request(`/api/board-templates/${encodeURIComponent(templateId)}`, decode, {
            method: "DELETE",
        });
        if (!result.ok) {
            setTemplateMessage({ text: result.message || "Unable to delete template", kind: "error" });
            return;
        }
        setTemplateId(null);
        await load();
    };

    return (
        <section
            className={`view active boards-view react-view${editorActive ? " editor-open" : ""}`}
            data-react-owned="true"
            id="boards-view">
            <div className="workflow-layout">
                <div className="workflow-main">
                    <header className="section-heading">
                        <div>
                            <p id="boards-summary" className="muted">
                                {boards.length} boards · {liveCards.length} cards · {pinned.length} pinned
                            </p>
                        </div>
                        <div className="settings-actions">
                            <button className="button" type="button" onClick={() => void load()}>
                                Refresh
                            </button>
                            <button className="button primary" type="button" onClick={() => openEditor()}>
                                New board
                            </button>
                        </div>
                    </header>
                    {message && (
                        <div className="settings-message" role="status">
                            {message}
                        </div>
                    )}
                    <section className="workflow-card">
                        <header>
                            <div>
                                <h2>Board scope</h2>
                                <p>Choose sources for board cards. Live node metrics stay in Dashboard.</p>
                            </div>
                        </header>
                        <div className="board-node-grid">
                            {nodes.length ? (
                                nodes.map((node) => (
                                    <span className={`board-scope-node ${node.online ? "" : "offline"}`} key={node.id}>
                                        <i aria-hidden="true" />
                                        <strong>{node.is_self ? `This device · ${node.name}` : node.name}</strong>
                                        <small>
                                            {node.role} · {node.sessions?.length ?? 0} workspace
                                            {node.sessions?.length === 1 ? "" : "s"}
                                        </small>
                                    </span>
                                ))
                            ) : (
                                <span className="muted">No nodes known.</span>
                            )}
                        </div>
                        <div className="workflow-targets board-workspace-grid">
                            {targets.length ? (
                                targets.map((target) => (
                                    <button
                                        className="board-source"
                                        type="button"
                                        key={`${target.node_id}-${target.session}`}
                                        onClick={() => addCard(target)}>
                                        <strong>{target.workspace_display_name}</strong>
                                        <span>
                                            {target.node_name} · {target.tasks?.length ?? 0} tasks
                                        </span>
                                    </button>
                                ))
                            ) : (
                                <span className="muted">No workspaces registered.</span>
                            )}
                        </div>
                    </section>
                    <section className="workflow-card">
                        <header>
                            <div>
                                <h2>Pinned tasks</h2>
                                <p>Status at a glance across every board.</p>
                            </div>
                        </header>
                        <div className="board-card-grid">
                            {pinned.length ? (
                                pinned.map((card) => (
                                    <article className="board-card compact" key={keyOf(card)}>
                                        <div className="board-card-title">
                                            <span
                                                className={`task-state-dot board-dot ${statusClass(statusFor(card))}`}
                                                aria-hidden="true"
                                            />
                                            <div>
                                                <strong>{card.task}</strong>
                                                <span>
                                                    {card.session} · {card.node_id}
                                                </span>
                                            </div>
                                        </div>
                                        <span className="status-pill">{statusFor(card)}</span>
                                    </article>
                                ))
                            ) : (
                                <div className="muted">Pin cards from any board to watch their status here.</div>
                            )}
                        </div>
                    </section>
                    {busy && !boards.length ? (
                        <div className="muted" aria-busy="true">
                            Loading boards…
                        </div>
                    ) : boards.length ? (
                        boards.map((board) => (
                            <BoardPanel
                                key={board.id}
                                board={board}
                                snapshots={snapshots}
                                onEdit={() => openEditor(board)}
                                onDelete={() => void removeBoard(board)}
                                onMode={(card, mode) => void mutateCard(card, { mode })}
                                onPin={(card) => void mutateCard(card, { pinned: !card.pinned })}
                                onRemove={(card) => void removeCard(card)}
                                onAction={action}
                            />
                        ))
                    ) : (
                        <div className="empty-state compact">
                            <div>
                                <h1>No boards</h1>
                                <p>
                                    Create a board to tile task status, logs, and performance across nodes and
                                    workspaces.
                                </p>
                            </div>
                        </div>
                    )}
                </div>
                <BoardEditor
                    active={editorActive}
                    editing={selected !== null}
                    draft={draft}
                    targets={targets}
                    boards={boards}
                    onChange={setDraft}
                    onAdd={addCard}
                    onSave={() => void save()}
                    onCancel={cancelEditor}
                    boardMessage={boardMessage}
                    templates={templates}
                    templateName={templateName}
                    templateId={templateId}
                    sourceBoardId={sourceBoardId}
                    templateMessage={templateMessage}
                    onTemplateName={setTemplateName}
                    onSourceBoard={setSourceBoardId}
                    onTemplateSelect={setTemplateId}
                    onSaveTemplate={() => void saveTemplate()}
                    onApplyTemplate={() => void applyTemplate()}
                    onExportTemplate={() => void exportTemplate()}
                    onImportTemplate={(file) => void importTemplate(file)}
                    onDeleteTemplate={() => void deleteTemplate()}
                />
            </div>
        </section>
    );
}
