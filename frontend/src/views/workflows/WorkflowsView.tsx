import { useCallback, useEffect, useState } from "react";
import { formatTimestamp } from "../../lib/helpers";
import { reorder } from "../../lib/helpers";
import { showToast } from "../../lib/toast";
import type { TaskAction, TaskDependencyView, WorkflowGraph, WorkflowGroup, WorkflowRevision, WorkflowTargetView } from "../../domain/models";
import { api } from "../shared/api";
import { emptyGraph, normalizedGraph, targetKey } from "./graph";
import type { DraftMember, Summary } from "./types";
import { WorkflowCard } from "./WorkflowCard";
import { WorkflowEditor } from "./WorkflowEditor";
import { Orchestrator } from "./Orchestrator";
import { Dependencies } from "./Dependencies";

const passthrough = (value: unknown): unknown => value;

export function WorkflowsView(): React.JSX.Element {
    const [groups, setGroups] = useState<WorkflowGroup[]>([]);
    const [targets, setTargets] = useState<WorkflowTargetView[]>([]);
    const [ungrouped, setUngrouped] = useState<WorkflowTargetView[]>([]);
    const [editing, setEditing] = useState<string | null>(null);
    const [editorActive, setEditorActive] = useState(false);
    const [name, setName] = useState("");
    const [members, setMembers] = useState<DraftMember[]>([]);
    const [graph, setGraph] = useState<WorkflowGraph>(emptyGraph);
    const [message, setMessage] = useState("");
    const [busy, setBusy] = useState(false);
    const [summary, setSummary] = useState<Summary | null>(null);
    const [dependencies, setDependencies] = useState<TaskDependencyView[]>([]);
    const [dependencyTargets, setDependencyTargets] = useState<WorkflowTargetView[]>([]);
    const [revisions, setRevisions] = useState<WorkflowRevision[]>([]);
    const [showRevisions, setShowRevisions] = useState(false);
    const load = useCallback(async () => {
        const result = await api.request("/api/workflow-groups", passthrough);
        if (!result.ok) {
            setMessage(result.message || "Workflows unavailable");
            return;
        }
        const data = (result.data ?? {}) as {
            groups?: WorkflowGroup[];
            targets?: WorkflowTargetView[];
            ungrouped?: WorkflowTargetView[];
        };
        setGroups(data.groups ?? []);
        setTargets(data.targets ?? []);
        setUngrouped(data.ungrouped ?? []);
    }, []);
    const loadDependencies = useCallback(async () => {
        const result = await api.request("/api/dependencies", passthrough);
        if (result.ok) {
            const data = (result.data ?? {}) as { dependencies?: TaskDependencyView[]; targets?: WorkflowTargetView[] };
            setDependencies(data.dependencies ?? []);
            setDependencyTargets(data.targets ?? []);
        }
    }, []);
    useEffect(() => {
        void load();
        void loadDependencies();
        const timer = window.setInterval(() => {
            void load();
            void loadDependencies();
        }, 5000);
        return () => window.clearInterval(timer);
    }, [load, loadDependencies]);
    const open = (group?: WorkflowGroup) => {
        setEditorActive(true);
        setEditing(group?.id ?? null);
        setName(group?.name ?? "");
        setMembers(
            (group?.members ?? []).map((item) => ({ node_id: item.node_id, session: item.session, task: item.task })),
        );
        setGraph({ positions: [...(group?.graph?.positions ?? [])], edges: [...(group?.graph?.edges ?? [])] });
        setSummary(null);
        setMessage("");
        setShowRevisions(false);
    };
    const close = () => {
        setEditorActive(false);
        setEditing(null);
        setName("");
        setMembers([]);
        setGraph(emptyGraph());
        setSummary(null);
        setShowRevisions(false);
    };
    const updateMember = (index: number, patch: Partial<DraftMember>) =>
        setMembers((current) => current.map((member, i) => (i === index ? { ...member, ...patch } : member)));
    const save = async () => {
        if (!name.trim()) {
            setMessage("Workflow name is required.");
            return;
        }
        if (members.some((member) => !member.node_id || !member.session || !member.task)) {
            setMessage("Every member needs a workspace and task.");
            return;
        }
        setBusy(true);
        const result = await api.request(
            editing ? `/api/workflow-groups/${encodeURIComponent(editing)}` : "/api/workflow-groups",
            passthrough,
            {
                method: editing ? "PUT" : "POST",
                headers: { "content-type": "application/json" },
                body: JSON.stringify({ name: name.trim(), members, graph: normalizedGraph(graph, members.length) }),
            },
        );
        setBusy(false);
        if (!result.ok) {
            setMessage(result.message || "Unable to save workflow");
            return;
        }
        setMessage("Workflow saved.");
        close();
        await load();
    };
    const remove = async (id: string, groupName: string) => {
        if (!window.confirm(`Delete workflow '${groupName}'?`)) return;
        const result = await api.request(`/api/workflow-groups/${encodeURIComponent(id)}`, passthrough, {
            method: "DELETE",
        });
        if (!result.ok) setMessage(result.message || "Unable to delete workflow");
        else {
            if (editing === id) close();
            await load();
        }
    };
    const action = async (id: string, operation: TaskAction, count: number) => {
        if (!window.confirm(`${operation} ${count} workflow member${count === 1 ? "" : "s"}?`)) return;
        const result = await api.request(`/api/workflow-groups/${encodeURIComponent(id)}/actions`, passthrough, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ action: operation }),
        });
        if (!result.ok) setMessage(result.message || "Workflow action failed");
        else {
            setSummary((result.data ?? {}) as Summary);
            await load();
        }
    };
    const run = async () => {
        if (!editing) return;
        const result = await api.request(`/api/workflow-groups/${encodeURIComponent(editing)}/run`, passthrough, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({}),
        });
        if (!result.ok) setMessage(result.message || "Workflow run failed");
        else setSummary((result.data ?? {}) as Summary);
    };
    const history = async () => {
        if (!editing) return;
        const result = await api.request(`/api/workflow-groups/${encodeURIComponent(editing)}/revisions`, passthrough);
        if (result.ok) {
            setRevisions(((result.data ?? {}) as { revisions?: WorkflowRevision[] }).revisions ?? []);
            setShowRevisions(true);
        }
    };
    const restore = async (revision: number) => {
        if (!editing) return;
        const result = await api.request(
            `/api/workflow-groups/${encodeURIComponent(editing)}/revisions/${revision}/restore`,
            passthrough,
            { method: "POST" },
        );
        if (!result.ok) setMessage(result.message || "Unable to restore revision");
        else {
            showToast(`Restored revision ${revision}`);
            await load();
            const group = groups.find((item) => item.id === editing);
            if (group) open(group);
        }
    };
    return (
        <section
            className={`view workflows-view react-view ${editorActive ? "editor-open" : ""}`}
            data-react-owned="true">
            <div className="workflow-layout">
                <div className="workflow-main">
                    <header className="section-heading">
                        <div className="settings-actions">
                            <button className="button" type="button" onClick={() => void load()}>
                                Refresh
                            </button>
                            <button className="button primary" type="button" onClick={() => open()}>
                                New workflow
                            </button>
                        </div>
                    </header>
                    <div className="workflow-groups">
                        {groups.length ? (
                            groups.map((group) => (
                                <WorkflowCard
                                    key={group.id}
                                    group={group}
                                    onOpen={open}
                                    onDelete={remove}
                                    onAction={action}
                                />
                            ))
                        ) : (
                            <div className="empty-state compact">
                                <div>
                                    <h1>No execution workflows</h1>
                                    <p>Connect tasks across nodes, define their order, and run the flow.</p>
                                </div>
                            </div>
                        )}
                    </div>
                    <section className="workflow-ungrouped">
                        <header>
                            <h2>Ungrouped workspaces</h2>
                            <p>Visible workspaces not assigned to any workflow group.</p>
                        </header>
                        <div className="workflow-targets">
                            {ungrouped.length ? (
                                ungrouped.map((target) => (
                                    <button className="workflow-target" key={targetKey(target)} type="button">
                                        <strong>{target.workspace_display_name}</strong>
                                        <span>
                                            {target.node_name} · {target.session} · {target.tasks.length} tasks
                                        </span>
                                    </button>
                                ))
                            ) : (
                                <div className="muted">Every visible workspace is assigned to a workflow group.</div>
                            )}
                        </div>
                    </section>
                    <Orchestrator
                        members={members}
                        graph={graph}
                        setGraph={setGraph}
                        active={Boolean(editing)}
                        onRun={() => void run()}
                        onSave={async () => {
                            if (!editing) return;
                            const group = groups.find((item) => item.id === editing);
                            if (!group) return;
                            const result = await api.request(
                                `/api/workflow-groups/${encodeURIComponent(editing)}`,
                                passthrough,
                                {
                                    method: "PUT",
                                    headers: { "content-type": "application/json" },
                                    body: JSON.stringify({
                                        name: group.name,
                                        members,
                                        graph: normalizedGraph(graph, members.length),
                                    }),
                                },
                            );
                            setMessage(result.ok ? "Layout saved." : result.message || "Unable to save layout");
                            await load();
                        }}
                        onHistory={() => void history()}
                        summary={summary}
                        revisions={showRevisions ? revisions : []}
                        showRevisionsBox={showRevisions}
                        onRestore={restore}
                    />
                    <Dependencies
                        dependencies={dependencies}
                        targets={dependencyTargets}
                        onRefresh={() => void loadDependencies()}
                        onAdd={async (body: Record<string, string>) => {
                            const result = await api.request("/api/dependencies", passthrough, {
                                method: "POST",
                                headers: { "content-type": "application/json" },
                                body: JSON.stringify(body),
                            });
                            if (!result.ok) setMessage(result.message || "Unable to add dependency");
                            await loadDependencies();
                        }}
                        onDelete={async (id: string) => {
                            const result = await api.request(
                                `/api/dependencies/${encodeURIComponent(id)}`,
                                passthrough,
                                { method: "DELETE" },
                            );
                            if (!result.ok) setMessage(result.message || "Unable to delete dependency");
                            await loadDependencies();
                        }}
                    />
                </div>
                <WorkflowEditor
                    editing={editing}
                    name={name}
                    setName={setName}
                    members={members}
                    targets={targets}
                    onMember={updateMember}
                    onAdd={() => {
                        const target = targets.find((item) => item.tasks.length) ?? targets[0];
                        if (target)
                            setMembers((current) => [
                                ...current,
                                { node_id: target.node_id, session: target.session, task: target.tasks[0] ?? "" },
                            ]);
                    }}
                    onRemove={(index: number) => setMembers((current) => current.filter((_, i) => i !== index))}
                    onMove={(from: number, to: number) => setMembers((current) => reorder(current, from, to))}
                    onCancel={close}
                    onSave={() => void save()}
                    busy={busy}
                    message={message}
                />
            </div>
        </section>
    );
}
