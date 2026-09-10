import { type FormEvent, useState } from "react";
import type { ApiToken, NodeSettingsView, ServiceStatus, WorkspaceQuota } from "../../domain/models";
import { isRecord } from "../../lib/narrow";
import { post } from "../shared/api";
import { useRequest } from "../shared/useRequest";

export function SettingsView(): React.JSX.Element {
    const [nodeData] = useRequest<{ nodes?: Array<{ id: string; name: string }> }>("/api/nodes", { nodes: [] });
    const nodes = nodeData.nodes ?? [];
    const [selected, setSelected] = useState("");
    const [settings, busy, error, reload] = useRequest<NodeSettingsView>(
        selected ? `/api/nodes/${encodeURIComponent(selected)}/settings` : "/api/nodes/self/settings",
        {} as NodeSettingsView,
        (value) => {
            const record = value as { settings?: NodeSettingsView };
            return record.settings ?? (value as NodeSettingsView);
        },
    );
    const [quotas, , quotaError, quotaReload] = useRequest<{ quotas: WorkspaceQuota[]; sessions: string[] }>(
        "/api/quotas",
        { quotas: [], sessions: [] },
    );
    const [tokens, , tokenError, tokenReload] = useRequest<{ tokens: ApiToken[] }>("/api/tokens", { tokens: [] });
    const [service, , serviceError, serviceReload] = useRequest<ServiceStatus>(
        "/api/nodes/self/service?scope=user",
        {},
    );
    const [message, setMessage] = useState("");
    const saveNode = async (e: FormEvent<HTMLFormElement>) => {
        e.preventDefault();
        setMessage("Saving…");
        const form = new FormData(e.currentTarget);
        try {
            const r = await post(
                `/api/nodes/${encodeURIComponent(selected || "self")}/settings`,
                Object.fromEntries(form),
            );
            if (!r.ok) throw new Error(r.message);
            setMessage("Saved.");
            reload();
        } catch (x) {
            setMessage(x instanceof Error ? x.message : "Unable to save settings");
        }
    };
    const addQuota = async (e: FormEvent<HTMLFormElement>) => {
        e.preventDefault();
        const f = new FormData(e.currentTarget);
        const r = await post("/api/quotas", {
            session: f.get("session") || null,
            max_running_tasks: Number(f.get("max_running_tasks")),
        });
        setMessage(r.ok ? "Quota added." : r.message);
        quotaReload();
    };
    const createToken = async (e: FormEvent<HTMLFormElement>) => {
        e.preventDefault();
        const f = new FormData(e.currentTarget);
        const r = await post("/api/tokens", { name: f.get("name") });
        setMessage(
            r.ok
                ? `Token created: ${isRecord(r.data) && typeof r.data.secret === "string" ? r.data.secret : "secret shown once"}`
                : r.message,
        );
        tokenReload();
    };
    return (
        <section className="view active settings-view react-view" data-react-owned="true">
            <div className="settings-layout">
                <section className="settings-panel">
                    <header>
                        <div>
                            <h2>Node settings</h2>
                            <p>{busy ? "Loading…" : error || settings.name || "Configure this device."}</p>
                        </div>
                        <select aria-label="Node" value={selected} onChange={(e) => setSelected(e.target.value)}>
                            <option value="">This device</option>
                            {nodes.map((n) => (
                                <option key={n.id} value={n.id}>
                                    {n.name || n.id}
                                </option>
                            ))}
                        </select>
                    </header>
                    <form className="settings-form" onSubmit={saveNode}>
                        <div className="field-grid">
                            <label className="field">
                                <span>Name</span>
                                <input name="name" defaultValue={settings.name || ""} />
                            </label>
                            <label className="field">
                                <span>Role</span>
                                <select name="role" defaultValue={settings.role || "worker"}>
                                    <option>worker</option>
                                    <option>leader</option>
                                </select>
                            </label>
                            <label className="field">
                                <span>Bind host</span>
                                <input name="bind_host" defaultValue={settings.bind_host || ""} />
                            </label>
                            <label className="field">
                                <span>Web port</span>
                                <input name="web_port" type="number" defaultValue={settings.web_port || ""} />
                            </label>
                        </div>
                        <p className="settings-message" role="status">
                            {message}
                        </p>
                        <button className="button primary" type="submit">
                            Save settings
                        </button>
                    </form>
                </section>
                <SettingsList title="Workspace aliases">
                    <p className="muted">Select a workspace from the legacy workspace manager to edit its alias.</p>
                </SettingsList>
                <section className="settings-panel">
                    <header>
                        <div>
                            <h2>Resource quotas</h2>
                            <p>Limit concurrent tasks by workspace.</p>
                        </div>
                        <button className="button" onClick={quotaReload}>
                            Refresh
                        </button>
                    </header>
                    {quotaError ? (
                        <p className="form-error">{quotaError}</p>
                    ) : (
                        quotas.quotas.map((q) => (
                            <div className="quota-row" key={q.id}>
                                <strong>{q.session || "node"}</strong>
                                <span className="muted">max {q.max_running_tasks} running</span>
                                <button
                                    className="button compact danger"
                                    onClick={() => {
                                        void post(`/api/quotas/${encodeURIComponent(q.id)}`, undefined, "DELETE").then(
                                            quotaReload,
                                        );
                                    }}>
                                    Delete
                                </button>
                            </div>
                        ))
                    )}
                    <form className="settings-form" onSubmit={addQuota}>
                        <div className="field-grid compact-grid">
                            <input name="session" placeholder="Workspace (optional)" />
                            <input name="max_running_tasks" type="number" min="1" defaultValue="2" />
                        </div>
                        <button className="button primary" type="submit">
                            Add quota
                        </button>
                    </form>
                </section>
                <section className="settings-panel">
                    <header>
                        <div>
                            <h2>API tokens</h2>
                            <p>Secret is shown once at creation.</p>
                        </div>
                        <button className="button" onClick={tokenReload}>
                            Refresh
                        </button>
                    </header>
                    {tokenError ? (
                        <p className="form-error">{tokenError}</p>
                    ) : (
                        tokens.tokens.map((t) => (
                            <div className="token-row" key={t.id}>
                                <strong>{t.name}</strong>
                                <span className="muted">{t.token_prefix}</span>
                            </div>
                        ))
                    )}
                    <form className="settings-form" onSubmit={createToken}>
                        <input name="name" placeholder="Token name" required />
                        <button className="button primary" type="submit">
                            Create token
                        </button>
                    </form>
                </section>
                <section className="settings-panel">
                    <header>
                        <div>
                            <h2>Daemon startup</h2>
                            <p>Manage the native service for this device.</p>
                        </div>
                        <button className="button" onClick={serviceReload}>
                            Refresh
                        </button>
                    </header>
                    <p className="muted">
                        {serviceError || service.status || (service.running ? "Running" : "Stopped")}
                    </p>
                    <div className="settings-actions">
                        {["install", "uninstall", "start", "stop"].map((action) => (
                            <button
                                className="button"
                                key={action}
                                type="button"
                                onClick={async () => {
                                    setMessage("Working…");
                                    const r = await post("/api/nodes/self/service", { action, scope: "user" });
                                    setMessage(r.ok ? "Service operation completed" : r.message);
                                    serviceReload();
                                }}>
                                {action[0].toUpperCase() + action.slice(1)}
                            </button>
                        ))}
                    </div>
                </section>
            </div>
        </section>
    );
}
function SettingsList({ title, children }: { title: string; children: React.ReactNode }): React.JSX.Element {
    return (
        <section className="settings-panel">
            <header>
                <h2>{title}</h2>
            </header>
            {children}
        </section>
    );
}
