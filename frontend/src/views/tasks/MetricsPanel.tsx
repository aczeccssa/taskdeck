import { useLayoutEffect, useRef } from "react";
import type { TaskMetricsSnapshot } from "../../domain/models";
import { formatBytes, formatRuntime } from "../../lib/helpers";
import { chartGeometry } from "./helpers";

function MetricChart({
    samples,
    metric,
    className,
    formatter,
    restartMarkers,
    title,
}: {
    samples: TaskMetricsSnapshot["samples"];
    metric: "cpu_percent" | "memory_bytes";
    className: string;
    formatter: (value: number) => string;
    restartMarkers: number[];
    title: string;
}): React.JSX.Element {
    const geometry = chartGeometry(samples, metric, restartMarkers);
    const current = Number(samples.at(-1)?.[metric] || 0);
    return (
        <div className="metric-chart">
            <div className="chart-title">
                <span>{title}</span>
                <strong>{formatter(current)}</strong>
            </div>
            <svg viewBox="0 0 300 72" preserveAspectRatio="none" aria-hidden="true">
                <path className="chart-grid" d="M0 70H300M0 38H300M0 6H300" />
                {geometry.markers.map((x, index) => (
                    <path className="chart-restart" key={index} d={`M${x} 4V70`} />
                ))}
                {geometry.points ? <polyline className={`chart-line ${className}`} points={geometry.points} /> : null}
            </svg>
        </div>
    );
}

export function MetricsPanel({ metrics }: { metrics: TaskMetricsSnapshot | null }): React.JSX.Element {
    const bodyRef = useRef<HTMLDivElement>(null);
    const scroll = useRef({ top: 0, left: 0 });
    useLayoutEffect(() => {
        const body = bodyRef.current;
        if (!body) return;
        body.scrollTop = scroll.current.top;
        body.scrollLeft = scroll.current.left;
    });
    if (!metrics)
        return (
            <aside className="monitor-panel" id="monitor-panel" aria-label="Task performance">
                <header className="monitor-header">
                    <strong>Performance</strong>
                    <span id="monitor-state">Waiting</span>
                </header>
                <div className="monitor-body" id="monitor-body" ref={bodyRef}>
                    <div className="monitor-empty">No samples</div>
                </div>
            </aside>
        );
    const current = metrics.current;
    const samples = metrics.samples;
    const restartMarkers = metrics.restart_markers_ms;
    return (
        <aside className="monitor-panel" id="monitor-panel" aria-label="Task performance">
            <header className="monitor-header">
                <strong>Performance</strong>
                <span id="monitor-state">{metrics.running ? "Live - 1s" : "Stopped"}</span>
            </header>
            <div
                className="monitor-body"
                id="monitor-body"
                ref={bodyRef}
                onScroll={(event) => {
                    scroll.current = { top: event.currentTarget.scrollTop, left: event.currentTarget.scrollLeft };
                }}>
                <div className="metric-summary">
                    <div>
                        <span>CPU</span>
                        <strong>{Number(current.cpu_percent || 0).toFixed(1)}%</strong>
                    </div>
                    <div>
                        <span>RSS</span>
                        <strong>{formatBytes(current.memory_bytes)}</strong>
                    </div>
                    <div>
                        <span>Processes</span>
                        <strong>{current.process_count || 0}</strong>
                    </div>
                </div>
                <div className="metric-charts">
                    <MetricChart
                        samples={samples}
                        metric="cpu_percent"
                        className=""
                        formatter={(value) => `${value.toFixed(1)}%`}
                        restartMarkers={restartMarkers}
                        title="CPU"
                    />
                    <MetricChart
                        samples={samples}
                        metric="memory_bytes"
                        className="memory"
                        formatter={formatBytes}
                        restartMarkers={restartMarkers}
                        title="Memory"
                    />
                </div>
                <div className="process-table-wrap">
                    {metrics.processes.length ? (
                        <table className="processes">
                            <thead>
                                <tr>
                                    <th>PID</th>
                                    <th>PPID</th>
                                    <th>Name</th>
                                    <th>CPU</th>
                                    <th>RSS</th>
                                    <th>Status</th>
                                    <th>Runtime</th>
                                </tr>
                            </thead>
                            <tbody>
                                {metrics.processes.map((process) => (
                                    <tr key={`${process.pid}-${process.name}`}>
                                        <td>{process.pid}</td>
                                        <td>{process.ppid ?? "-"}</td>
                                        <td className="name" title={process.name}>
                                            {process.name}
                                        </td>
                                        <td>{Number(process.cpu_percent || 0).toFixed(1)}%</td>
                                        <td>{formatBytes(process.memory_bytes)}</td>
                                        <td>{process.status}</td>
                                        <td>{formatRuntime(process.run_time_seconds)}</td>
                                    </tr>
                                ))}
                            </tbody>
                        </table>
                    ) : (
                        <div className="monitor-empty">No running processes</div>
                    )}
                </div>
            </div>
        </aside>
    );
}
