import type { Summary } from "./types";

export function WorkflowResult({ summary }: { summary: Summary }): React.JSX.Element {
    return (
        <div className="workflow-results">
            <h3>
                {summary.group_name ?? "Workflow"} · {summary.action ?? "run"}
            </h3>
            <p>
                {summary.success_count ?? 0} ok · {summary.failed_count ?? 0} failed · {summary.skipped_count ?? 0}{" "}
                skipped
            </p>
            {summary.results?.map((item, i) => (
                <div className={`workflow-result ${item.status ?? ""}`} key={i}>
                    <strong>
                        #{i + 1} {item.workspace_display_name} / {item.task}
                    </strong>
                    <span>
                        {item.status} · {item.message}
                    </span>
                </div>
            ))}
        </div>
    );
}
