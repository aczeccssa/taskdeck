import type { WorkflowGroupMember, WorkflowTargetView } from "../../domain/models";
import { targetKey } from "./graph";
import type { DraftMember } from "./types";

export function WorkflowEditor({
    editing,
    name,
    setName,
    members,
    targets,
    onMember,
    onAdd,
    onRemove,
    onMove,
    onCancel,
    onSave,
    busy,
    message,
}: {
    editing: string | null;
    name: string;
    setName: (value: string) => void;
    members: DraftMember[];
    targets: WorkflowTargetView[];
    onMember: (index: number, patch: Partial<DraftMember>) => void;
    onAdd: () => void;
    onRemove: (index: number) => void;
    onMove: (from: number, to: number) => void;
    onCancel: () => void;
    onSave: () => void;
    busy: boolean;
    message: string;
}): React.JSX.Element {
    return (
        <aside className="workflow-editor">
            <header>
                <div>
                    <h2>{editing ? "Edit workflow" : "New workflow"}</h2>
                    <p>Define ordered task steps across nodes and workspaces.</p>
                </div>
                <button
                    className="icon-button editor-close"
                    type="button"
                    aria-label="Close workflow editor"
                    onClick={onCancel}>
                    ×
                </button>
            </header>
            <label className="field">
                <span>Name</span>
                <input
                    type="text"
                    value={name}
                    onChange={(event) => setName(event.target.value)}
                    placeholder="Release train"
                    autoComplete="off"
                />
            </label>
            <div className="workflow-member-list">
                {members.length ? (
                    members.map((member: DraftMember, index: number) => (
                        <div className="workflow-member-editor" key={index}>
                            <select
                                value={targets.findIndex(
                                    (target: WorkflowTargetView) =>
                                        target.node_id === member.node_id && target.session === member.session,
                                )}
                                onChange={(event) => {
                                    const target = targets[Number(event.target.value)];
                                    if (target)
                                        onMember(index, {
                                            node_id: target.node_id,
                                            session: target.session,
                                            task: target.tasks[0] ?? "",
                                        });
                                }}
                                aria-label="Workflow member workspace">
                                <option value={-1}>Cached or missing</option>
                                {targets.map((target: WorkflowTargetView, i: number) => (
                                    <option value={i} key={targetKey(target)}>
                                        {target.node_name} / {target.workspace_display_name} ({target.session})
                                    </option>
                                ))}
                            </select>
                            <select
                                value={member.task}
                                onChange={(event) => onMember(index, { task: event.target.value })}
                                aria-label="Workflow member task">
                                {(
                                    targets.find(
                                        (target: WorkflowTargetView) =>
                                            target.node_id === member.node_id && target.session === member.session,
                                    )?.tasks ?? [member.task]
                                ).map((task: string) => (
                                    <option value={task} key={task}>
                                        {task}
                                    </option>
                                ))}
                            </select>
                            <button
                                className="icon-button"
                                type="button"
                                onClick={() => onMove(index, index - 1)}
                                disabled={index === 0}
                                aria-label="Move member up">
                                ↑
                            </button>
                            <button
                                className="icon-button"
                                type="button"
                                onClick={() => onMove(index, index + 1)}
                                disabled={index === members.length - 1}
                                aria-label="Move member down">
                                ↓
                            </button>
                            <button
                                className="icon-button"
                                type="button"
                                onClick={() => onRemove(index)}
                                aria-label="Remove member">
                                ×
                            </button>
                        </div>
                    ))
                ) : (
                    <div className="muted">Add at least one workspace task member.</div>
                )}
            </div>
            <button className="button" type="button" onClick={onAdd} disabled={!editing && !targets.length}>
                Add member
            </button>
            <div className="settings-message" role="status">
                {message}
            </div>
            <footer className="settings-footer">
                <button className="button" type="button" onClick={onCancel}>
                    Cancel
                </button>
                <button className="button primary" type="button" onClick={onSave} disabled={busy}>
                    Save workflow
                </button>
            </footer>
        </aside>
    );
}
