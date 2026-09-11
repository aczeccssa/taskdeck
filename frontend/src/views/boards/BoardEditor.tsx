import type { BoardCard, BoardCardMode, WorkflowTargetView } from "../../domain/models";
import type { TemplateMessage } from "./helpers";

export function BoardEditor({
    active,
    editing,
    draft,
    targets,
    onChange,
    onAdd,
    onSave,
    onCancel,
    boardMessage,
}: {
    active: boolean;
    editing: boolean;
    draft: { name: string; cards: BoardCard[] };
    targets: WorkflowTargetView[];
    onChange: (draft: { name: string; cards: BoardCard[] }) => void;
    onAdd: (target: WorkflowTargetView) => void;
    onSave: () => void;
    onCancel: () => void;
    boardMessage: TemplateMessage;
}): React.JSX.Element {
    return (
        <aside className="workflow-editor" aria-labelledby="board-editor-title">
            <header>
                <div>
                    <h2 id="board-editor-title">{editing ? "Edit board" : "New board"}</h2>
                    <p>Choose the task cards shown on this board.</p>
                </div>
                <button
                    className="icon-button editor-close"
                    type="button"
                    aria-label="Close board editor"
                    onClick={onCancel}>
                    ×
                </button>
            </header>
            <label className="field">
                <span>Name</span>
                <input
                    id="board-name"
                    type="text"
                    value={draft.name}
                    onChange={(event) => onChange({ ...draft, name: event.target.value })}
                    placeholder="Ops dashboard"
                    autoComplete="off"
                />
            </label>
            <div className="workflow-member-list" id="board-cards">
                {active ? (
                    draft.cards.map((card, index) => (
                        <div className="workflow-member-editor board-card-editor" key={card.id}>
                            <label className="field"><span>Workspace</span><select
                                aria-label="Card workspace"
                                value={`${card.node_id}\u0000${card.session}`}
                                onChange={(event) => {
                                    const [node, session] = event.target.value.split("\u0000");
                                    onChange({
                                        ...draft,
                                        cards: draft.cards.map((item, i) =>
                                            i === index
                                                ? {
                                                      ...item,
                                                      node_id: node,
                                                      session,
                                                      task:
                                                          targets.find(
                                                              (target) =>
                                                                  target.node_id === node && target.session === session,
                                                          )?.tasks?.[0] ?? item.task,
                                                  }
                                                : item,
                                        ),
                                    });
                                }}>
                                {targets.map((target) => (
                                    <option
                                        key={`${target.node_id}-${target.session}`}
                                        value={`${target.node_id}\u0000${target.session}`}>
                                        {target.workspace_display_name}
                                    </option>
                                ))}
                            </select></label>
                            <label className="field"><span>Task</span><select
                                value={card.task}
                                onChange={(event) =>
                                    onChange({
                                        ...draft,
                                        cards: draft.cards.map((item, i) =>
                                            i === index ? { ...item, task: event.target.value } : item,
                                        ),
                                    })
                                }>
                                {(targets.find((target) => target.node_id === card.node_id && target.session === card.session)?.tasks ?? []).map((task) => <option key={task} value={task}>{task}</option>)}
                            </select></label>
                            <label className="field"><span>Default view</span><select
                                value={card.mode}
                                onChange={(event) =>
                                    onChange({
                                        ...draft,
                                        cards: draft.cards.map((item, i) =>
                                            i === index ? { ...item, mode: event.target.value as BoardCardMode } : item,
                                        ),
                                    })
                                }>
                                <option value="status">status</option>
                                <option value="logs">logs</option>
                                <option value="metrics">metrics</option>
                            </select></label>
                            <label className="check-field">
                                <input
                                    type="checkbox"
                                    checked={card.pinned}
                                    onChange={(event) =>
                                        onChange({
                                            ...draft,
                                            cards: draft.cards.map((item, i) =>
                                                i === index ? { ...item, pinned: event.target.checked } : item,
                                            ),
                                        })
                                    }
                                />
                                <span>Pin</span>
                            </label>
                            <button
                                className="icon-button"
                                type="button"
                                aria-label="Remove card"
                                onClick={() =>
                                    onChange({ ...draft, cards: draft.cards.filter((_, i) => i !== index) })
                                }>
                                ×
                            </button>
                        </div>
                    ))
                ) : (
                    <div className="muted">Select a board to edit, or create a new one.</div>
                )}
            </div>
            <button
                className="button"
                id="add-board-card"
                type="button"
                disabled={!active}
                onClick={() => targets[0] && onAdd(targets[0])}>
                Add card
            </button>
            <div
                className={`settings-message${boardMessage.kind ? ` ${boardMessage.kind}` : ""}`}
                id="board-message"
                role="status">
                {boardMessage.text}
            </div>
            <footer className="settings-footer">
                <button className="button" id="cancel-board" type="button" disabled={!active} onClick={onCancel}>
                    Cancel
                </button>
                <button className="button primary" id="save-board" type="button" disabled={!active} onClick={onSave}>
                    Save board
                </button>
            </footer>
        </aside>
    );
}
