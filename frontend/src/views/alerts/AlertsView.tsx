import { useEffect, useState } from "react";
import type { Notification, NotificationRule, NotificationsView } from "../../domain/models";
import { formatTimestamp } from "../../lib/helpers";
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
    const [collapsed, setCollapsed] = useState(
        () => localStorage.getItem("taskdeck-alert-rules-collapsed") !== "false",
    );
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
            )
            .then(() => load());
    const toggle = () => {
        const next = !collapsed;
        setCollapsed(next);
        localStorage.setItem("taskdeck-alert-rules-collapsed", String(next));
    };
    return (
        <section className="view alerts-view active" data-react-owned="true" id="alerts-view">
            <div className={`alerts-layout${collapsed ? " editor-collapsed" : ""}`}>
                <div className="alerts-main">
                    <header className="section-heading">
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
                            <button
                                className="button"
                                id="open-alert-rules"
                                type="button"
                                onClick={() => setCollapsed(false)}>
                                Rules
                            </button>
                        </div>
                    </header>
                    <div className="notifications-list" id="notifications-list">
                        {notifications.length ? (
                            notifications.map((n) => (
                                <article className={`notification-item ${n.read ? "read" : "unread"}`} key={n.id}>
                                    <header>
                                        <span className={`status-pill ${n.severity === "critical" ? "error" : ""}`}>
                                            {n.event_type}
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
                    </div>
                </div>
                <aside className="alerts-editor">
                    <header>
                        <div>
                            <h2 id="rules-title">Alert rules</h2>
                            <p>Notify on task transitions; optionally POST the event to a webhook.</p>
                        </div>
                        <button
                            className="icon-button alerts-editor-toggle"
                            id="toggle-alert-rules"
                            type="button"
                            aria-expanded={!collapsed}
                            aria-label={collapsed ? "Expand alert rules" : "Collapse alert rules"}
                            onClick={toggle}>
                            ×
                        </button>
                    </header>
                    <div className="notification-rules" id="notification-rules">
                        {rules.length ? (
                            rules.map((r) => (
                                <article className={`rule-card ${r.enabled ? "" : "disabled"}`} key={r.id}>
                                    <header>
                                        <strong>{r.name}</strong>
                                        <span className={`status-pill ${r.enabled ? "" : "muted-pill"}`}>
                                            {r.enabled ? "enabled" : "disabled"}
                                        </span>
                                        <div className="workflow-card-actions">
                                            <button
                                                className="button compact"
                                                type="button"
                                                onClick={() => setEditing(r)}>
                                                Edit
                                            </button>
                                            <button
                                                className="button compact danger"
                                                type="button"
                                                onClick={() =>
                                                    void api
                                                        .request(
                                                            `/api/notification-rules/${encodeURIComponent(r.id)}`,
                                                            objectDecoder((x) => x),
                                                            { method: "DELETE" },
                                                        )
                                                        .then(() => load())
                                                }>
                                                Delete
                                            </button>
                                        </div>
                                    </header>
                                    <p className="muted">
                                        {r.event_types.join(" · ")}
                                        {r.scope_session ? ` · ${r.scope_session}` : ""}
                                        {r.webhook_url ? " · webhook" : ""}
                                    </p>
                                </article>
                            ))
                        ) : (
                            <div className="muted">No alert rules yet.</div>
                        )}
                    </div>
                    <RuleForm
                        rule={editing}
                        message={message}
                        onCancel={() => {
                            setEditing(null);
                            setMessage("");
                        }}
                        onSaved={() => {
                            setEditing(null);
                            setMessage("Rule saved.");
                            load();
                        }}
                    />
                </aside>
            </div>
        </section>
    );
}
