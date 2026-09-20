import {useCallback, useEffect, useRef, useState} from "react";
import {Chart3, ClipboardText3, Code4, Diagram, DocumentText, Kanban, Notification2, Settings4, Task} from "reicon-react";
import {UI_ICON_STROKE_WIDTH} from "../components/ReiconNavigation";
import {Outlet, useLocation, useNavigate} from "react-router-dom";
import {routes, type View} from "../app/routes";
import {ThemeIcon} from "./ThemeIcon";
import {LegacyApiAdapter} from "../compat/api";
import {formatTimestamp} from "../lib/helpers";
import {applyI18n, storedLanguage, viewTitle} from "../lib/i18n";
import {showToast} from "../lib/toast";
import type {Notification} from "../domain/models";
import type {Language} from "../domain/state";
import {useSelection} from "./SelectionContext";

const api = new LegacyApiAdapter();
const POPOVER_LIMIT = 5;

const navigation: ReadonlyArray<{label: string; items: ReadonlyArray<{view: View; label: string; icon: typeof Task}>}> = [
    {label: "Workspace", items: [
        {view: "tasks", label: "Tasks", icon: Task},
        {view: "workflows", label: "Workflows", icon: Diagram},
        {view: "boards", label: "Boards", icon: Kanban},
    ]},
    {label: "Observe", items: [
        {view: "dashboard", label: "Dashboard", icon: Chart3},
        {view: "alerts", label: "Alerts", icon: Notification2},
        {view: "calls", label: "MCP Calls", icon: Code4},
        {view: "audit", label: "Audit Log", icon: ClipboardText3},
    ]},
    {label: "Manage", items: [{view: "settings", label: "Settings", icon: Settings4}]},
    {label: "Help", items: [{view: "docs", label: "MCP Guide", icon: DocumentText}]},
];

type ThemeMode = "system" | "light" | "dark";
const isThemeMode = (value: unknown): value is ThemeMode => value === "system" || value === "light" || value === "dark";
const severityClass = (severity: string): string => severity === "critical" ? "error" : severity === "warning" ? "warning" : "";

function decodeNotification(value: unknown): Notification {
    const record = typeof value === "object" && value !== null ? value as Record<string, unknown> : {};
    return {
        id: typeof record.id === "number" ? record.id : 0,
        node_id: typeof record.node_id === "string" ? record.node_id : "",
        rule_id: typeof record.rule_id === "string" ? record.rule_id : null,
        rule_name: typeof record.rule_name === "string" ? record.rule_name : null,
        event_type: typeof record.event_type === "string" ? record.event_type : "",
        severity: typeof record.severity === "string" ? record.severity : "info",
        session: typeof record.session === "string" ? record.session : null,
        task: typeof record.task === "string" ? record.task : null,
        title: typeof record.title === "string" ? record.title : "",
        message: typeof record.message === "string" ? record.message : "",
        read: record.read === true,
        created_at_ms: typeof record.created_at_ms === "number" ? record.created_at_ms : 0,
    };
}

