import { useRef } from "react";
import type { BoardCard, BoardCardMode, BoardTemplate, BoardView, WorkflowTargetView } from "../../domain/models";
import type { TemplateMessage } from "./helpers";

export function BoardEditor({
    active,
    editing,
    draft,
    targets,
    boards,
    onChange,
    onAdd,
    onSave,
    onCancel,
    boardMessage,
    templates,
    templateName,
    templateId,
    sourceBoardId,
    templateMessage,
    onTemplateName,
    onSourceBoard,
    onTemplateSelect,
    onSaveTemplate,
    onApplyTemplate,
    onExportTemplate,
    onImportTemplate,
    onDeleteTemplate,
}: {
    active: boolean;
    editing: boolean;
    draft: { name: string; cards: BoardCard[] };
    targets: WorkflowTargetView[];
    boards: BoardView[];
    onChange: (draft: { name: string; cards: BoardCard[] }) => void;
    onAdd: (target: WorkflowTargetView) => void;
    onSave: () => void;
    onCancel: () => void;
    boardMessage: TemplateMessage;
    templates: BoardTemplate[];
    templateName: string;
    templateId: string | null;
    sourceBoardId: string;
    templateMessage: TemplateMessage;
    onTemplateName: (value: string) => void;
    onSourceBoard: (value: string) => void;
    onTemplateSelect: (value: string | null) => void;
    onSaveTemplate: () => void;
    onApplyTemplate: () => void;
    onExportTemplate: () => void;
    onImportTemplate: (file: File) => void;
    onDeleteTemplate: () => void;
}): React.JSX.Element {
    const fileInput = useRef<HTMLInputElement>(null);
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
                        <div className="workflow-member-editor" key={card.id}>
                            <select
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
                            </select>
                            <input
                                aria-label="Card task"
                                value={card.task}
                                onChange={(event) =>
                                    onChange({
                                        ...draft,
                                        cards: draft.cards.map((item, i) =>
                                            i === index ? { ...item, task: event.target.value } : item,
                                        ),
                                    })
                                }
                            />
                            <select
                                aria-label="Card view"
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
                            </select>
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
            <div className="board-templates">
                <header>
                    <h3>Board templates</h3>
                    <p>Save a board as a template, clone it later, or share the JSON with other nodes.</p>
                </header>
                <div className="field-grid compact-grid">
                    <label className="field">
                        <span>Template name</span>
                        <input
                            id="template-name"
                            type="text"
                            value={templateName}
                            onChange={(event) => onTemplateName(event.target.value)}
                            placeholder="Ops template"
                            autoComplete="off"
                        />
                    </label>
                    <label className="field">
                        <span>From board</span>
                        <select
                            id="template-source"
                            value={sourceBoardId}
                            onChange={(event) => onSourceBoard(event.target.value)}>
                            <option value="">—</option>
                            {boards.map((board) => (
                                <option key={board.id} value={board.id}>
                                    {board.name}
                                </option>
                            ))}
                        </select>
                    </label>
                </div>
                <div className="settings-actions">
                    <button className="button" id="save-template" type="button" onClick={onSaveTemplate}>
                        Save as template
                    </button>
                    <button className="button" id="apply-template" type="button" onClick={onApplyTemplate}>
                        Create board
                    </button>
                    <button className="button" id="export-template" type="button" onClick={onExportTemplate}>
                        Export
                    </button>
                    <button
                        className="button"
                        id="import-template"
                        type="button"
                        onClick={() => fileInput.current?.click()}>
                        Import
                    </button>
                    <input
                        id="import-template-file"
                        type="file"
                        accept="application/json,.json"
                        hidden
                        ref={fileInput}
                        onChange={(event) => {
                            const file = event.target.files?.[0];
                            if (file) onImportTemplate(file);
                            event.target.value = "";
                        }}
                    />
                    <button className="button" id="delete-template" type="button" onClick={onDeleteTemplate}>
                        Delete
                    </button>
                </div>
                <div className="template-list" id="board-template-list">
                    {templates.length ? (
                        templates.map((template) => (
                            <button
                                className={`template-row${templateId === template.id ? " selected" : ""}`}
                                type="button"
                                key={template.id}
                                data-template-select={template.id}
                                onClick={() => onTemplateSelect(template.id)}>
                                <strong>{template.name}</strong>
                                <span className="muted">
                                    {template.cards?.length ?? 0} cards
                                    {template.description ? ` · ${template.description}` : ""}
                                </span>
                            </button>
                        ))
                    ) : (
                        <div className="muted">No templates saved.</div>
                    )}
                </div>
                <div
                    className={`settings-message${templateMessage.kind ? ` ${templateMessage.kind}` : ""}`}
                    id="template-message"
                    role="status">
                    {templateMessage.text}
                </div>
            </div>
        </aside>
    );
}
