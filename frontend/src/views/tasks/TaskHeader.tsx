import type { SessionSnapshot } from "../../domain/models";
import { actionIcon, settingsIcon } from "./icons";
import { endpointElement } from "./utils";
import type { WorkspaceMode } from "./types";

export function TaskHeader({
    task,
    online,
    status,
    technology,
    technologyLabel,
    workspaceMode,
    setWorkspaceMode,
    busyAction,
    canStart,
    canPause,
    canResume,
    canStop,
    act,
    setConfigOpen,
}: {
    task: SessionSnapshot["tasks"][string];
    online: boolean;
    status: string;
    technology: { framework?: string; runtime?: string; evidence?: string[] } | null | undefined;
    technologyLabel: string;
    workspaceMode: WorkspaceMode;
    setWorkspaceMode: (mode: WorkspaceMode) => void;
    busyAction: "start" | "pause" | "resume" | "restart" | "stop" | null;
    canStart: boolean;
    canPause: boolean;
    canResume: boolean;
    canStop: boolean;
    act: (action: "start" | "pause" | "resume" | "restart" | "stop", button: HTMLButtonElement) => Promise<void>;
    setConfigOpen: (open: boolean) => void;
}): React.JSX.Element {
    return (
                        <div className="task-header" id="task-header">
                            <div className="task-heading">
                                <div className="eyebrow">{online ? "Selected task" : "Offline snapshot"}</div>
                                <h1>{task.label}</h1>
                                <div className="task-detail">
                                    <span className={`status ${status}`}>{status}</span>
                                    {task.pid ? <span>PID {task.pid}</span> : null}
                                    {technologyLabel ? (
                                        <span
                                            className="technology-chip"
                                            title={((technology as { evidence?: string[] })?.evidence || []).join(" · ")}>
                                            {technologyLabel}
                                        </span>
                                    ) : null}
                                    {(task.service?.endpoints || []).map(endpointElement)}
                                    <span className="cwd">{task.cwd}</span>
                                </div>
                            </div>
                            <div className="view-modes" aria-label="Workspace layout">
                                {(["log", "split", "monitor"] as const).map((mode) => {
                                    const active =
                                        mode === workspaceMode ||
                                        (window.matchMedia("(max-width: 820px)").matches &&
                                            mode === "log" &&
                                            workspaceMode === "split");
                                    return (
                                        <button
                                            key={mode}
                                            type="button"
                                            data-mode={mode}
                                            className={`${mode === "split" ? "desktop-split-mode" : ""}${active ? " active" : ""}`}
                                            aria-pressed={active}>
                                            {mode === "log" ? "Logs" : mode === "split" ? "Split" : "Monitor"}
                                        </button>
                                    );
                                })}
                            </div>
                            <div className="actions">
                                <button
                                    className="button primary"
                                    type="button"
                                    data-action="start"
                                    disabled={!online || !canStart || busyAction === "start"}>
                                    {actionIcon("start")}Start
                                </button>
                                <button
                                    className="button"
                                    type="button"
                                    data-action="pause"
                                    disabled={!online || !canPause || busyAction === "pause"}>
                                    {actionIcon("pause")}Pause
                                </button>
                                <button
                                    className="button"
                                    type="button"
                                    data-action="resume"
                                    disabled={!online || !canResume || busyAction === "resume"}>
                                    {actionIcon("restart")}Resume
                                </button>
                                <button
                                    className="button"
                                    type="button"
                                    data-action="restart"
                                    disabled={!online || busyAction === "restart"}>
                                    {actionIcon("restart")}Restart
                                </button>
                                <button
                                    className="button danger"
                                    type="button"
                                    data-action="stop"
                                    disabled={!online || !canStop || busyAction === "stop"}>
                                    {actionIcon("stop")}Stop
                                </button>
                                <button
                                    className="icon-button"
                                    type="button"
                                    data-config
                                    aria-label="Edit configuration"
                                    title="Edit configuration"
                                    disabled={!online}>
                                    {settingsIcon}
                                </button>
                            </div>
                        </div>
    );
}
