import type { TaskAction, WorkflowGroup } from "../../domain/models";

export function WorkflowCard({
    group,
    onOpen,
    onDelete,
    onAction,
}: {
    group: WorkflowGroup;
    onOpen: (group: WorkflowGroup) => void;
    onDelete: (id: string, name: string) => void;
    onAction: (id: string, action: TaskAction, count: number) => void;
}): React.JSX.Element {
    const edges = group.graph?.edges?.length ?? 0;
    return (
        <article className="workflow-card">
            <header>
                <div>
                    <h2>{group.name}</h2>
                    <p>
                        {group.members.length} execution step{group.members.length === 1 ? "" : "s"} · {edges}{" "}
                        connection{edges === 1 ? "" : "s"}
                    </p>
                </div>
                <div className="workflow-card-actions">
                    <button className="button compact" type="button" onClick={() => onOpen(group)}>
                        Edit graph
                    </button>
                    <button
                        className="button compact danger"
                        type="button"
                        onClick={() => onDelete(group.id, group.name)}>
                        Delete
                    </button>
                </div>
            </header>
            <ol className="workflow-steps">
                {group.members.map((member, index) => (
                    <li className="workflow-step" key={`${member.node_id}-${member.session}-${member.task}-${index}`}>
                        <span className="workflow-step-number">{index + 1}</span>
                        <span className="workflow-member">
                            <strong>{member.task || "Untitled task"}</strong>
                            <span>
                                {member.node_id} · {member.session}
                            </span>
                        </span>
                        {index < group.members.length - 1 && (
                            <span className="workflow-step-arrow" aria-hidden="true">
                                ↓
                            </span>
                        )}
                    </li>
                ))}
            </ol>
            <footer className="workflow-actions">
                {(["start", "stop", "restart", "pause", "resume"] as TaskAction[]).map((item) => (
                    <button
                        className="button compact"
                        type="button"
                        key={item}
                        onClick={() => onAction(group.id, item, group.members.length)}>
                        {item}
                    </button>
                ))}
            </footer>
        </article>
    );
}
