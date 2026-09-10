import { useEffect, useRef, useState, type FormEvent } from "react";
import type { EditableTask, NodeSummary } from "../../domain/models";
import { showToast } from "../../lib/toast";
import { LegacyApiAdapter } from "../../compat/api";
import {
    configSaveFailure,
    decodeConfig,
    decodeConfigOrFailure,
} from "../../api/session";
import { requireData } from "./utils";
import { taskToDraft, validateConfigTasks, validateWorkspaceEnv, type ConfigTaskDraft } from "./helpers";
import type { SortDrag } from "./types";
import { useConfigTaskDrag } from "./useConfigTaskDrag";
import { ConfigTaskList } from "./ConfigTaskList";
import { ConfigTaskForm } from "./ConfigTaskForm";
import { ConfigWorkspaceEnv } from "./ConfigWorkspaceEnv";

const api = new LegacyApiAdapter();

export function ConfigDialog({
    open,
    setOpen,
    node,
    session,
    currentTask,
    nodes,
    onSnapshotRefresh,
    registerGuard,
}: {
    open: boolean;
    setOpen: (open: boolean) => void;
    node: string;
    session: string;
    currentTask: string | null;
    nodes: NodeSummary[];
    onSnapshotRefresh: () => void;
    registerGuard: (guard: () => boolean) => void;
}): React.JSX.Element {
    const dialogRef = useRef<HTMLDialogElement>(null);
    const requestRef = useRef(0);
    const [config, setConfig] = useState<{
        session: string;
        project: string;
        source: string;
        revision: string;
        node: string;
    } | null>(null);
    const [tasks, setTasks] = useState<ConfigTaskDraft[]>([]);
    const [workspaceEnvRows, setWorkspaceEnvRows] = useState<Array<{ key: string; value: string }>>([]);
    const [taskIndex, setTaskIndex] = useState(0);
    const [dirty, setDirty] = useState(false);
    const [saving, setSaving] = useState(false);
    const [message, setMessage] = useState("");
    const [messageError, setMessageError] = useState(false);
    const [reloadVisible, setReloadVisible] = useState(false);
    const sortDrag = useRef<SortDrag | null>(null);
    const configListRef = useRef<HTMLElement>(null);
    useConfigTaskDrag({
        configListRef, sortDrag, tasks, taskIndex, setTasks, setTaskIndex, setDirty,
    });

    const showMessage = (value: string, error = false, reload = false): void => {
        setMessage(value);
        setMessageError(error);
        setReloadVisible(reload);
    };
    const requestClose = (): boolean => {
        const dialog = dialogRef.current;
        if (!dialog?.open) return true;
        if (saving) {
            showMessage("Wait for the current save to finish.", true);
            return false;
        }
        if (dirty && !window.confirm("Discard unsaved configuration changes?")) return false;
        requestRef.current += 1;
        setDirty(false);
        setOpen(false);
        dialog.close();
        return true;
    };
    useEffect(() => {
        registerGuard(requestClose);
    });

    useEffect(() => {
        const dialog = dialogRef.current;
        if (!dialog) return;
        if (open && !dialog.open) dialog.showModal();
        if (!open && dialog.open) dialog.close();
    }, [open]);
    const loadConfig = async (confirmDiscard = false): Promise<void> => {
        if (saving) return;
        if (confirmDiscard && dirty && !window.confirm("Discard unsaved configuration changes and reload?")) return;
        const requestId = requestRef.current + 1;
        requestRef.current = requestId;
        showMessage("Loading configuration...");
        try {
            const response = await api.request(
                `/api/sessions/${encodeURIComponent(session)}/config?${new URLSearchParams({ node })}`,
                decodeConfig,
            );
            if (
                requestId !== requestRef.current ||
                !dialogRef.current?.open ||
                session !== dialogRef.current.dataset.session
            )
                return;
            const data = requireData(response);
            setConfig({
                session: data.session,
                project: data.project,
                source: data.source,
                revision: data.revision,
                node,
            });
            setTasks(data.tasks.map((task) => taskToDraft(task)));
            setWorkspaceEnvRows(Object.entries(data.workspace_env).map(([key, value]) => ({ key, value })));
            const currentTask = dialogRef.current.dataset.currentTask;
            const index = data.tasks.findIndex((task: EditableTask) => task.label === currentTask);
            setTaskIndex(Math.max(0, index));
            setDirty(false);
            showMessage("");
            setMessageError(false);
            setReloadVisible(false);
        } catch (error) {
            if (requestId === requestRef.current)
                showMessage(error instanceof Error ? error.message : "Unable to load configuration", true);
        }
    };
    useEffect(() => {
        const dialog = dialogRef.current;
        if (!dialog) return;
        dialog.dataset.session = session;
        dialog.dataset.currentTask = currentTask ?? "";
        if (open && session) void loadConfig(false);
    }, [open, session, node]);

    const addTask = (): void => {
        const nextTasks = [
            ...tasks,
            taskToDraft({
                label: "new-task",
                command: "",
                shell: true,
                auto_start: false,
                stop_timeout_ms: 3000,
                origin: { imported: false, has_yaml_override: false },
            }),
        ];
        setTasks(nextTasks);
        setTaskIndex(nextTasks.length - 1);
        setDirty(true);
    };
    const updateTask = (index: number, update: (task: ConfigTaskDraft) => ConfigTaskDraft): void => {
        setTasks((current) => current.map((task, taskPosition) => (taskPosition === index ? update(task) : task)));
        setDirty(true);
    };
    const save = async (event: FormEvent<HTMLFormElement>): Promise<void> => {
        event.preventDefault();
        if (!config || saving) return;
        let validated;
        let workspaceEnv;
        try {
            validated = validateConfigTasks(tasks);
            workspaceEnv = validateWorkspaceEnv(workspaceEnvRows);
        } catch (error) {
            showMessage(error instanceof Error ? error.message : "Unable to save configuration", true);
            return;
        }
        setSaving(true);
        showMessage("");
        setMessageError(false);
        setReloadVisible(false);
        try {
            if (session !== config.session || node !== config.node)
                throw new Error("The selected node or session changed. Reopen configuration before saving.");
            const response = await api.request(
                `/api/sessions/${encodeURIComponent(session)}/config?${new URLSearchParams({ node })}`,
                decodeConfigOrFailure,
                {
                    method: "PUT",
                    headers: { "content-type": "application/json" },
                    body: JSON.stringify({ revision: config.revision, workspace_env: workspaceEnv, tasks: validated }),
                },
            );
            if (!response.ok) {
                const failure = configSaveFailure(response.data);
                if (failure?.kind === "stale_revision") {
                    showMessage("The file changed outside Taskdeck. Reload before applying again.", true, true);
                    return;
                }
                if (failure?.kind === "reconciliation_error" && failure.saved) {
                    setConfig({ ...config, revision: failure.current_revision || config.revision });
                    setDirty(false);
                    showMessage(
                        "Saved to taskdeck.yaml, but one or more live sessions could not reconcile. Retry after checking the affected tasks.",
                        true,
                        true,
                    );
                    onSnapshotRefresh();
                    return;
                }
                throw new Error(response.message);
            }
            const data = requireData(response);
            if (!data || !("session" in data)) throw new Error(response.message);
            setConfig({ ...config, revision: data.revision });
            setTasks(data.tasks.map((task) => taskToDraft(task)));
            setWorkspaceEnvRows(Object.entries(data.workspace_env).map(([key, value]) => ({ key, value })));
            setDirty(false);
            showToast("Configuration applied");
            setOpen(false);
            dialogRef.current?.close();
            onSnapshotRefresh();
        } catch (error) {
            showMessage(error instanceof Error ? error.message : "Unable to save configuration", true);
        } finally {
            setSaving(false);
        }
    };
    const nodeSummary = nodes.find((candidate) => candidate.id === config?.node);
    const selectedTask = tasks[taskIndex];
    return (
        <dialog
            className="drawer config-drawer"
            id="config-dialog"
            aria-labelledby="config-dialog-title"
            ref={dialogRef}
            onCancel={(event) => {
                event.preventDefault();
                requestClose();
            }}
            onClick={(event) => {
                if (event.target === dialogRef.current) requestClose();
            }}
            onClose={() => setOpen(false)}>
            <div className="drawer-shell">
                <header className="drawer-header">
                    <div>
                        <span>Registered configuration</span>
                        <h2 id="config-dialog-title">Edit tasks</h2>
                    </div>
                    <button
                        className="icon-button"
                        id="close-config"
                        type="button"
                        aria-label="Close configuration"
                        title="Close"
                        onClick={() => requestClose()}>
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                            <path d="m7 7 10 10M17 7 7 17" />
                        </svg>
                    </button>
                </header>
                <div
                    className={`config-message${messageError ? " error" : ""}`}
                    id="config-message"
                    role="alert"
                    aria-live="assertive"
                    hidden={!message}>
                    {message}
                </div>
            <div className="config-layout">
                <ConfigTaskList
                    tasks={tasks}
                    taskIndex={taskIndex}
                    listRef={configListRef}
                    onSelect={setTaskIndex}
                    onAdd={addTask}
                />
                <form className="config-form" id="config-form" onSubmit={save}>
                    <ConfigWorkspaceEnv rows={workspaceEnvRows} setRows={setWorkspaceEnvRows} setDirty={setDirty} />
                    <ConfigTaskForm
                        selectedTask={selectedTask}
                        taskIndex={taskIndex}
                        tasks={tasks}
                        updateTask={updateTask}
                        setTasks={setTasks}
                        setTaskIndex={setTaskIndex}
                        setDirty={setDirty}
                    />
                    <footer>
                        <button
                            className="button"
                            id="reload-config"
                            type="button"
                            hidden={!reloadVisible}
                            onClick={() => void loadConfig(true)}>
                            Reload
                        </button>
                        <button className="button primary" id="save-config" type="submit" disabled={saving}>
                            Apply
                        </button>
                    </footer>
                </form>
            </div>
        </div>
    </dialog>
    );
}