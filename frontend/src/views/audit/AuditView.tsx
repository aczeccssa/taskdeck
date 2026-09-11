import { useEffect, useMemo, useState } from "react";
import type { AuditListItem, AuditListPage, AuditRecord, Page } from "../../domain/models";
import { jsonValue } from "../../lib/narrow";
import { formatTimestamp } from "../../lib/helpers";
import { api } from "../shared/api";
import { objectDecoder, page, pageDecoder, text, num, bool } from "../shared/decode";
import { Pager } from "../shared/Pager";
import { AuditDialog } from "./AuditDialog";
import { auditQueryString } from "./query";

const auditItem = objectDecoder<AuditListItem>((r) => ({
    audit_id: text(r.audit_id),
    timestamp_ms: num(r.timestamp_ms),
    source: text(r.source, "unknown"),
    operation: text(r.operation),
    status: text(r.status, "unknown"),
    node_id: text(r.node_id) || null,
    session: text(r.session) || null,
    task: text(r.task) || null,
    duration_ms: num(r.duration_ms),
    replicated_at_ms: num(r.replicated_at_ms),
    summary: text(r.summary),
}));
const auditRecord = objectDecoder<AuditRecord>((r) => ({
    ...auditItem(r),
    context: jsonValue(r.context ?? null),
    request: jsonValue(r.request ?? null),
    response: jsonValue(r.response ?? null),
    error: text(r.error) || null,
}));

