import { useEffect, useRef } from "react";
import type { AuditRecord, JsonValue } from "../../domain/models";
import { formatTimestamp } from "../../lib/helpers";

function valueText(value: JsonValue | undefined): string {
    return typeof value === "string" ? value : JSON.stringify(value ?? null, null, 2);
}

export function AuditDialog({
    record,
    mode,
    setMode,
    close,
}: {
    record: AuditRecord | null;
    mode: "summary" | "raw";
    setMode: (m: "summary" | "raw") => void;
    close: () => void;
}): React.JSX.Element {
    const ref = useRef<HTMLDialogElement>(null);
    useEffect(() => {
        if (record) ref.current?.showModal();
        else if (ref.current?.open) ref.current.close();
    }, [record]);
    return (
        <dialog className="drawer audit-dialog" id="audit-dialog" ref={ref} onClose={close}>
            <div className="drawer-shell">
                <header className="drawer-header call-header">
                    <div className="detail-status-icon" aria-hidden="true">
                        <svg viewBox="0 0 24 24">
                            <path d="m7.5 12 3 3 6-7" />
                        </svg>
                    </div>
                    <div className="call-title">
                        <span>Audit record</span>
                        <h2>Audit details</h2>
                    </div>
                    <button className="icon-button" type="button" aria-label="Close audit details" onClick={close}>
                        ×
                    </button>
                </header>
                {record && (
                    <div className="call-detail">
                        <div className="detail-modes" role="tablist" aria-label="Audit detail view">
                            <button
                                className={mode === "summary" ? "active" : ""}
                                role="tab"
                                aria-selected={mode === "summary"}
                                onClick={() => setMode("summary")}>
                                Summary
                            </button>
                            <button
                                className={mode === "raw" ? "active" : ""}
                                role="tab"
                                aria-selected={mode === "raw"}
                                onClick={() => setMode("raw")}>
                                Raw
                            </button>
                        </div>
                        {mode === "summary" ? (
                            <div className="call-result-view">
                                <div className="call-overview">
                                    <div className="overview-item">
                                        <span>Time</span>
                                        <strong>{new Date(record.timestamp_ms).toLocaleString()}</strong>
                                    </div>
                                    <div className="overview-item">
                                        <span>Status</span>
                                        <strong>{titleCase(record.status)}</strong>
                                    </div>
                                </div>
                                <section className="detail-section">
                                    <header>
                                        <span>Context</span>
                                        <h3>Who did what, where</h3>
                                    </header>
                                    <pre>{valueText(record.context)}</pre>
                                </section>
                                <section className="detail-section">
                                    <header>
                                        <span>Outcome</span>
                                        <h3>Result summary</h3>
                                    </header>
                                    <p>{record.error || record.summary || "No error summary."}</p>
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
