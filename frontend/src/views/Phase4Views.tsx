import {FormEvent, useCallback, useEffect, useRef, useState} from "react";
import {LegacyApiAdapter} from "../compat/api";
import type {ApiToken, NodeMetricsView, NodeSettingsView, ScalingPolicy, ServiceStatus, WorkspaceQuota} from "../domain/models";
import {isRecord} from "../lib/narrow";
import "../legacy/styles.css";

const api = new LegacyApiAdapter();
const decode = (value: unknown): unknown => value;
function useRequest<T>(url: string, fallback: T, map: (value: unknown) => T = (value => value as T)): [T, boolean, string, () => void] {
 const [data, setData] = useState(fallback); const [busy, setBusy] = useState(true); const [error, setError] = useState(""); const [tick, refresh] = useState(0);
 useEffect(() => { let live = true; setBusy(true); setError(""); void api.request(url, decode).then(r => { if (!live) return; if (!r.ok) throw new Error(r.message); setData(r.data === undefined ? fallback : map(r.data)); }).catch(e => live && setError(e instanceof Error ? e.message : "Unable to load data")).finally(() => live && setBusy(false)); return () => { live = false; }; }, [url, tick]);
 return [data, busy, error, () => refresh(v => v + 1)];
}
function fmtBytes(n: number): string { if (!n) return "0 B"; const units = ["B", "KB", "MB", "GB"]; let i = 0; let v = n; while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; } return `${v.toFixed(i ? 1 : 0)} ${units[i]}`; }
function post(url: string, body?: unknown, method = "POST") { return api.request(url, decode, {method, headers: body ? {"content-type": "application/json"} : undefined, body: body ? JSON.stringify(body) : undefined}); }

