import { useEffect, useState } from "react";
import type { Notification, NotificationRule, NotificationsView } from "../../domain/models";
import { formatTimestamp } from "../../lib/helpers";
import { moveTabFocus } from "../../lib/tabs";
import { api } from "../shared/api";
import { objectDecoder, text, num, bool } from "../shared/decode";
import { RuleForm } from "./RuleForm";

const notification = objectDecoder<Notification>((r) => ({
    id: num(r.id),
    node_id: text(r.node_id),
    rule_id: text(r.rule_id) || null,
    rule_name: text(r.rule_name) || null,
    event_type: text(r.event_type),
    severity: text(r.severity),
    session: text(r.session) || null,
    task: text(r.task) || null,
    title: text(r.title),
    message: text(r.message),
    read: bool(r.read),
    created_at_ms: num(r.created_at_ms),
}));
const rule = objectDecoder<NotificationRule>((r) => ({
    id: text(r.id),
    name: text(r.name),
    event_types: Array.isArray(r.event_types)
        ? r.event_types.filter((v): v is string => typeof v === "string")
        : [],
    scope_session: text(r.scope_session) || null,
    scope_task: text(r.scope_task) || null,
    webhook_url: text(r.webhook_url) || null,
    enabled: bool(r.enabled),
    created_at_ms: num(r.created_at_ms),
    updated_at_ms: num(r.updated_at_ms),
}));

