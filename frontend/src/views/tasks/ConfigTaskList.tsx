import type { ConfigTaskDraft } from "./helpers";
import { gripIcon } from "./icons";

export function ConfigTaskList({
    tasks,
    taskIndex,
    listRef,
    onSelect,
    onAdd,
}: {
    tasks: ConfigTaskDraft[];
    taskIndex: number;
    listRef: React.RefObject<HTMLElement | null>;
    onSelect: (index: number) => void;
    onAdd: () => void;
}): React.JSX.Element {
    return (
        <aside className="config-task-list" id="config-task-list" ref={listRef}>
            <div>
                {tasks.length ? (
                    tasks.map((task, index) => (
                        <div className="config-task-item" data-order-key={task._key} key={task._key}>
                            <button
                                className="drag-handle"
                                type="button"
                                data-drag-handle
                                aria-label={`Drag ${task.label || "Untitled task"}`}
                                title="Drag to reorder">
                                {gripIcon}
                            </button>
                            <button
                                className={`config-task-button${index === taskIndex ? " active" : ""}`}
                                type="button"
                                data-config-task={index}
                                onClick={() => onSelect(index)}>
                                <span>{task.label || "Untitled task"}</span>
                                <small>{task.origin.imported ? "VS Code import" : "YAML task"}</small>
                            </button>
                        </div>
                    ))
                ) : (
                    <div className="monitor-empty">No tasks</div>
                )}
            </div>
            <button className="button" id="add-config-task" type="button" onClick={onAdd}>
                Add task
            </button>
        </aside>
    );
}
