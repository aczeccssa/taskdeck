import type { ConfigTaskDraft } from "./helpers";

export function ConfigTaskForm({
    selectedTask,
    taskIndex,
    tasks,
    updateTask,
    setTasks,
    setTaskIndex,
    setDirty,
}: {
    selectedTask: ConfigTaskDraft | undefined;
    taskIndex: number;
    tasks: ConfigTaskDraft[];
    updateTask: (index: number, update: (task: ConfigTaskDraft) => ConfigTaskDraft) => void;
    setTasks: (tasks: ConfigTaskDraft[]) => void;
    setTaskIndex: (index: number) => void;
    setDirty: (dirty: boolean) => void;
}): React.JSX.Element {
    return (
        <div className="config-form-body" id="config-form-body">
            {selectedTask ? (
                <>
                    <label className="field">
                        <span>Label</span>
                        <input
                            data-field="label"
                            value={selectedTask.label}
                            required
                            onChange={(event) =>
                                updateTask(taskIndex, (task) => ({
                                    ...task,
                                    label: event.target.value,
                                }))
                            }
                        />
                    </label>
                    <label className="field">
                        <span>Command</span>
                        <input
                            data-field="command"
                            value={selectedTask.command}
                            required
                            onChange={(event) =>
                                updateTask(taskIndex, (task) => ({
                                    ...task,
                                    command: event.target.value,
                                }))
                            }
                        />
                    </label>
                    <label className="field">
                        <span>Cron schedule (5 or 6 fields; empty disables)</span>
                        <input
                            data-field="schedule"
                            value={selectedTask.schedule || ""}
                            placeholder="*/10 * * * *"
                            onChange={(event) =>
                                updateTask(taskIndex, (task) => ({
                                    ...task,
                                    schedule: event.target.value,
                                }))
                            }
                        />
                    </label>
                    <div className="field-grid">
                        <label className="field">
                            <span>Working directory</span>
                            <input
                                data-field="cwd"
                                value={selectedTask.cwd}
                                onChange={(event) =>
                                    updateTask(taskIndex, (task) => ({
                                        ...task,
                                        cwd: event.target.value,
                                    }))
                                }
                            />
                        </label>
                        <label className="field">
                            <span>Stop timeout (ms)</span>
                            <input
                                data-field="stop_timeout_ms"
                                type="number"
                                min={1}
                                max={300000}
                                value={selectedTask.stop_timeout_ms}
                                onChange={(event) =>
                                    updateTask(taskIndex, (task) => ({
                                        ...task,
                                        stop_timeout_ms: Number(event.target.value),
                                    }))
                                }
                            />
                        </label>
                    </div>
                    <div className="toggle-row">
                        <label>
                            <input
                                data-field="shell"
                                type="checkbox"
                                checked={selectedTask.shell}
                                onChange={(event) =>
                                    updateTask(taskIndex, (task) => ({
                                        ...task,
                                        shell: event.target.checked,
                                    }))
                                }
                            />{" "}
                            Run through shell
                        </label>
                        <label>
                            <input
                                data-field="auto_start"
                                type="checkbox"
                                checked={selectedTask.auto_start}
                                onChange={(event) =>
                                    updateTask(taskIndex, (task) => ({
                                        ...task,
                                        auto_start: event.target.checked,
                                    }))
                                }
                            />{" "}
                            Auto start
                        </label>
                        <label>
                            <input
                                data-field="clear_logs_on_restart"
                                type="checkbox"
                                checked={selectedTask.clear_logs_on_restart}
                                onChange={(event) =>
                                    updateTask(taskIndex, (task) => ({
                                        ...task,
                                        clear_logs_on_restart: event.target.checked,
                                    }))
                                }
                            />{" "}
                            Clear logs and performance history on restart
                        </label>
                    </div>
                    <div className="origin-note">
                        {selectedTask.origin.imported
                            ? "Imported from .vscode/tasks.json; Taskdeck saves only overrides."
                            : "Defined in taskdeck.yaml."}
                    </div>
                    <fieldset className="field">
                        <legend>Arguments</legend>
                        <div className="repeater" id="args-rows">
                            {selectedTask.args.map((argument, index) => (
                                <div className="repeater-row" key={index}>
                                    <input
                                        data-arg={index}
                                        value={argument}
                                        aria-label={`Argument ${index + 1}`}
                                        onChange={(event) =>
                                            updateTask(taskIndex, (task) => ({
                                                ...task,
                                                args: task.args.map((item, position) =>
                                                    position === index ? event.target.value : item,
                                                ),
                                            }))
                                        }
                                    />
                                    <button
                                        className="icon-button"
                                        type="button"
                                        data-remove-arg={index}
                                        aria-label="Remove argument"
                                        title="Remove"
                                        onClick={() =>
                                            updateTask(taskIndex, (task) => ({
                                                ...task,
                                                args: task.args.filter(
                                                    (_, position) => position !== index,
                                                ),
                                            }))
                                        }>
                                        ×
                                    </button>
                                </div>
                            ))}
                        </div>
                        <button
                            className="button compact"
                            type="button"
                            data-add-arg
                            onClick={() =>
                                updateTask(taskIndex, (task) => ({ ...task, args: [...task.args, ""] }))
                            }>
                            Add argument
                        </button>
                    </fieldset>
                    <fieldset className="field">
                        <legend>Environment</legend>
                        <div className="repeater" id="env-rows">
                            {selectedTask.envRows.map((row, index) => (
                                <div className="repeater-row env" key={index}>
                                    <input
                                        data-env-key={index}
                                        value={row.key}
                                        placeholder="NAME"
                                        aria-label="Environment key"
                                        onChange={(event) =>
                                            updateTask(taskIndex, (task) => ({
                                                ...task,
                                                envRows: task.envRows.map((item, position) =>
                                                    position === index
                                                        ? { ...item, key: event.target.value }
                                                        : item,
                                                ),
                                            }))
                                        }
                                    />
                                    <input
                                        data-env-value={index}
                                        value={row.value}
                                        placeholder="Value"
                                        aria-label="Environment value"
                                        onChange={(event) =>
                                            updateTask(taskIndex, (task) => ({
                                                ...task,
                                                envRows: task.envRows.map((item, position) =>
                                                    position === index
                                                        ? { ...item, value: event.target.value }
                                                        : item,
                                                ),
                                            }))
                                        }
                                    />
                                    <button
                                        className="icon-button"
                                        type="button"
                                        data-remove-env={index}
                                        aria-label="Remove environment variable"
                                        title="Remove"
                                        onClick={() =>
                                            updateTask(taskIndex, (task) => ({
                                                ...task,
                                                envRows: task.envRows.filter(
                                                    (_, position) => position !== index,
                                                ),
                                            }))
                                        }>
                                        ×
                                    </button>
                                </div>
                            ))}
                        </div>
                        <button
                            className="button compact"
                            type="button"
                            data-add-env
                            onClick={() =>
                                updateTask(taskIndex, (task) => ({
                                    ...task,
                                    envRows: [...task.envRows, { key: "", value: "" }],
                                }))
                            }>
                            Add variable
                        </button>
                    </fieldset>
                    <button
                        className="button danger"
                        type="button"
                        data-delete-task
                        onClick={() => {
                            const next = tasks.filter((_, index) => index !== taskIndex);
                            setTasks(next);
                            setTaskIndex(Math.max(0, Math.min(taskIndex, next.length - 1)));
                            setDirty(true);
                        }}>
                        Delete task
                    </button>
                </>
            ) : (
                <div className="monitor-empty">Add a task to begin.</div>
            )}
        </div>
    );
}