export function AppShell(): React.JSX.Element {
    const location = useLocation();
    const navigate = useNavigate();
    const currentView = routes.find((route) => route.path === location.pathname)?.view ?? "tasks";
    const [collapsed, setCollapsed] = useState(() => localStorage.getItem("taskdeck-sidebar-collapsed") === "true");
    const [language, setLanguage] = useState<Language>(storedLanguage);
    const [theme, setTheme] = useState<ThemeMode>(() => {
        const stored = localStorage.getItem("taskdeck-theme");
        return isThemeMode(stored) ? stored : "system";
    });
    const [unread, setUnread] = useState<number | null>(null);
    const [recent, setRecent] = useState<Notification[]>([]);
    const [popoverOpen, setPopoverOpen] = useState(false);
    const [update, setUpdate] = useState<{available: boolean; latest_version?: string; release_url?: string} | null>(null);
    const selection = useSelection();

    const toggleSidebar = (): void => {
        const next = !collapsed;
        setCollapsed(next);
        localStorage.setItem("taskdeck-sidebar-collapsed", String(next));
        window.dispatchEvent(new CustomEvent("taskdeck:sidebar-changed", {detail: next}));
    };

    const cycleTheme = (): void => {
        const next: ThemeMode = theme === "system" ? "light" : theme === "light" ? "dark" : "system";
        setTheme(next);
        if (next === "system") delete document.documentElement.dataset.theme;
        else document.documentElement.dataset.theme = next;
        localStorage.setItem("taskdeck-theme", next);
        window.dispatchEvent(new CustomEvent("taskdeck:theme-changed", {detail: next}));
        showToast(`Theme: ${next}`);
    };

    const toggleLanguage = (): void => {
        const next: Language = language === "en" ? "zh" : "en";
        setLanguage(next);
        localStorage.setItem("taskdeck-lang", next);
        applyI18n(next);
        showToast(next === "zh" ? "语言：中文" : "Language: English");
    };

    const previousUnread = useRef<number | null>(null);
    const loadRecent = useCallback(async (): Promise<void> => {
        const result = await api.request("/api/notifications?limit=200", (value) => {
            const record = typeof value === "object" && value !== null ? value as Record<string, unknown> : {};
            return Array.isArray(record.notifications) ? record.notifications.map(decodeNotification) : [];
        });
        if (!result.ok || result.data === undefined) return;
        const notifications = result.data;
        setRecent([...notifications].sort((a, b) => b.created_at_ms - a.created_at_ms).slice(0, POPOVER_LIMIT));
        const count = result.data.filter((item) => !item.read).length;
        previousUnread.current = count;
        setUnread(count);
    }, []);

    const loadUnread = useCallback(async (): Promise<void> => {
        const result = await api.request("/api/notifications?limit=1", (value) => {
            const record = typeof value === "object" && value !== null ? value as Record<string, unknown> : {};
            return typeof record.unread_count === "number" ? record.unread_count : 0;
        });
        if (!result.ok || result.data === undefined) return;
        const count = result.data;
        const changed = previousUnread.current !== null && count > previousUnread.current;
        previousUnread.current = count;
        setUnread(count);
        if (changed) {
            window.dispatchEvent(new CustomEvent("taskdeck:notifications-changed"));
            void loadRecent();
            showToast("New task alert received");
            if (typeof Notification !== "undefined" && Notification.permission === "granted") {
                new Notification("Taskdeck alert", {body: "A task lifecycle alert needs your attention."});
            }
        }
    }, [loadRecent]);

    useEffect(() => { window.__taskdeckReactShell = true; return () => { delete window.__taskdeckReactShell; }; }, []);
    useEffect(() => {
        void loadUnread();
        const timer = window.setInterval(() => void loadUnread(), 5000);
        return () => window.clearInterval(timer);
    }, [loadUnread]);
    useEffect(() => {
        void fetch("/api/update", {credentials: "same-origin"}).then((response) => response.json() as Promise<{data?: {available?: boolean; latest_version?: string; release_url?: string}}>).then((result) => {
            if (result.data?.available) setUpdate({available: true, latest_version: result.data.latest_version, release_url: result.data.release_url});
        }).catch(() => undefined);
    }, []);

    const togglePopover = (): void => {
        const next = !popoverOpen;
        setPopoverOpen(next);
        if (next) {
            void loadRecent();
            if (typeof Notification !== "undefined" && Notification.permission === "default" && window.isSecureContext)
                void Notification.requestPermission();
        }
    };

    const markPopoverRead = async (id: number): Promise<void> => {
        await api.request("/api/notifications/read", () => undefined, {method: "POST", headers: {"content-type": "application/json"}, body: JSON.stringify({id})});
        window.dispatchEvent(new CustomEvent("taskdeck:notifications-changed"));
        await loadRecent();
    };

    const openAllNotifications = (): void => {
        setPopoverOpen(false);
        navigate("/alerts");
    };

    const navLabel = (view: View, fallback: string): string => language === "zh" ? ZH_NAV[view] ?? fallback : fallback;

    return <div className={`shell${collapsed ? " sidebar-collapsed" : ""}`} id="app-shell">
        <aside className="sidebar" id="sidebar">
            <div className="brand-row">
                <button className="brand" id="brand-toggle" type="button" aria-label={collapsed ? "Expand navigation" : "Collapse navigation"} title={collapsed ? "Expand navigation" : "Collapse navigation"} aria-expanded={!collapsed} onClick={toggleSidebar}>
                    <span className="brand-mark" aria-hidden="true"><img className="brand-favicon" src="/favicon.svg" alt="" aria-hidden="true"/></span>
                    <span className="brand-copy"><strong>Taskdeck</strong><small>Control plane</small></span>
                </button>
            </div>
            <nav className="nav" aria-label="Primary navigation">
                {navigation.map((group) => <div className="nav-group" key={group.label}>
                    <div className="nav-label">{group.label}</div>
                    {group.items.map(({view, label, icon: Icon}) => <button className={`nav-button${currentView === view ? " active" : ""}`} key={view} type="button" data-view={view} aria-label={view === "alerts" && unread ? `${label}, ${unread} unread` : label} title={label} onClick={() => window.dispatchEvent(new CustomEvent("taskdeck:route-request", {detail: view}))}>
                        <i className="nav-reicon" aria-hidden="true"><Icon className="ui-icon ui-icon--navigation" size={18} strokeWidth={UI_ICON_STROKE_WIDTH} weight="Outline" aria-hidden/></i>
                        <span data-i18n={`nav.${view}`}>{navLabel(view, label)}</span>
                        {view === "alerts" && unread ? <i className="nav-alert-indicator" aria-hidden="true"/> : null}
                    </button>)}
                </div>)}
            </nav>
            <div className="nav-spacer"/>
            <div className="connection" id="connection-state"><i/><span>Daemon connected</span></div>
        </aside>
        <main className="main react-owned-route">
            <header className="topbar">
                <div className="topbar-title"><strong id="page-title">{viewTitle(currentView, language)}</strong>{currentView === "tasks" && <span id="meta">{selection.meta}</span>}</div>
                {currentView === "tasks" && <>
                    <div className="context-control">
                        <select className="node-select" id="nodes" aria-label="Taskdeck node" value={selection.selectedNode} disabled={selection.loading && !selection.nodes.length} onChange={(event) => selection.selectNode(event.target.value)}>
                            {selection.nodes.length ? selection.nodes.map((node) => <option key={node.id} value={node.id}>{node.is_self ? `This device · ${node.name}` : `${node.name}${node.online ? "" : " · offline"}`}</option>) : <option value="">No nodes</option>}
                        </select>
                    </div>
                    <div className="context-control">
                        <select className="session-select" id="sessions" aria-label="Registered session" value={selection.selectedSession} disabled={!selection.sessions.length} onChange={(event) => selection.selectSession(event.target.value)}>
                            {selection.sessions.length ? selection.sessions.map((session) => {
                                const workspace = selection.workspaces.find((item) => item.session === session);
                                return <option key={session} value={session}>{workspace?.alias ? `${workspace.alias} · ${session}` : session}</option>;
                            }) : <option value="">No sessions</option>}
                        </select>
                    </div>
                </>}
                <div className="notification-menu-wrap">
                    <button className="icon-button alerts-bell" id="alerts-bell" type="button" aria-expanded={popoverOpen} aria-haspopup="true" aria-controls="notification-popover" aria-label="Notifications" title="Notifications" hidden={unread === null} onClick={togglePopover}>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 9a6 6 0 1 1 12 0c0 5 2 6 2 6H4s2-1 2-6M10 20a2 2 0 0 0 4 0"/></svg>
                        <span className="alerts-unread" id="alerts-unread" hidden={unread === null || unread === 0}>{unread ?? 0}</span>
                    </button>
                    <div className="notification-popover" id="notification-popover" role="dialog" aria-label="Recent notifications" hidden={!popoverOpen}>
                        <div className="notification-popover-header"><strong>Notifications</strong><span id="notification-popover-count">{unread ? `${unread} unread` : ""}</span></div>
                        <div className="notification-popover-list" id="notification-popover-list">
                            {recent.length ? recent.map((item) => <article className={`notification-popover-item ${item.read ? "read" : "unread"}`} key={item.id}>
                                <div><span className={`status-dot ${severityClass(item.severity)}`}/><strong>{item.title}</strong></div>
                                <p>{item.message}</p>
                                <small>{formatTimestamp(item.created_at_ms, language)}{item.session ? ` · ${item.session}${item.task ? ` · ${item.task}` : ""}` : ""}</small>
                                {item.read ? null : <button className="notification-popover-read" type="button" onClick={() => void markPopoverRead(item.id)}>Mark read</button>}
                            </article>) : <div className="notification-popover-empty">No notifications yet.</div>}
                        </div>
                        <button className="notification-popover-all" id="notification-popover-all" type="button" onClick={openAllNotifications}>View all notifications</button>
                    </div>
                </div>
                {update?.available && <a className="update-pill" href={update.release_url ?? "https://github.com/aczeccssa/taskdeck/releases"} target="_blank" rel="noreferrer">Update {update.latest_version}</a>}
                <button className="icon-button lang-toggle" id="lang" type="button" aria-label="Switch language" title="Switch language" onClick={toggleLanguage}>{language === "en" ? "EN" : "中"}</button>
                <button className="icon-button theme-toggle" id="theme" type="button" aria-label={`Color theme: ${theme}`} title={`Color theme: ${theme}`} onClick={cycleTheme}><span id="theme-icon" aria-hidden="true"><ThemeIcon/></span></button>
            </header>
            <Outlet/>
        </main>
        <div className="toast" id="toast" role="status" aria-live="polite"></div>
    </div>;
}

const ZH_NAV: Readonly<Record<View, string>> = {
    tasks: "任务",
    dashboard: "仪表盘",
    workflows: "工作流",
    boards: "看板",
    alerts: "告警",
    calls: "MCP 调用",
    audit: "审计日志",
    docs: "MCP 指南",
    settings: "设置",
};
