import { type FormEvent, useEffect, useState } from "react";
import type { ApiToken, NodeSettingsView, ServiceStatus, WorkspaceQuota } from "../../domain/models";
import { isRecord } from "../../lib/narrow";
import { useSelection } from "../../shell/SelectionContext";
import { api, decode, post } from "../shared/api";
import { useRequest } from "../shared/useRequest";
import { ScalingSettings } from "./ScalingSettings";

type Section = "nodes" | "workspaces" | "automation" | "access" | "runtime";

export function SettingsView(): React.JSX.Element {
    const selection = useSelection();
    const querySection = new URLSearchParams(location.search).get("section");
    const [section, setSection] = useState<Section>(querySection === "automation" ? "automation" : "nodes");
    const nodeId = selection.selectedNode || "self";
    const node = selection.nodes.find((item) => item.id === selection.selectedNode);
    const [settings, busy, error, reload] = useRequest<NodeSettingsView>(`/api/nodes/${encodeURIComponent(nodeId)}/settings`, {} as NodeSettingsView, (value) => (value as {settings?: NodeSettingsView}).settings ?? value as NodeSettingsView);
    const [quotas, , quotaError, quotaReload] = useRequest<{quotas: WorkspaceQuota[]; sessions: string[]}>("/api/quotas", {quotas: [], sessions: []});
    const [tokens, , tokenError, tokenReload] = useRequest<{tokens: ApiToken[]}>("/api/tokens", {tokens: []});
    const [service, , serviceError, serviceReload] = useRequest<ServiceStatus>("/api/nodes/self/service?scope=user", {});
    const [message, setMessage] = useState("");
    const [disconnecting, setDisconnecting] = useState<string | null>(null);
    useEffect(() => setMessage(""), [nodeId, section]);
    const choose = (next: Section) => { setSection(next); history.replaceState(null, "", `/settings${next === "nodes" ? "" : `?section=${next}`}`); };
    const saveNode = async (event: FormEvent<HTMLFormElement>) => { event.preventDefault(); const form = new FormData(event.currentTarget); setMessage("Saving…"); const result = await post(`/api/nodes/${encodeURIComponent(nodeId)}/settings`, Object.fromEntries(form)); setMessage(result.ok ? "Node settings saved." : result.message); if (result.ok) { reload(); void selection.refresh(); } };
    const renameWorkspace = async (session: string, alias: string) => { const result = await api.request(`/api/workspaces/${encodeURIComponent(session)}/alias?${new URLSearchParams({node: nodeId})}`, decode, {method: "PUT", headers: {"content-type": "application/json"}, body: JSON.stringify({alias: alias.trim() || null})}); setMessage(result.ok ? "Workspace name saved." : result.message); if (result.ok) await selection.refresh(); };
    const disconnectNode = async (nodeIdToRemove: string, nodeName: string): Promise<void> => {
        if (!window.confirm(`Disconnect ${nodeName}? Its saved connection will be removed from this leader.`)) return;
        setDisconnecting(nodeIdToRemove);
        const result = await api.request(`/api/nodes/${encodeURIComponent(nodeIdToRemove)}`, decode, {method: "DELETE"});
        setDisconnecting(null);
        setMessage(result.ok ? `${nodeName} disconnected.` : result.message);
        if (result.ok) await selection.refresh();
    };
    const addQuota = async (event: FormEvent<HTMLFormElement>) => { event.preventDefault(); const form = new FormData(event.currentTarget); const result = await post("/api/quotas", {session: form.get("session") || null, max_running_tasks: Number(form.get("max_running_tasks"))}); setMessage(result.ok ? "Quota added." : result.message); quotaReload(); };
    const createToken = async (event: FormEvent<HTMLFormElement>) => { event.preventDefault(); const form = new FormData(event.currentTarget); const result = await post("/api/tokens", {name: form.get("name")}); setMessage(result.ok ? `Token created: ${isRecord(result.data) && typeof result.data.secret === "string" ? result.data.secret : "secret shown once"}` : result.message); tokenReload(); };
    return <section className="view active settings-view react-view" data-react-owned="true">
        <header className="section-heading page-heading"><div><h1>Settings</h1><p className="muted">Configure {node?.name ?? "the selected node"}. Advanced controls stay separated from everyday setup.</p></div></header>
        <div className="settings-shell">
            <nav className="settings-nav" aria-label="Settings sections"><span>Basic settings</span><button aria-current={section === "nodes"} onClick={() => choose("nodes")}>Nodes</button><button aria-current={section === "workspaces"} onClick={() => choose("workspaces")}>Workspaces</button><span>Advanced settings</span><button aria-current={section === "automation"} onClick={() => choose("automation")}>Automation</button><button aria-current={section === "access"} onClick={() => choose("access")}>Access</button><button aria-current={section === "runtime"} onClick={() => choose("runtime")}>Runtime</button></nav>
            <div className="settings-content" key={`${section}-${nodeId}`}>
                {section === "nodes" && <>
                    <section className="settings-panel node-manager-panel">
                        <header><div><h2>Connected nodes</h2><p>Choose a node to configure it. Disconnect removes a saved remote connection; it does not delete the node's own data.</p></div><span className="status-pill">{selection.nodes.length} connected</span></header>
                        <div className="node-manager-list" role="listbox" aria-label="Connected nodes">
                            {selection.nodes.map((candidate) => <div className={`node-manager-row${candidate.id === selection.selectedNode ? " selected" : ""}`} key={candidate.id} role="option" aria-selected={candidate.id === selection.selectedNode}>
                                <button className="node-manager-select" type="button" onClick={() => selection.selectNode(candidate.id)}><span className={`status-dot ${candidate.online ? "success" : "muted"}`} /><span><strong>{candidate.name}</strong><small>{candidate.id} · {candidate.is_self ? "This device" : candidate.online ? "Online" : "Offline"}</small></span></button>
                                {!candidate.is_self && <button className="button compact danger" type="button" disabled={disconnecting === candidate.id} onClick={() => void disconnectNode(candidate.id, candidate.name)}>{disconnecting === candidate.id ? "Disconnecting…" : "Disconnect"}</button>}
                            </div>)}
                        </div>
                    </section>
                    <section className="settings-panel"><header><div><h2>Node configuration</h2><p>{busy ? "Loading…" : error || "The display name is editable; the stable ID is not."}</p></div><code>{nodeId}</code></header>{error ? <p className="form-error">{error}</p> : <form className="settings-form" onSubmit={saveNode}><label className="field"><span>Display name</span><input name="name" defaultValue={settings.name ?? ""} required /></label><details><summary>Advanced node configuration</summary><div className="field-grid"><label className="field"><span>Role</span><select name="role" defaultValue={settings.role ?? "worker"}><option value="worker">Worker</option><option value="leader">Leader</option></select></label><label className="field"><span>Leader URL</span><input name="leader_url" defaultValue={settings.leader_url ?? ""}/></label><label className="field"><span>Bind host</span><input name="bind_host" defaultValue={settings.bind_host ?? ""}/></label><label className="field"><span>Web port</span><input name="web_port" type="number" defaultValue={settings.web_port ?? ""}/></label></div></details><p className="settings-message" role="status">{message}</p><button className="button primary" type="submit">Save node</button></form>}</section>
                </>}
                {section === "workspaces" && <section className="settings-panel"><header><div><h2>Workspace display names</h2><p>Names are shown in Taskdeck while session IDs stay stable.</p></div></header>{selection.workspaces.length ? selection.workspaces.map((workspace) => <WorkspaceAlias key={workspace.session} session={workspace.session} alias={workspace.alias ?? ""} onSave={renameWorkspace}/>) : <p className="muted">No workspaces on this node.</p>}<details><summary>Advanced resource quotas</summary>{quotaError ? <p className="form-error">{quotaError}</p> : quotas.quotas.map((quota) => <div className="quota-row" key={quota.id}><strong>{quota.session || "Node default"}</strong><span>Maximum {quota.max_running_tasks} running</span><button className="button compact danger" onClick={() => void post(`/api/quotas/${encodeURIComponent(quota.id)}`, undefined, "DELETE").then(quotaReload)}>Delete</button></div>)}<form className="settings-form" onSubmit={addQuota}><div className="field-grid"><label className="field"><span>Workspace</span><select name="session"><option value="">Node default</option>{selection.sessions.map((session) => <option key={session}>{session}</option>)}</select></label><label className="field"><span>Maximum running tasks</span><input name="max_running_tasks" type="number" min="1" defaultValue="2"/></label></div><button className="button" type="submit">Add quota</button></form></details><p className="settings-message" role="status">{message}</p></section>}
                {section === "automation" && <ScalingSettings/>}
                {section === "access" && <section className="settings-panel"><header><div><h2>API access</h2><p>Tokens grant programmatic access. A secret is shown only once.</p></div><button className="button" onClick={tokenReload}>Refresh</button></header>{tokenError ? <p className="form-error">{tokenError}</p> : tokens.tokens.map((token) => <div className="token-row" key={token.id}><strong>{token.name}</strong><span>{token.token_prefix}</span></div>)}<form className="settings-form" onSubmit={createToken}><label className="field"><span>Token name</span><input name="name" required placeholder="CI agent"/></label><button className="button primary">Create token</button></form><p className="settings-message" role="status">{message}</p></section>}
                {section === "runtime" && <section className="settings-panel"><header><div><h2>Local runtime</h2><p>Native service controls apply only to the device running this UI.</p></div>{node?.is_self && <button className="button" onClick={serviceReload}>Refresh</button>}</header>{node?.is_self ? <><p className="muted">{serviceError || service.status || (service.running ? "Running" : "Stopped")}</p><div className="settings-actions">{["install", "uninstall", "start", "stop"].map((action) => <button className="button" key={action} onClick={async () => { const result = await post("/api/nodes/self/service", {action, scope: "user"}); setMessage(result.ok ? "Service operation completed." : result.message); serviceReload(); }}>{action[0].toUpperCase() + action.slice(1)}</button>)}</div></> : <div className="empty-state compact"><div><h3>Remote node selected</h3><p>Runtime controls are intentionally unavailable because they would operate on this local machine, not {node?.name ?? nodeId}.</p></div></div>}<p className="settings-message" role="status">{message}</p></section>}
            </div>
        </div>
    </section>;
}

function WorkspaceAlias({session, alias, onSave}: {session: string; alias: string; onSave: (session: string, alias: string) => Promise<void>}): React.JSX.Element {
    const [value, setValue] = useState(alias);
    return <div className="workspace-alias-row"><div><strong>{alias || session}</strong><code>{session}</code></div><input aria-label={`Display name for ${session}`} value={value} onChange={(event) => setValue(event.target.value)} placeholder="Display name"/><button className="button compact" onClick={() => void onSave(session, value)}>Save</button></div>;
}
