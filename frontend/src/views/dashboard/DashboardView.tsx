import { useEffect, useRef } from "react";
import { useNavigate } from "react-router-dom";
import type { NodeMetricsView, ScalingPolicy } from "../../domain/models";
import { formatBytes } from "../../lib/helpers";
import { useRequest } from "../shared/useRequest";

export function DashboardView(): React.JSX.Element {
    const navigate = useNavigate();
    const [data, busy, error, refresh] = useRequest<NodeMetricsView>("/api/node-metrics", {
        nodes: [],
        task_status_counts: {},
    });
    const [policies, , policyError, refreshPolicies] = useRequest<{ policies: ScalingPolicy[] }>(
        "/api/scaling-policies",
        { policies: [] },
    );
    const canvas = useRef<HTMLCanvasElement>(null);
    useEffect(() => {
        const id = window.setInterval(() => {
            refresh();
            refreshPolicies();
        }, 5000);
        return () => window.clearInterval(id);
    }, []);
    useEffect(() => {
        const c = canvas.current;
        if (!c) return;
        const w = c.clientWidth || 220,
            h = c.clientHeight || 48,
            ratio = window.devicePixelRatio || 1;
        c.width = w * ratio;
        c.height = h * ratio;
        const x = c.getContext("2d");
        if (!x) return;
        x.setTransform(ratio, 0, 0, ratio, 0, 0);
        x.clearRect(0, 0, w, h);
        const samples = data.nodes[0]?.samples ?? [];
        if (samples.length < 2) return;
        x.strokeStyle = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim() || "#4a7dff";
        x.beginPath();
        samples.forEach((s, i) => {
            const px = (i / (samples.length - 1)) * w,
                py = h - (s.cpu_percent / 100) * (h - 4) - 2;
            i ? x.lineTo(px, py) : x.moveTo(px, py);
        });
        x.stroke();
    }, [data]);
    const counts = Object.entries(data.task_status_counts);
    return (
        <section className="view active dashboard-view react-view" data-react-owned="true">
            <div className="dashboard">
                <header className="section-heading page-heading">
                    <div><h1>Dashboard</h1><p className="muted">Health, resource trends, exceptions, and automation status across nodes.</p></div>
                    <div className="settings-actions">
                        <button className="button" onClick={refresh}>
                            Refresh
                        </button>
                    </div>
                </header>
                {busy && (
                    <div className="muted" aria-busy="true">
                        Loading metrics…
                    </div>
                )}
                {error && (
                    <div className="settings-message error" role="alert">
                        {error}{" "}
                        <button className="button compact" onClick={refresh}>
                            Retry
                        </button>
                    </div>
                )}{" "}
                {!busy && !error && (
                    <>
                        <div className="dashboard-status">
                            <h3>Status counts</h3>
                            <div className="dashboard-chips">
                                {counts.length ? (
                                    counts.map(([k, v]) => (
                                        <span key={k} className="status-pill">
                                            {k}: {v}
                                        </span>
                                    ))
                                ) : (
                                    <span className="muted">No metrics available</span>
                                )}
                            </div>
                        </div>
                        <div className="dashboard-nodes">
                            {data.nodes.length ? (
                                data.nodes.map((node) => {
                                    const current = node.current;
                                    const mem = current?.memory_total_bytes
                                        ? Math.round((current.memory_bytes / current.memory_total_bytes) * 100)
                                        : 0;
                                    return (
                                        <article
                                            className={`dashboard-node ${node.online ? "" : "offline"}`}
                                            key={node.node_id}>
                                            <header>
                                                <div>
                                                    <strong>{node.node_name || node.node_id}</strong>
                                                    <small>{node.node_id}</small>
                                                </div>
                                                <span className={`status-pill ${node.online ? "" : "error"}`}>
                                                    {node.online ? "online" : "offline"}
                                                </span>
                                            </header>
                                            <div className="dashboard-metrics">
                                                <div className="gauge">
                                                    <span>CPU</span>
                                                    <strong>
                                                        {current ? `${Math.round(current.cpu_percent)}%` : "–"}
                                                    </strong>
                                                    <div className="gauge-bar">
                                                        <i
                                                            style={{
                                                                width: `${Math.min(100, current?.cpu_percent ?? 0)}%`,
                                                            }}
                                                        />
                                                    </div>
                                                </div>
                                                <div className="gauge">
                                                    <span>Memory</span>
                                                    <strong>
                                                        {formatBytes(current?.memory_bytes ?? 0)} /{" "}
                                                        {formatBytes(current?.memory_total_bytes ?? 0)}
                                                    </strong>
                                                    <div className="gauge-bar memory">
                                                        <i style={{ width: `${mem}%` }} />
                                                    </div>
                                                </div>
                                            </div>
                                            <div className="dashboard-node-stats">
                                                <span>
                                                    <b>{current?.running_tasks ?? 0}</b> running tasks
                                                </span>
                                                <span>
                                                    <b>{node.session_count}</b> workspaces
                                                </span>
                                            </div>
                                            <canvas
                                                ref={node === data.nodes[0] ? canvas : undefined}
                                                className="sparkline"
                                                aria-hidden="true"
                                            />
                                        </article>
                                    );
                                })
                            ) : (
                                <div className="muted">No metrics available</div>
                            )}
                        </div>
                        <section className="workflow-card scaling-summary">
                            <header>
                                <div>
                                    <h2>Auto-scaling policies</h2>
                                    <p>Automation is configured in Settings, away from routine health checks.</p>
                                </div>
                                <button className="button" onClick={() => navigate("/settings?section=automation")}>Manage automation</button>
                            </header>
                            {policyError ? (
                                <div className="muted">{policyError}</div>
                            ) : policies.policies.length ? (
                                policies.policies.map((p) => (
                                    <div className="scaling-row" key={p.id}>
                                        <strong>{p.name}</strong>
                                        <span>
                                            {p.enabled ? "Enabled" : "Disabled"} · {p.metric}
                                        </span>
                                    </div>
                                ))
                            ) : (
                                <div className="muted">No scaling policies.</div>
                            )}
                        </section>
                    </>
                )}
            </div>
        </section>
    );
}
