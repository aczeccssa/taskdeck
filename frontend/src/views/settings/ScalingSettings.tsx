import { useEffect, useMemo, useState } from "react";
import type { ScalingMetric, ScalingPoliciesView, ScalingPolicy, WorkflowTargetView } from "../../domain/models";
import { formatTimestamp } from "../../lib/helpers";
import { api, decode } from "../shared/api";
import { useRequest } from "../shared/useRequest";

export interface ScalingPolicyDraft {
    name: string; enabled: boolean; watch: string; metric: ScalingMetric; scale_out_threshold: string;
    scale_in_threshold: string; scale_out: string; cooldown_seconds: string;
}
const key = (node: string, session: string, task: string) => `${node}\u0000${session}\u0000${task}`;
const empty = (): ScalingPolicyDraft => ({name: "", enabled: true, watch: "", metric: "cpu_percent", scale_out_threshold: "80", scale_in_threshold: "20", scale_out: "", cooldown_seconds: "300"});
const fromPolicy = (policy: ScalingPolicy): ScalingPolicyDraft => ({name: policy.name, enabled: policy.enabled, watch: key(policy.watch_node_id, policy.watch_session, policy.watch_task), metric: policy.metric, scale_out_threshold: String(policy.scale_out_threshold), scale_in_threshold: String(policy.scale_in_threshold), scale_out: key(policy.scale_out_node_id, policy.scale_out_session, policy.scale_out_task), cooldown_seconds: String(policy.cooldown_seconds)});

export function scalingPayload(draft: ScalingPolicyDraft): Record<string, string | number | boolean> {
    const out = Number(draft.scale_out_threshold), inside = Number(draft.scale_in_threshold), cooldown = Number(draft.cooldown_seconds);
    if (!draft.name.trim() || !draft.watch || !draft.scale_out) throw new Error("Name, monitored task, and action target are required.");
    if (!Number.isFinite(out) || !Number.isFinite(inside) || inside >= out) throw new Error("The lower threshold must be smaller than the upper threshold.");
    if (!Number.isInteger(cooldown) || cooldown < 0) throw new Error("Cooldown must be zero or more seconds.");
    const [watch_node_id, watch_session, watch_task] = draft.watch.split("\u0000");
    const [scale_out_node_id, scale_out_session, scale_out_task] = draft.scale_out.split("\u0000");
    return {name: draft.name.trim(), enabled: draft.enabled, metric: draft.metric, scale_out_threshold: out, scale_in_threshold: inside, cooldown_seconds: cooldown, watch_node_id, watch_session, watch_task, scale_out_node_id, scale_out_session, scale_out_task};
}