export function AuditView(): React.JSX.Element {
    const [filters, setFilters] = useState({
        q: "",
        source: "all",
        status: "all",
        node: "",
        session: "",
        task: "",
        operation: "",
        page: 1,
        pageSize: 20,
    });
    const [result, setResult] = useState<AuditListPage>({
        items: [],
        page: 1,
        page_size: 20,
        total: 0,
        total_pages: 0,
    });
    const [detail, setDetail] = useState<AuditRecord | null>(null);
    const [mode, setMode] = useState<"summary" | "raw">("summary");
    const query = useMemo(() => auditQueryString(filters), [filters]);
    const reload = () =>
        void api.request(`/api/audit?${query}`, pageDecoder(auditItem)).then((r) => {
            if (r.ok && r.data) setResult(page<AuditListPage>(r.data));
        });
    useEffect(() => {
        const t = window.setTimeout(reload, 200);
        return () => window.clearTimeout(t);
    }, [query]);
    const u = (key: keyof typeof filters, value: string) =>
        setFilters((f) => ({ ...f, [key]: value, page: key === "pageSize" ? 1 : f.page }));
    return (
        <section className="view active" data-react-owned="true" id="audit-view">
            <div className="audit-log">
                <header className="section-heading page-heading">
                    <div className="section-heading-actions">
                        <div><h1>Audit Log</h1><p className="muted">Trace configuration changes and operational actions across Taskdeck.</p></div>
                        <span id="audit-summary" className="sr-only">
                            {result.total} retained audit records
                        </span>
                    </div>
                </header>
                <div className="calls-toolbar audit-toolbar" aria-label="Audit filters">
                    <input
                        id="audit-search"
                        type="search"
                        placeholder="Search audit"
                        aria-label="Search audit"
                        value={filters.q}
                        onChange={(e) => u("q", e.target.value.trim())}
                    />
                    <select
                        id="audit-source"
                        aria-label="Filter by source"
                        value={filters.source}
                        onChange={(e) => u("source", e.target.value)}>
                        <option value="all">All sources</option>
                        <option value="api">API</option>
                        <option value="cli">CLI</option>
                        <option value="agent">Agent</option>
                    </select>
                    <select
                        id="audit-status"
                        aria-label="Filter by result"
                        value={filters.status}
                        onChange={(e) => u("status", e.target.value)}>
                        <option value="all">All results</option>
                        <option value="success">Success</option>
                        <option value="error">Error</option>
                    </select>
                    <input
                        id="audit-node"
                        placeholder="Node"
                        aria-label="Filter by node"
                        value={filters.node}
                        onChange={(e) => u("node", e.target.value.trim())}
                    />
                    <input
                        id="audit-session"
                        placeholder="Session"
                        aria-label="Filter by session"
                        value={filters.session}
                        onChange={(e) => u("session", e.target.value.trim())}
                    />
                    <input
                        id="audit-task"
                        placeholder="Task"
                        aria-label="Filter by task"
                        value={filters.task}
                        onChange={(e) => u("task", e.target.value.trim())}
                    />
                    <input
                        id="audit-operation"
                        placeholder="Operation"
                        aria-label="Filter by operation"
                        value={filters.operation}
                        onChange={(e) => u("operation", e.target.value.trim())}
                    />
                    <select
                        id="audit-page-size"
                        aria-label="Audit records per page"
                        value={filters.pageSize}
                        onChange={(e) => u("pageSize", e.target.value)}>
                        <option value={20}>20 / page</option>
                        <option value={50}>50 / page</option>
                        <option value={100}>100 / page</option>
                    </select>
                    <button className="button" id="refresh-audit" type="button" onClick={reload}>Refresh</button>
                    <button
                        className="button"
                        id="clear-audit-filters"
                        type="button"
                        aria-label="Clear audit filters"
                        onClick={() =>
                            setFilters({
                                q: "",
                                source: "all",
                                status: "all",
                                node: "",
                                session: "",
                                task: "",
                                operation: "",
                                page: 1,
                                pageSize: 20,
                            })
                        }>
                        Clear filters
                    </button>
                </div>
                <div className="calls-table-wrap audit-table-wrap">
                    <table className="calls-table audit-table">
                        <thead>
                            <tr>
                                <th>Time</th>
                                <th>Source</th>
                                <th>Node</th>
                                <th>Operation</th>
                                <th>Target</th>
                                <th>Result</th>
                                <th>Duration</th>
                                <th>Sync</th>
                                <th />
                            </tr>
                        </thead>
                        <tbody id="audit-body">
                            {result.items.length ? (
                                result.items.map((r) => (
                                    <tr key={r.audit_id}>
                                        <td>{new Date(r.timestamp_ms).toLocaleString()}</td>
                                        <td>
                                            <span className="source-pill">{r.source.toUpperCase()}</span>
                                        </td>
                                        <td>{r.node_id || "unknown"}</td>
                                        <td>{titleCase(r.operation)}</td>
                                        <td>{r.task || r.session || r.operation}</td>
                                        <td>
                                            <span
                                                className={`status-pill ${r.status === "error" ? "error" : "success"}`}>
                                                {titleCase(r.status)}
                                            </span>
                                        </td>
                                        <td>{r.duration_ms || 0} ms</td>
                                        <td>
                                            {r.replicated_at_ms
                                                ? `Synced ${new Date(r.replicated_at_ms).toLocaleTimeString()}`
                                                : "Local pending"}
                                        </td>
                                        <td>
                                            <button
                                                className="button compact"
                                                type="button"
                                                data-audit-id={r.audit_id}
                                                onClick={() =>
                                                    void api
                                                        .request(
                                                            `/api/audit/${encodeURIComponent(r.audit_id)}`,
                                                            objectDecoder(auditRecord),
                                                        )
                                                        .then((x) => {
                                                            if (x.ok && x.data) {
                                                                setDetail(page<AuditRecord>(x.data));
                                                                setMode("summary");
                                                            }
                                                        })
                                                }>
                                                View
                                            </button>
                                        </td>
                                    </tr>
                                ))
                            ) : (
                                <tr className="empty-row">
                                    <td colSpan={9}>No matching audit records.</td>
                                </tr>
                            )}
                        </tbody>
                    </table>
                </div>
                <Pager
                    page={result}
                    onPrev={() => setFilters((f) => ({ ...f, page: Math.max(1, f.page - 1) }))}
                    onNext={() => setFilters((f) => ({ ...f, page: f.page + 1 }))}
                    prefix="audit"
                />
            </div>
            <AuditDialog record={detail} mode={mode} setMode={setMode} close={() => setDetail(null)} />
        </section>
    );
}

function titleCase(value: string): string {
    return value.replace(/[_-]+/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}
