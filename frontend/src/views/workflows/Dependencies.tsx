import { useState } from "react";
import type { TaskDependencyView, WorkflowTargetView } from "../../domain/models";

type DependencyForm = {
    node_id: string;
    session: string;
    task: string;
    depends_node_id: string;
    depends_session: string;
    depends_task: string;
};
export function Dependencies({
    dependencies: items,
    targets,
    onRefresh,
    onAdd,
    onDelete,
}: {
    dependencies: TaskDependencyView[];
    targets: WorkflowTargetView[];
    onRefresh: () => void;
    onAdd: (body: Record<string, string>) => void | Promise<void>;
    onDelete: (id: string) => void | Promise<void>;
}): React.JSX.Element {
    const [form, setForm] = useState<DependencyForm>({
        node_id: "",
        session: "",
        task: "",
        depends_node_id: "",
        depends_session: "",
        depends_task: "",
    });
    const update = (key: keyof DependencyForm, value: string): void =>
        setForm((current) => {
            const next = { ...current, [key]: value };
            const nodeKey = key === "node_id" || key === "depends_node_id" ? key : null;
            if (nodeKey) {
                const sessionKey = nodeKey === "node_id" ? "session" : "depends_session";
                const taskKey = nodeKey === "node_id" ? "task" : "depends_task";
                next[sessionKey] = "";
                next[taskKey] = "";
            }
            if (key === "session" || key === "depends_session") {
                const taskKey = key === "session" ? "task" : "depends_task";
                next[taskKey] = "";
            }
            return next;
        });
    const nodeOptions = [...new Set(targets.map((target) => target.node_id))];
    const sessionsFor = (nodeKey: "node_id" | "depends_node_id"): Array<{ value: string; label: string }> => {
        const nodeTargets = targets.filter((target) => target.node_id === form[nodeKey]);
        const seen = new Set<string>();
        const out: Array<{ value: string; label: string }> = [];
        for (const target of nodeTargets) {
            if (seen.has(target.session)) continue;
            seen.add(target.session);
            out.push({ value: target.session, label: target.workspace_display_name || target.session });
        }
        return out;
    };
    const tasksFor = (nodeKey: "node_id" | "depends_node_id", sessionKey: "session" | "depends_session"): string[] =>
        targets.find((target) => target.node_id === form[nodeKey] && target.session === form[sessionKey])?.tasks ?? [];
    const select = (
        nodeKey: "node_id" | "depends_node_id",
        sessionKey: "session" | "depends_session",
        taskKey: "task" | "depends_task",
    ) => (
        <>
            <select
                value={form[nodeKey]}
                onChange={(event) => update(nodeKey, event.target.value)}
                aria-label={nodeKey === "node_id" ? "Task node" : "Depends on node"}>
                <option value="">–</option>
                {nodeOptions.map((node) => (
                    <option value={node} key={node}>
                        {targets.find((target) => target.node_id === node)?.node_name || node}
                    </option>
                ))}
            </select>
            <select
                value={form[sessionKey]}
                onChange={(event) => update(sessionKey, event.target.value)}
                aria-label={nodeKey === "node_id" ? "Task workspace" : "Depends on workspace"}>
                <option value="">–</option>
                {sessionsFor(nodeKey).map((item) => (
                    <option value={item.value} key={item.value}>
                        {item.label}
                    </option>
                ))}
            </select>
            <select
                value={form[taskKey]}
                onChange={(event) => update(taskKey, event.target.value)}
                aria-label={nodeKey === "node_id" ? "Task" : "Depends on task"}>
                <option value="">–</option>
                {tasksFor(nodeKey, sessionKey).map((task) => (
                    <option value={task} key={task}>
                        {task}
                    </option>
                ))}
            </select>
        </>
    );
    return (
        <section className="workflow-card">
            <header>
                <div>
                    <h2>Task dependencies</h2>
                    <p>Cross-workspace start gates: a task only starts while every dependency is running.</p>
                </div>
                <button className="button" type="button" onClick={onRefresh}>
                    Refresh
                </button>
            </header>
            <div className="dependencies-list">
                {items.length ? (
                    items.map((item: TaskDependencyView) => (
                        <article className="dependency-row" key={item.id}>
                            <span className={`status-pill ${item.task_status ? "" : "error"}`}>
                                {item.task_status ? "ready" : "missing"}
                            </span>
                            <strong>
                                {item.task_node_id}/{item.task_session}/{item.task}
                            </strong>
                            <span className="dependency-arrow">← depends on →</span>
                            <strong>
                                {item.depends_node_id}/{item.depends_session}/{item.depends_task}
                            </strong>
                            <button className="button compact danger" type="button" onClick={() => onDelete(item.id)}>
                                Delete
                            </button>
                        </article>
                    ))
                ) : (
                    <div className="muted">No task dependencies declared.</div>
                )}
            </div>
            <form
                className="settings-form"
                onSubmit={(event) => {
                    event.preventDefault();
                    void onAdd({ ...form });
                }}>
                <div className="field-grid">
                    <label className="field">
                        <span>Task</span>
                        {select("node_id", "session", "task")}
                    </label>
                    <label className="field">
                        <span>Depends on</span>
                        {select("depends_node_id", "depends_session", "depends_task")}
                    </label>
                </div>
                <footer className="settings-footer">
                    <span className="muted">Required state: running.</span>
                    <button className="button primary" type="submit">
                        Add dependency
                    </button>
                </footer>
            </form>
        </section>
    );
}