export function DashboardView(): React.JSX.Element {
 const [data, busy, error, refresh] = useRequest<NodeMetricsView>("/api/node-metrics", {nodes: [], task_status_counts: {}});
 const [policies, , policyError, refreshPolicies] = useRequest<{policies: ScalingPolicy[]}>("/api/scaling-policies", {policies: []});
 const [scalingOpen, setScalingOpen] = useState(false); const canvas = useRef<HTMLCanvasElement>(null);
 useEffect(() => { const id = window.setInterval(() => { refresh(); refreshPolicies(); }, 5000); return () => window.clearInterval(id); }, []);
 useEffect(() => { const c = canvas.current; if (!c) return; const w = c.clientWidth || 220, h = c.clientHeight || 48, ratio = window.devicePixelRatio || 1; c.width = w * ratio; c.height = h * ratio; const x = c.getContext("2d"); if (!x) return; x.setTransform(ratio, 0, 0, ratio, 0, 0); x.clearRect(0, 0, w, h); const samples = data.nodes[0]?.samples ?? []; if (samples.length < 2) return; x.strokeStyle = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim() || "#4a7dff"; x.beginPath(); samples.forEach((s, i) => { const px = i / (samples.length - 1) * w, py = h - s.cpu_percent / 100 * (h - 4) - 2; i ? x.lineTo(px, py) : x.moveTo(px, py); }); x.stroke(); }, [data]);
 const counts = Object.entries(data.task_status_counts); return <section className="view active dashboard-view react-view" data-react-owned="true"><div className="dashboard"><header className="section-heading"><div className="settings-actions"><button className="button" onClick={refresh}>Refresh</button><button className="button" onClick={() => setScalingOpen(true)}>Scaling settings</button></div></header>{busy && <div className="muted" aria-busy="true">Loading metrics…</div>}{error && <div className="settings-message error" role="alert">{error} <button className="button compact" onClick={refresh}>Retry</button></div>} {!busy && !error && <><div className="dashboard-status"><h3>Status counts</h3><div className="dashboard-chips">{counts.length ? counts.map(([k,v]) => <span key={k} className="status-pill">{k}: {v}</span>) : <span className="muted">No metrics available</span>}</div></div><div className="dashboard-nodes">{data.nodes.length ? data.nodes.map(node => { const current = node.current; const mem = current?.memory_total_bytes ? Math.round(current.memory_bytes / current.memory_total_bytes * 100) : 0; return <article className={`dashboard-node ${node.online ? "" : "offline"}`} key={node.node_id}><header><div><strong>{node.node_name || node.node_id}</strong><small>{node.node_id}</small></div><span className={`status-pill ${node.online ? "" : "error"}`}>{node.online ? "online" : "offline"}</span></header><div className="dashboard-metrics"><div className="gauge"><span>CPU</span><strong>{current ? `${Math.round(current.cpu_percent)}%` : "–"}</strong><div className="gauge-bar"><i style={{width: `${Math.min(100, current?.cpu_percent ?? 0)}%`}} /></div></div><div className="gauge"><span>Memory</span><strong>{fmtBytes(current?.memory_bytes ?? 0)} / {fmtBytes(current?.memory_total_bytes ?? 0)}</strong><div className="gauge-bar memory"><i style={{width: `${mem}%`}} /></div></div></div><div className="dashboard-node-stats"><span><b>{current?.running_tasks ?? 0}</b> running tasks</span><span><b>{node.session_count}</b> workspaces</span></div><canvas ref={node === data.nodes[0] ? canvas : undefined} className="sparkline" aria-hidden="true" /></article>; }) : <div className="muted">No metrics available</div>}</div><section className="workflow-card scaling-summary"><header><div><h2>Auto-scaling policies</h2><p>Configure policies in Scaling settings.</p></div></header>{policyError ? <div className="muted">{policyError}</div> : policies.policies.length ? policies.policies.map(p => <div className="scaling-row" key={p.id}><strong>{p.name}</strong><span>{p.enabled ? "Enabled" : "Disabled"} · {p.metric}</span></div>) : <div className="muted">No scaling policies.</div>}</section></>}</div><ScalingDialog open={scalingOpen} close={() => setScalingOpen(false)} onSaved={() => { refreshPolicies(); setScalingOpen(false); }} /></section>;
}
function ScalingDialog({open, close, onSaved}: {open: boolean; close: () => void; onSaved: () => void}): React.JSX.Element {
    const dialogRef = useRef<HTMLDialogElement>(null);
    const [name, setName] = useState("");
    const [message, setMessage] = useState("");
    useEffect(() => {
        const dialog = dialogRef.current;
        if (!dialog) return;
        if (open && !dialog.open) dialog.showModal();
        if (!open && dialog.open) dialog.close();
    }, [open]);
    useEffect(() => {
        if (!open) return;
        setName("");
        setMessage("");
    }, [open]);
    const submit = async (e: FormEvent) => {
        e.preventDefault();
        setMessage("Saving…");
        try {
            const r = await post("/api/scaling-policies", {name, enabled: true, metric: "cpu_percent", scale_out_threshold: 80, scale_in_threshold: 20, cooldown_seconds: 300});
            if (!r.ok) throw new Error(r.message);
            onSaved();
        } catch (x) {
            setMessage(x instanceof Error ? x.message : "Unable to save policy");
        }
    };
    return <dialog className="drawer settings-drawer" ref={dialogRef} onClose={close} onClick={(event) => { if (event.target === dialogRef.current) close(); }}>
        <div className="drawer-shell">
            <header className="drawer-header">
                <div><span>Dashboard</span><h2>Scaling settings</h2></div>
                <button className="icon-button" type="button" onClick={close} aria-label="Close scaling settings">×</button>
            </header>
            <form className="settings-form" onSubmit={submit}>
                <label className="field"><span>Policy name</span><input value={name} onChange={e => setName(e.target.value)} required /></label>
                <p className="settings-message" role="status">{message}</p>
                <button className="button primary" type="submit">Save</button>
            </form>
        </div>
    </dialog>;
}

