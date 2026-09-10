import { useEffect, useRef } from "react";
import type { JsonValue, McpCallRecord } from "../../domain/models";
import { formatTimestamp } from "../../lib/helpers";

function valueText(value: JsonValue | undefined): string {
    return typeof value === "string" ? value : JSON.stringify(value ?? null, null, 2);
}

export function CallDialog({
    record,
    mode,
    setMode,
    close,
}: {
    record: McpCallRecord | null;
    mode: "result" | "raw";
    setMode: (m: "result" | "raw") => void;
    close: () => void;
}): React.JSX.Element {
    const dialog = useRef<HTMLDialogElement>(null);
    useEffect(() => {
        if (record) dialog.current?.showModal();
        else if (dialog.current?.open) dialog.current.close();
    }, [record]);
    return (
        <dialog className="drawer call-dialog" id="call-dialog" ref={dialog} onClose={close}>
            <div className="drawer-shell">
                <header className="drawer-header call-header">
                    <div className="detail-status-icon" aria-hidden="true">
                        <svg viewBox="0 0 24 24">
                            <path d={record?.success ? "m7.5 12 3 3 6-7" : "m8 8 8 8m0-8-8 8"} />
                        </svg>
                    </div>
                    <div className="call-title">
                        <span>MCP call</span>
                        <h2>Call details</h2>
                    </div>
                    <button
                        className="icon-button"
                        type="button"
                        aria-label="Close details"
                        title="Close"
                        onClick={close}>
                        ×
                    </button>
                </header>
                {record && (
                    <div className="call-detail">
                        <div className="detail-modes" role="tablist" aria-label="Call detail view">
                            <button
                                className={mode === "result" ? "active" : ""}
                                role="tab"
                                aria-selected={mode === "result"}
                                onClick={() => setMode("result")}>
                                Result
                            </button>
                            <button
                                className={mode === "raw" ? "active" : ""}
                                role="tab"
                                aria-selected={mode === "raw"}
                                onClick={() => setMode("raw")}>
                                Raw
                            </button>
                        </div>
                        {mode === "result" ? (
                            <div className="call-result-view">
                                <div className="call-overview">
                                    <div className="overview-item">
                                        <span>Operation</span>
                                        <strong>{titleCase(record.operation || "MCP call")}</strong>
                                    </div>
                                    <div className="overview-item">
                                        <span>Duration</span>
                                        <strong>{record.duration_ms} ms</strong>
                                    </div>
                                </div>
                                <section className="detail-section">
                                    <header>
                                        <span>Request</span>
                                        <h3>What the agent asked for</h3>
                                    </header>
                                    <pre>{valueText(record.request)}</pre>
                                </section>
                                <section className="detail-section">
                                    <header>
                                        <span>Result</span>
                                        <h3>What Taskdeck returned</h3>
                                    </header>
                                    <pre>{valueText(record.response)}</pre>
                                </section>
                            </div>
                        ) : (
                            <div className="call-raw-view">
                                <section>
                                    <h3>Request</h3>
                                    <pre>{valueText(record.request)}</pre>
                                </section>
                                <section>
                                    <h3>Response</h3>
                                    <pre>{valueText(record.response)}</pre>
                                </section>
                            </div>
                        )}
                    </div>
                )}
            </div>
        </dialog>
    );
}

function titleCase(value: string): string {
    return value.replace(/[_-]+/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}
