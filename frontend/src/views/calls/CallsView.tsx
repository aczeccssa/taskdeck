import { useEffect, useMemo, useState } from "react";
import type { McpCallListItem, McpCallListPage, McpCallRecord, Page } from "../../domain/models";
import { jsonValue } from "../../lib/narrow";
import { formatTimestamp } from "../../lib/helpers";
import { api } from "../shared/api";
import { objectDecoder, page, pageDecoder, text, num, bool } from "../shared/decode";
import { Pager } from "../shared/Pager";
import { CallDialog } from "./CallDialog";
import { callsQueryString } from "./query";

const callItem = objectDecoder<McpCallListItem>((r) => ({
    id: num(r.id),
    operation: text(r.operation) || null,
    tool: text(r.tool),
    target_node: text(r.target_node) || null,
    input: jsonValue(r.input ?? null),
    success: bool(r.success),
    started_at_ms: num(r.started_at_ms),
    duration_ms: num(r.duration_ms),
}));
const callRecord = objectDecoder<McpCallRecord>((r) => ({
    ...callItem(r),
    request: jsonValue(r.request ?? null),
    response: jsonValue(r.response ?? null),
}));
function titleCase(value: string): string {
    return value.replace(/[_-]+/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}
function targetForCall(call: McpCallListItem): [string, string] {
    return [call.tool || "MCP call", call.target_node || "No target"];
}

export function CallsView(): React.JSX.Element {
    const [filters, setFilters] = useState({
        q: "",
        operation: "",
        status: "all",
        session: "",
        task: "",
        page: 1,
        pageSize: 20,
    });
    const [result, setResult] = useState<McpCallListPage>({
        items: [],
        page: 1,
        page_size: 20,
        total: 0,
        total_pages: 0,
    });
    const [detail, setDetail] = useState<McpCallRecord | null>(null);
    const [mode, setMode] = useState<"result" | "raw">("result");
    const query = useMemo(() => callsQueryString(filters), [filters]);
    const reload = () =>
        void api.request(`/api/mcp-calls?${query}`, pageDecoder(callItem)).then((r) => {
            if (r.ok && r.data) setResult(page<McpCallListPage>(r.data));
        });
    useEffect(() => {
        const t = window.setTimeout(reload, 200);
        return () => window.clearTimeout(t);
    }, [query]);
    const update = (key: keyof typeof filters, value: string) =>
        setFilters((f) => ({ ...f, [key]: value, page: key === "pageSize" ? 1 : f.page }));
    return (
        <section className="view active" data-react-owned="true" id="calls-view">
            <div className="mcp-calls">
                <header className="section-heading page-heading">
                    <div className="section-heading-actions">
                        <div><h1>MCP Calls</h1><p className="muted">Inspect tool activity, targets, outcomes, and timing.</p></div>
                        <span id="calls-summary" className="sr-only">
                            {result.total} retained calls
                        </span>
                    </div>
                </header>
                <div className="calls-toolbar" aria-label="MCP call filters">
                    <label className="search-field">
                        <span className="sr-only">Search calls</span>
                        <input
                            id="calls-search"
                            type="search"
                            placeholder="Search calls"
                            value={filters.q}
                            onChange={(e) => update("q", e.target.value.trim())}
                        />
                    </label>
                    <select
                        id="calls-operation"
                        aria-label="Filter by operation"
                        value={filters.operation}
                        onChange={(e) => update("operation", e.target.value)}>
                        <option value="">All operations</option>
                        {["sessions", "status", "logs", "start", "stop", "restart", "pause", "resume"].map((v) => (
                            <option key={v} value={v}>
                                {titleCase(v)}
                            </option>
                        ))}
                    </select>
                    <select
                        id="calls-status"
                        aria-label="Filter by result"
                        value={filters.status}
                        onChange={(e) => update("status", e.target.value)}>
                        <option value="all">All results</option>
                        <option value="success">Success</option>
                        <option value="error">Error</option>
                    </select>
                    <input
                        id="calls-session"
                        placeholder="Session"
                        aria-label="Filter by exact session"
                        value={filters.session}
                        onChange={(e) => update("session", e.target.value.trim())}
                    />
                    <input
                        id="calls-task"
                        placeholder="Task"
                        aria-label="Filter by exact task"
                        value={filters.task}
                        onChange={(e) => update("task", e.target.value.trim())}
                    />
                    <select
                        id="calls-page-size"
                        aria-label="Calls per page"
                        value={filters.pageSize}
                        onChange={(e) => update("pageSize", e.target.value)}>
                        <option value={20}>20 / page</option>
                        <option value={50}>50 / page</option>
                        <option value={100}>100 / page</option>
                    </select>
                    <button className="button" id="refresh-calls" type="button" onClick={reload}>Refresh</button>
                    <button
                        className="button"
                        id="clear-call-filters"
                        type="button"
                        aria-label="Clear filters"
                        title="Clear filters"
                        onClick={() =>
                            setFilters({
                                q: "",
                                operation: "",
                                status: "all",
                                session: "",
                                task: "",
                                page: 1,
                                pageSize: 20,
                            })
                        }>
                        Clear filters
                    </button>
                </div>
                <div className="calls-table-wrap">
                    <table className="calls-table mcp-table">
                        <thead>
                            <tr>
                                <th>Operation</th>
                                <th>Target</th>
                                <th>Result</th>
                                <th>Started</th>
                                <th>Duration</th>
                                <th />
                            </tr>
                        </thead>
                        <tbody id="calls-body">
                            {result.items.length ? (
                                result.items.map((call) => {
                                    const target = targetForCall(call);
                                    return (
                                        <tr key={call.id}>
                                            <td>
                                                <div className="cell-stack">
                                                    <strong>{titleCase(call.operation || "MCP call")}</strong>
                                                    <span>{call.tool}</span>
                                                </div>
                                            </td>
                                            <td>
                                                <div className="cell-stack">
                                                    <strong>{target[0]}</strong>
                                                    <span>{target[1]}</span>
                                                </div>
                                            </td>
                                            <td>
                                                <span className={`status-pill ${call.success ? "" : "error"}`}>
                                                    {call.success ? "Success" : "Error"}
                                                </span>
                                            </td>
                                            <td>{new Date(call.started_at_ms).toLocaleString()}</td>
                                            <td>{call.duration_ms} ms</td>
                                            <td>
                                                <button
                                                    className="button compact"
                                                    type="button"
                                                    data-call-id={call.id}
                                                    onClick={() =>
                                                        void api
                                                            .request(
                                                                `/api/mcp-calls/${call.id}`,
                                                                objectDecoder(callRecord),
                                                            )
                                                            .then((r) => {
                                                                if (r.ok && r.data) {
                                                                    setDetail(page<McpCallRecord>(r.data));
                                                                    setMode("result");
                                                                }
                                                            })
                                                    }>
                                                    View
                                                </button>
                                            </td>
                                        </tr>
                                    );
                                })
                            ) : (
                                <tr className="empty-row">
                                    <td colSpan={6}>No matching MCP calls.</td>
                                </tr>
                            )}
                        </tbody>
                    </table>
                </div>
                <Pager
                    page={result}
                    onPrev={() => setFilters((f) => ({ ...f, page: Math.max(1, f.page - 1) }))}
                    onNext={() => setFilters((f) => ({ ...f, page: f.page + 1 }))}
                    prefix="calls"
                />
            </div>
            <CallDialog record={detail} mode={mode} setMode={setMode} close={() => setDetail(null)} />
        </section>
    );
}
