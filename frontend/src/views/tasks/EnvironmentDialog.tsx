import { useEffect, useRef } from "react";
import type { ConfigTaskDraft } from "./helpers";

type EnvironmentScope = "task" | "workspace";
type EnvironmentRow = { key: string; value: string };

export function EnvironmentDialog({
    open,
    setOpen,
    scope,
    setScope,
    selectedTask,
    updateTask,
    workspaceRows,
    setWorkspaceRows,
    setDirty,
}: {
    open: boolean;
    setOpen: (open: boolean) => void;
    scope: EnvironmentScope;
    setScope: (scope: EnvironmentScope) => void;
    selectedTask: ConfigTaskDraft | undefined;
    updateTask: (update: (task: ConfigTaskDraft) => ConfigTaskDraft) => void;
    workspaceRows: EnvironmentRow[];
    setWorkspaceRows: (rows: EnvironmentRow[]) => void;
    setDirty: (dirty: boolean) => void;
}): React.JSX.Element {
    const dialogRef = useRef<HTMLDialogElement>(null);

    useEffect(() => {
        const dialog = dialogRef.current;
        if (!dialog) return;
        if (open && !dialog.open) dialog.showModal();
        if (!open && dialog.open) dialog.close();
    }, [open]);

    const rows = scope === "workspace" ? workspaceRows : selectedTask?.envRows ?? [];
    const setRows = (nextRows: EnvironmentRow[]): void => {
        if (scope === "workspace") setWorkspaceRows(nextRows);
        else updateTask((task) => ({ ...task, envRows: nextRows }));
        setDirty(true);
    };
    const scopeLabel = scope === "workspace" ? "Workspace" : selectedTask?.label || "Current task";

    return (
        <dialog
            className="environment-dialog"
            aria-labelledby="environment-dialog-title"
            ref={dialogRef}
            onCancel={(event) => {
                event.preventDefault();
                setOpen(false);
            }}
            onClick={(event) => {
                if (event.target === dialogRef.current) setOpen(false);
            }}>
            <div className="environment-dialog-shell">
                <header className="environment-dialog-header">
                    <div>
                        <span>Configuration</span>
                        <h3 id="environment-dialog-title">Environment variables</h3>
                    </div>
                    <button
                        className="icon-button"
                        type="button"
                        aria-label="Close environment variables"
                        title="Close"
                        onClick={() => setOpen(false)}>
                        ×
                    </button>
                </header>
                <div className="environment-scope" role="group" aria-label="Environment variable scope">
                    <button
                        className={`button compact${scope === "task" ? " active" : ""}`}
                        type="button"
                        aria-pressed={scope === "task"}
                        disabled={!selectedTask}
                        onClick={() => setScope("task")}>
                        Current task
                    </button>
                    <button
                        className={`button compact${scope === "workspace" ? " active" : ""}`}
                        type="button"
                        aria-pressed={scope === "workspace"}
                        onClick={() => setScope("workspace")}>
                        Workspace
                    </button>
                </div>
                <div className="environment-dialog-body">
                    <p className="environment-scope-note">Editing variables for {scopeLabel}.</p>
                    <div className="repeater" id="environment-variable-rows">
                        {rows.map((row, index) => (
                            <div className="repeater-row env" key={index}>
                                <input
                                    value={row.key}
                                    placeholder="NAME"
                                    aria-label={`Environment key ${index + 1}`}
                                    onChange={(event) =>
                                        setRows(rows.map((item, position) =>
                                            position === index ? { ...item, key: event.target.value } : item,
                                        ))
                                    }
                                />
                                <input
                                    value={row.value}
                                    placeholder="Value"
                                    aria-label={`Environment value ${index + 1}`}
                                    onChange={(event) =>
                                        setRows(rows.map((item, position) =>
                                            position === index ? { ...item, value: event.target.value } : item,
                                        ))
                                    }
                                />
                                <button
                                    className="icon-button"
                                    type="button"
                                    aria-label={`Remove environment variable ${index + 1}`}
                                    title="Remove"
                                    onClick={() => setRows(rows.filter((_, position) => position !== index))}>
                                    ×
                                </button>
                            </div>
                        ))}
                    </div>
                    <button
                        className="button compact"
                        type="button"
                        onClick={() => setRows([...rows, { key: "", value: "" }])}>
                        Add variable
                    </button>
                </div>
                <footer className="environment-dialog-footer">
                    <button className="button primary" type="button" onClick={() => setOpen(false)}>
                        Done
                    </button>
                </footer>
            </div>
        </dialog>
    );
}