export function AlertsView(): React.JSX.Element {
    const [notifications, setNotifications] = useState<Notification[]>([]);
    const [rules, setRules] = useState<NotificationRule[]>([]);
    const [unread, setUnread] = useState(0);
    const [tab, setTab] = useState<"inbox" | "rules">("inbox");
    const [editorOpen, setEditorOpen] = useState(false);
    const [editing, setEditing] = useState<NotificationRule | null>(null);
    const [message, setMessage] = useState("");
    const load = () =>
        void Promise.all([
            api.request(
                "/api/notifications?limit=200",
                objectDecoder<NotificationsView>((r) => ({
                    notifications: Array.isArray(r.notifications) ? r.notifications.map(notification) : [],
                    unread_count: num(r.unread_count),
                })),
            ),
            api.request("/api/notification-rules", (v) => (Array.isArray(v) ? v.map(rule) : [])),
        ]).then(([n, r]) => {
            if (n.ok && n.data) {
                setNotifications(n.data.notifications);
                setUnread(n.data.unread_count);
            }
            if (r.ok && r.data) setRules(r.data);
        });
    useEffect(() => {
        load();
        const id = window.setInterval(load, 5000);
        return () => window.clearInterval(id);
    }, []);
    useEffect(() => {
        const changed = () => load();
        window.addEventListener("taskdeck:notifications-changed", changed);
        return () => window.removeEventListener("taskdeck:notifications-changed", changed);
    }, []);
    const markAll = () =>
        void api
            .request(
                "/api/notifications/read",
                objectDecoder((r) => r),
                {
                    method: "POST",
                    headers: { "content-type": "application/json" },
                    body: JSON.stringify({ all: true }),
                },
            )
            .then((result) => {
                if (!result.ok) return;
                window.dispatchEvent(new CustomEvent("taskdeck:notifications-changed"));
                return load();
            });
    const eventName = (value: string) => ({task_started: "Task started", task_exited: "Task exited", task_failed: "Task failed", task_stopped: "Task stopped"})[value] ?? value.replaceAll("_", " ");
    return (
        <section className="view alerts-view active" data-react-owned="true" id="alerts-view">
            <div className={`alerts-layout${editorOpen ? "" : " editor-collapsed"}`}>
                <div className="alerts-main">
                    <header className="section-heading page-heading">
                        <div><h1>Alerts</h1><p className="muted">Review important task events and decide when Taskdeck should notify you.</p></div>
                        <span id="alerts-summary" className="sr-only">
                            {unread} unread notifications
                        </span>
                        <div className="settings-actions">
                            <button className="button" id="refresh-alerts" type="button" onClick={load}>
                                Refresh
                            </button>
                            <button className="button" id="mark-all-read" type="button" onClick={markAll}>
                                Mark all read
                            </button>
                        </div>
                    </header>
                    <div className="secondary-tabs" role="tablist" aria-label="Alert views" onKeyDown={moveTabFocus}><button role="tab" aria-selected={tab === "inbox"} onClick={() => setTab("inbox")}>Inbox <span>{unread}</span></button><button role="tab" aria-selected={tab === "rules"} onClick={() => setTab("rules")}>Rules <span>{rules.length}</span></button></div>
                    {tab === "inbox" && <div className="notifications-list" id="notifications-list">
                        {notifications.length ? (
                            notifications.map((n) => (
                                <article className={`notification-item ${n.read ? "read" : "unread"}`} key={n.id}>
                                    <header>
                                        <span className={`status-pill ${n.severity === "critical" ? "error" : ""}`}>
                                            {eventName(n.event_type)}
                                        </span>
                                        <strong>{n.title}</strong>
                                        <span className="muted">{formatTimestamp(n.created_at_ms)}</span>
                                        {!n.read && (
                                            <button
                                                className="button compact"
                                                type="button"
                                                onClick={() =>
                                                    void api
                                                        .request(
                                                            "/api/notifications/read",
                                                            objectDecoder((r) => r),
                                                            {
                                                                method: "POST",
                                                                headers: { "content-type": "application/json" },
                                                                body: JSON.stringify({ id: n.id }),
                                                            },
                                                        )
                                                        .then(() => load())
                                                }>
                                                Mark read
                                            </button>
                                        )}
                                    </header>
                                    <p>{n.message}</p>
                                    {n.session && (
                                        <p className="muted">
                                            {n.session}
                                            {n.task ? ` · ${n.task}` : ""}
                                        </p>
                                    )}
                                </article>
                            ))
                        ) : (
                            <div className="empty-state compact">
                                <div>
                                    <h1>No notifications yet.</h1>
                                </div>
                            </div>
                        )}
                    </div>}
                    {tab === "rules" && <div className="notification-rules rules-main" id="notification-rules"><div className="list-toolbar"><p className="muted">Rules turn task state changes into an inbox item and, optionally, a webhook delivery.</p><button className="button primary" onClick={() => { setEditing(null); setEditorOpen(true); }}>New rule</button></div>{rules.length ? rules.map((r) => <article className={`rule-card ${r.enabled ? "" : "disabled"}`} key={r.id}><header><div><strong>{r.name}</strong><p>{r.event_types.map(eventName).join(" · ")}</p></div><span className={`status-pill ${r.enabled ? "success" : "muted-pill"}`}>{r.enabled ? "Enabled" : "Paused"}</span><div className="workflow-card-actions"><button className="button compact" onClick={() => { setEditing(r); setEditorOpen(true); }}>Edit</button><button className="button compact danger" onClick={() => void api.request(`/api/notification-rules/${encodeURIComponent(r.id)}`, objectDecoder((x) => x), {method: "DELETE"}).then(() => load())}>Delete</button></div></header><p className="muted">{r.scope_session ? `Workspace ${r.scope_session}${r.scope_task ? ` / ${r.scope_task}` : ""}` : "All workspaces"}{r.webhook_url ? " · Inbox and webhook" : " · Inbox only"}</p></article>) : <div className="empty-state compact"><div><h3>No alert rules</h3><p>Add a rule for failures or important lifecycle changes.</p></div></div>}</div>}
                </div>
                {editorOpen && <aside className="alerts-editor">
                    <header>
                        <div>
                            <h2 id="rules-title">Alert rules</h2>
                            <p>Notify on task transitions; optionally POST the event to a webhook.</p>
                        </div>
                        <button
                            className="icon-button alerts-editor-toggle"
                            id="toggle-alert-rules"
                            type="button"
                            aria-expanded={editorOpen}
                            aria-label="Close alert rule editor"
                            onClick={() => setEditorOpen(false)}>
                            ×
                        </button>
                    </header>
                    <RuleForm
                        rule={editing}
                        message={message}
                        onCancel={() => {
                            setEditing(null);
                            setEditorOpen(false);
                            setMessage("");
                        }}
                        onSaved={() => {
                            setEditing(null);
                            setEditorOpen(false);
                            setMessage("Rule saved.");
                            load();
                        }}
                        onError={setMessage}
                    />
                </aside>}
            </div>
        </section>
    );
}
