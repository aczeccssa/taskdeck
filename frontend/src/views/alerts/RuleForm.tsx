import { useEffect, useState } from "react";
import type { NotificationRule } from "../../domain/models";
import { api } from "../shared/api";
import { objectDecoder } from "../shared/decode";

export function RuleForm({
    rule: current,
    message,
    onCancel,
    onSaved,
}: {
    rule: NotificationRule | null;
    message: string;
    onCancel: () => void;
    onSaved: () => void;
}): React.JSX.Element {
    const [name, setName] = useState("");
    const [started, setStarted] = useState(true);
    const [exited, setExited] = useState(false);
    const [failed, setFailed] = useState(true);
    const [stopped, setStopped] = useState(false);
    const [session, setSession] = useState("");
    const [task, setTask] = useState("");
    const [webhook, setWebhook] = useState("");
    const [enabled, setEnabled] = useState(true);
    useEffect(() => {
        setName(current?.name || "");
        setStarted(current ? current.event_types.includes("task_started") : true);
        setExited(current?.event_types.includes("task_exited") || false);
        setFailed(current ? current.event_types.includes("task_failed") : true);
        setStopped(current?.event_types.includes("task_stopped") || false);
        setSession(current?.scope_session || "");
        setTask(current?.scope_task || "");
        setWebhook(current?.webhook_url || "");
        setEnabled(current?.enabled ?? true);
    }, [current]);
    const submit = (e: React.FormEvent<HTMLFormElement>) => {
        e.preventDefault();
        const event_types = [
            started && "task_started",
            exited && "task_exited",
            failed && "task_failed",
            stopped && "task_stopped",
        ].filter((v): v is string => Boolean(v));
        void api
            .request(
                current ? `/api/notification-rules/${encodeURIComponent(current.id)}` : "/api/notification-rules",
                objectDecoder((x) => x),
                {
                    method: current ? "PUT" : "POST",
                    headers: { "content-type": "application/json" },
                    body: JSON.stringify({
                        name: name.trim(),
                        event_types,
                        scope_session: session.trim() || null,
                        scope_task: task.trim() || null,
                        webhook_url: webhook.trim() || null,
                        enabled,
                    }),
                },
            )
            .then((r) => {
                if (r.ok) onSaved();
            });
    };
    return (
        <form className="settings-form" id="rule-form" onSubmit={submit}>
            <label className="field">
                <span>Rule name</span>
                <input
                    id="rule-name"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder="API failures"
                    autoComplete="off"
                />
            </label>
            <fieldset className="rule-events">
                <legend>Events</legend>
                {[
                    ["Task started", started, setStarted],
                    ["Task exited", exited, setExited],
                    ["Task failed", failed, setFailed],
                    ["Task stopped", stopped, setStopped],
                ].map(([label, value, setter]) => (
                    <label className="check-field" key={String(label)}>
                        <input
                            type="checkbox"
                            checked={Boolean(value)}
                            onChange={(e) => (setter as (v: boolean) => void)(e.target.checked)}
                        />
                        <span>{String(label)}</span>
                    </label>
                ))}
            </fieldset>
            <div className="field-grid compact-grid">
                <label className="field">
                    <span>Workspace (optional)</span>
                    <input
                        id="rule-scope-session"
                        value={session}
                        onChange={(e) => setSession(e.target.value)}
                        placeholder="api"
                        autoComplete="off"
                    />
                </label>
                <label className="field">
                    <span>Task (optional)</span>
                    <input
                        id="rule-scope-task"
                        value={task}
                        onChange={(e) => setTask(e.target.value)}
                        autoComplete="off"
                    />
                </label>
            </div>
            <label className="field">
                <span>Webhook URL (optional)</span>
                <input
                    id="rule-webhook"
                    type="url"
                    value={webhook}
                    onChange={(e) => setWebhook(e.target.value)}
                    placeholder="https://example.com/hook"
                    autoComplete="off"
                />
            </label>
            <label className="check-field">
                <input
                    id="rule-enabled"
                    type="checkbox"
                    checked={enabled}
                    onChange={(e) => setEnabled(e.target.checked)}
                />
                <span>Enabled</span>
            </label>
            <div className="settings-message" id="rule-message" role="status">
                {message}
            </div>
            <footer className="settings-footer">
                {current && (
                    <button className="button" id="cancel-rule" type="button" onClick={onCancel}>
                        Cancel
                    </button>
                )}
                <button className="button primary" id="save-rule" type="submit">
                    {current ? "Save" : "Add rule"}
                </button>
            </footer>
        </form>
    );
}
