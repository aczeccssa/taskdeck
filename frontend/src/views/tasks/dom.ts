import type { WorkspaceSummary } from "../../domain/models";

export function exitKey(node: string, project: string, task: string): string {
    return `${node}\u0000${project}\u0000${task}`;
}

export function setConnectionState(connected: boolean): void {
    const connection = document.getElementById("connection-state");
    if (!connection) return;
    connection.classList.toggle("offline", !connected);
    const label = connection.querySelector("span");
    if (label) label.textContent = connected ? "Daemon connected" : "Daemon unavailable";
}

export function setMetaText(text: string): void {
    const meta = document.getElementById("meta");
    if (meta) meta.textContent = text;
}

export function setSelectOptions(
    select: HTMLSelectElement | null,
    options: Array<{ value: string; label: string }>,
    selected: string,
    emptyLabel: string,
): void {
    if (!select) return;
    select.replaceChildren(
        ...options.map((option) => {
            const element = document.createElement("option");
            element.value = option.value;
            element.textContent = option.label;
            return element;
        }),
    );
    select.value = options.some((option) => option.value === selected) ? selected : (options[0]?.value ?? "");
}

export function sessionLabel(workspaces: readonly WorkspaceSummary[], session: string): string {
    const workspace = workspaces.find((item) => item.session === session);
    return workspace?.alias ? `${workspace.alias} · ${session}` : session;
}

export function sessionOptions(
    workspaces: readonly WorkspaceSummary[],
    sessions: readonly string[],
    emptyLabel: string,
): Array<{ value: string; label: string }> {
    return sessions.length
        ? sessions.map((session) => ({ value: session, label: sessionLabel(workspaces, session) }))
        : [{ value: "", label: emptyLabel }];
}