export function ScalingSettings(): React.JSX.Element {
    const [data, busy, error, reload] = useRequest<ScalingPoliciesView>("/api/scaling-policies", {policies: [], targets: []});
    const [editing, setEditing] = useState<string | null>(null);
    const [draft, setDraft] = useState<ScalingPolicyDraft>(empty);
    const [message, setMessage] = useState("");
    const options = useMemo(() => data.targets.flatMap((target) => target.tasks.map((task) => ({value: key(target.node_id, target.session, task), label: `${target.workspace_display_name} / ${task}`, meta: target.node_name}))), [data.targets]);
    useEffect(() => { if (!draft.watch && options[0]) setDraft((value) => ({...value, watch: options[0].value, scale_out: options[0].value})); }, [options, draft.watch]);
    const begin = (policy?: ScalingPolicy) => { setEditing(policy?.id ?? ""); setDraft(policy ? fromPolicy(policy) : {...empty(), watch: options[0]?.value ?? "", scale_out: options[0]?.value ?? ""}); setMessage(""); };
    const save = async () => {
        let body: Record<string, string | number | boolean>;
        try { body = scalingPayload(draft); } catch (error) { setMessage(error instanceof Error ? error.message : "Invalid policy"); return; }
        const result = await api.request(editing ? `/api/scaling-policies/${encodeURIComponent(editing)}` : "/api/scaling-policies", decode, {method: editing ? "PUT" : "POST", headers: {"content-type": "application/json"}, body: JSON.stringify(body)});
        if (!result.ok) return setMessage(result.message || "Unable to save policy");
        setEditing(null); setMessage("Policy saved."); reload();
    };
    const updatePolicy = async (policy: ScalingPolicy, patch: Partial<ScalingPolicyDraft>) => {
        const next = {...fromPolicy(policy), ...patch};
        const [watch_node_id, watch_session, watch_task] = next.watch.split("\u0000");
        const [scale_out_node_id, scale_out_session, scale_out_task] = next.scale_out.split("\u0000");
        const result = await api.request(`/api/scaling-policies/${encodeURIComponent(policy.id)}`, decode, {method: "PUT", headers: {"content-type": "application/json"}, body: JSON.stringify({...next, scale_out_threshold: Number(next.scale_out_threshold), scale_in_threshold: Number(next.scale_in_threshold), cooldown_seconds: Number(next.cooldown_seconds), watch_node_id, watch_session, watch_task, scale_out_node_id, scale_out_session, scale_out_task})});
        setMessage(result.ok ? `Policy ${next.enabled ? "enabled" : "paused"}.` : result.message); reload();
    };
    const remove = async (policy: ScalingPolicy) => { if (!confirm(`Delete policy '${policy.name}'?`)) return; const result = await api.request(`/api/scaling-policies/${encodeURIComponent(policy.id)}`, decode, {method: "DELETE"}); setMessage(result.ok ? "Policy deleted." : result.message); reload(); };
    return <section className="settings-panel automation-panel">
        <header><div><h2>Automation</h2><p>Scale a target task from sustained CPU or memory pressure.</p></div><button className="button primary" onClick={() => begin()}>New policy</button></header>
        <div className="automation-explainer"><strong>How it works</strong><span>Taskdeck acts only after 3 consecutive samples cross a threshold, then waits for the cooldown before another action.</span></div>
        {busy ? <p className="muted">Loading policies…</p> : error ? <p className="form-error">{error}</p> : data.policies.length ? <div className="policy-list">{data.policies.map((policy) => <article className="policy-row" key={policy.id}><div><strong>{policy.name}</strong><span>{policy.enabled ? "Enabled" : "Paused"} · {policy.metric === "cpu_percent" ? "CPU" : "Memory"} · {policy.scale_in_threshold} → {policy.scale_out_threshold}</span><small>{policy.last_action ? `Last action: ${policy.last_action}${policy.last_action_ms ? ` · ${formatTimestamp(policy.last_action_ms)}` : ""}` : "No actions yet"}</small></div><div className="settings-actions"><button className="button compact" onClick={() => void updatePolicy(policy, {enabled: !policy.enabled})}>{policy.enabled ? "Pause" : "Enable"}</button><button className="button compact" onClick={() => begin(policy)}>Edit</button><button className="button compact danger" onClick={() => void remove(policy)}>Delete</button></div></article>)}</div> : <div className="empty-state compact"><div><h3>No automation policies</h3><p>Create one only when a task has a clear workload signal and target action.</p></div></div>}
        {editing !== null && <div className="settings-subpanel"><header><div><h3>{editing ? "Edit policy" : "New policy"}</h3><p>All targets are required so automation can never act on an ambiguous task.</p></div><button className="icon-button" aria-label="Close policy editor" onClick={() => setEditing(null)}>×</button></header>
            <div className="field-grid"><label className="field"><span>Policy name</span><input value={draft.name} onChange={(e) => setDraft({...draft, name: e.target.value})}/></label><label className="check-field"><input type="checkbox" checked={draft.enabled} onChange={(e) => setDraft({...draft, enabled: e.target.checked})}/><span>Enabled</span></label></div>
            <h4>Monitor</h4><div className="field-grid"><TargetField label="Monitored task" value={draft.watch} options={options} onChange={(watch) => setDraft({...draft, watch})}/><label className="field"><span>Metric</span><select value={draft.metric} onChange={(e) => setDraft({...draft, metric: e.target.value as ScalingMetric})}><option value="cpu_percent">CPU percent</option><option value="memory_bytes">Memory bytes</option></select></label></div>
            <h4>Thresholds</h4><div className="field-grid"><label className="field"><span>Upper threshold</span><input type="number" value={draft.scale_out_threshold} onChange={(e) => setDraft({...draft, scale_out_threshold: e.target.value})}/></label><label className="field"><span>Lower threshold</span><input type="number" value={draft.scale_in_threshold} onChange={(e) => setDraft({...draft, scale_in_threshold: e.target.value})}/></label></div>
            <h4>Action</h4><div className="field-grid"><TargetField label="Task to start or stop" value={draft.scale_out} options={options} onChange={(scale_out) => setDraft({...draft, scale_out})}/><label className="field"><span>Cooldown (seconds)</span><input type="number" min="0" value={draft.cooldown_seconds} onChange={(e) => setDraft({...draft, cooldown_seconds: e.target.value})}/></label></div>
            <div className="settings-message" role="status">{message}</div><div className="settings-actions"><button className="button" onClick={() => setEditing(null)}>Cancel</button><button className="button primary" onClick={() => void save()}>Save policy</button></div>
        </div>}
        {editing === null && <div className="settings-message" role="status">{message}</div>}
    </section>;
}

function TargetField({label, value, options, onChange}: {label: string; value: string; options: Array<{value: string; label: string; meta: string}>; onChange: (value: string) => void}): React.JSX.Element {
    return <label className="field"><span>{label}</span><select value={value} onChange={(event) => onChange(event.target.value)}><option value="">Select a task</option>{options.map((option) => <option key={option.value} value={option.value}>{option.label} · {option.meta}</option>)}</select></label>;
}
