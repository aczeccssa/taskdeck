import { type KeyboardEvent } from "react";
import type { SessionSnapshot } from "../../domain/models";
import { taskStateDot } from "./helpers";
import type { SeenExits } from "./types";

export function TaskTabs({
    tabOrderSaving,
    tabsRef,
    labels,
    snapshot,
    seenExits,
    currentTask,
    selectTask,
    persistWorkspaceOrder,
}: {
    tabOrderSaving: boolean;
    tabsRef: React.RefObject<HTMLDivElement | null>;
    labels: string[];
    snapshot: SessionSnapshot | null;
    seenExits: SeenExits;
    currentTask: string | null;
    selectTask: (label: string) => void;
    persistWorkspaceOrder: (order: string[], previousOrder: string[]) => Promise<void>;
}): React.JSX.Element {
    return (
        <div
                className={`task-tabs${tabOrderSaving ? " saving-order" : ""}`}
                id="tabs"
                role="tablist"
                aria-label="Tasks"
                ref={tabsRef}
                onClick={(event) => {
                    const button = (event.target as Element).closest<HTMLElement>("[data-task]");
                    if (button?.dataset.task) selectTask(button.dataset.task);
                }}
                onKeyDown={(event: KeyboardEvent<HTMLDivElement>) => {
                    if (!["ArrowLeft", "ArrowRight"].includes(event.key)) return;
                    const tabs = Array.from(document.querySelectorAll<HTMLButtonElement>("#tabs [data-task]"));
                    const index = tabs.indexOf(document.activeElement as HTMLButtonElement);
                    if (index < 0 || !tabs.length) return;
                    event.preventDefault();
                    if (event.altKey) {
                        const target = index + (event.key === "ArrowRight" ? 1 : -1);
                        if (target < 0 || target >= tabs.length) return;
                        const previous = tabs.map((tab) => tab.dataset.task ?? "");
                        const order = [...previous];
                        [order[index], order[target]] = [order[target], order[index]];
                        const activeTab = tabs[index];
                        void persistWorkspaceOrder(order, previous);
                        window.requestAnimationFrame(() => activeTab.focus());
                        return;
                    }
                    const next = tabs[(index + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length];
                    next.click();
                    next.focus();
                }}>
                {labels.map((label) => {
                    const tabTask = snapshot?.tasks[label];
                    const dot = tabTask ? taskStateDot(label, tabTask, seenExits) : null;
                    return (
                        <button
                            className={`tab${label === currentTask ? " active" : ""}`}
                            key={label}
                            type="button"
                            role="tab"
                            tabIndex={label === currentTask ? 0 : -1}
                            aria-selected={label === currentTask}
                            aria-label={`${label}${dot ? `, ${dot.label}` : ""}`}
                            data-task={label}
                            data-order-key={label}>
                            {dot ? <span className={`task-state-dot ${dot.className}`} title={dot.label} /> : null}
                            <span>{label}</span>
                        </button>
                    );
                })}
            </div>
    );
}