export function SettingsView(): React.JSX.Element { const [nodeData] = useRequest<{nodes?: Array<{id:string;name:string}>}>("/api/nodes", {nodes: []}); const nodes = nodeData.nodes ?? []; const [selected,setSelected]=useState(""); const [settings, busy, error, reload]=useRequest<NodeSettingsView>(selected ? `/api/nodes/${encodeURIComponent(selected)}/settings` : "/api/nodes/self/settings", {} as NodeSettingsView, value => { const record = value as {settings?: NodeSettingsView}; return record.settings ?? value as NodeSettingsView; }); const [quotas,,quotaError,quotaReload]=useRequest<{quotas:WorkspaceQuota[];sessions:string[]}>("/api/quotas", {quotas:[],sessions:[]}); const [tokens,,tokenError,tokenReload]=useRequest<{tokens:ApiToken[]}>("/api/tokens", {tokens:[]}); const [service,,serviceError,serviceReload]=useRequest<ServiceStatus>("/api/nodes/self/service?scope=user", {}); const [message,setMessage]=useState("");
 const saveNode=async(e:FormEvent<HTMLFormElement>)=>{e.preventDefault(); setMessage("Saving…"); const form=new FormData(e.currentTarget); try {const r=await post(`/api/nodes/${encodeURIComponent(selected || "self")}/settings`, Object.fromEntries(form)); if(!r.ok) throw new Error(r.message); setMessage("Saved."); reload();}catch(x){setMessage(x instanceof Error?x.message:"Unable to save settings");}};
 const addQuota=async(e:FormEvent<HTMLFormElement>)=>{e.preventDefault(); const f=new FormData(e.currentTarget); const r=await post("/api/quotas",{session:(f.get("session")||null),max_running_tasks:Number(f.get("max_running_tasks"))}); setMessage(r.ok?"Quota added.":r.message); quotaReload();};
 const createToken=async(e:FormEvent<HTMLFormElement>)=>{e.preventDefault(); const f=new FormData(e.currentTarget); const r=await post("/api/tokens",{name:f.get("name")}); setMessage(r.ok?`Token created: ${isRecord(r.data)&&typeof r.data.secret === "string" ? r.data.secret : "secret shown once"}`:r.message); tokenReload();};
 return <section className="view active settings-view react-view" data-react-owned="true"><div className="settings-layout"><section className="settings-panel"><header><div><h2>Node settings</h2><p>{busy?"Loading…":error||settings.name||"Configure this device."}</p></div><select aria-label="Node" value={selected} onChange={e=>setSelected(e.target.value)}><option value="">This device</option>{nodes.map(n=><option key={n.id} value={n.id}>{n.name || n.id}</option>)}</select></header><form className="settings-form" onSubmit={saveNode}><div className="field-grid"><label className="field"><span>Name</span><input name="name" defaultValue={settings.name || ""} /></label><label className="field"><span>Role</span><select name="role" defaultValue={settings.role || "worker"}><option>worker</option><option>leader</option></select></label><label className="field"><span>Bind host</span><input name="bind_host" defaultValue={settings.bind_host || ""} /></label><label className="field"><span>Web port</span><input name="web_port" type="number" defaultValue={settings.web_port || ""} /></label></div><p className="settings-message" role="status">{message}</p><button className="button primary" type="submit">Save settings</button></form></section><SettingsList title="Workspace aliases"><p className="muted">Select a workspace from the legacy workspace manager to edit its alias.</p></SettingsList><section className="settings-panel"><header><div><h2>Resource quotas</h2><p>Limit concurrent tasks by workspace.</p></div><button className="button" onClick={quotaReload}>Refresh</button></header>{quotaError?<p className="form-error">{quotaError}</p>:quotas.quotas.map(q=><div className="quota-row" key={q.id}><strong>{q.session||"node"}</strong><span className="muted">max {q.max_running_tasks} running</span><button className="button compact danger" onClick={()=>{void post(`/api/quotas/${encodeURIComponent(q.id)}`,undefined,"DELETE").then(quotaReload)}}>Delete</button></div>)}<form className="settings-form" onSubmit={addQuota}><div className="field-grid compact-grid"><input name="session" placeholder="Workspace (optional)" /><input name="max_running_tasks" type="number" min="1" defaultValue="2" /></div><button className="button primary" type="submit">Add quota</button></form></section><section className="settings-panel"><header><div><h2>API tokens</h2><p>Secret is shown once at creation.</p></div><button className="button" onClick={tokenReload}>Refresh</button></header>{tokenError?<p className="form-error">{tokenError}</p>:tokens.tokens.map(t=><div className="token-row" key={t.id}><strong>{t.name}</strong><span className="muted">{t.token_prefix}</span></div>)}<form className="settings-form" onSubmit={createToken}><input name="name" placeholder="Token name" required /><button className="button primary" type="submit">Create token</button></form></section><section className="settings-panel"><header><div><h2>Daemon startup</h2><p>Manage the native service for this device.</p></div><button className="button" onClick={serviceReload}>Refresh</button></header><p className="muted">{serviceError || service.status || (service.running ? "Running" : "Stopped")}</p><div className="settings-actions">{["install","uninstall","start","stop"].map(action => <button className="button" key={action} type="button" onClick={async()=>{setMessage("Working…"); const r=await post("/api/nodes/self/service",{action,scope:"user"}); setMessage(r.ok ? "Service operation completed" : r.message); serviceReload();}}>{action[0].toUpperCase()+action.slice(1)}</button>)}</div></section></div></section>; }
function SettingsList({title,children}:{title:string;children:React.ReactNode}):React.JSX.Element{return <section className="settings-panel"><header><h2>{title}</h2></header>{children}</section>}
