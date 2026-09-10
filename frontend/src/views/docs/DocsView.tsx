export function DocsView(): React.JSX.Element {
    const endpoint = `${location.origin}/mcp`;
    const config = JSON.stringify({ mcpServers: { taskdeck: { type: "http", url: endpoint } } }, null, 2);
    return (
        <section className="view active" data-react-owned="true" id="docs-view">
            <article className="docs">
                <header className="docs-header">
                    <div className="eyebrow">Streamable HTTP</div>
                    <div className="endpoint">
                        <i />
                        <code id="endpoint-label">{endpoint.replace(/^https?:\/\//, "")}</code>
                    </div>
                </header>
                <section className="docs-section">
                    <header>
                        <h2>Client configuration</h2>
                    </header>
                    <CopyBlock label="Copy configuration" value={config} />
                </section>
                <section className="docs-section">
                    <header>
                        <h2>Operations</h2>
                    </header>
                    <div className="operation-list">
                        {[
                            ["sessions", "List registered sessions"],
                            ["status", "Inspect task state"],
                            ["logs", "Read recent output"],
                            ["start", "Start one or all tasks"],
                            ["pause", "Pause a process group"],
                            ["resume", "Resume a process group"],
                            ["restart", "Restart one or all tasks"],
                            ["stop", "Stop one or all tasks"],
                        ].map(([name, desc]) => (
                            <div key={name}>
                                <code>{name}</code>
                                <span>{desc}</span>
                            </div>
                        ))}
                    </div>
                </section>
                <section className="docs-section">
                    <header>
                        <h2>External API</h2>
                    </header>
                    <p className="muted">
                        Create an API token under Settings, then call the REST API with{" "}
                        <code>Authorization: Bearer tdk_…</code>.
                    </p>
                    <CopyBlock
                        label="Copy API examples"
                        value={'curl -H "Authorization: Bearer tdk_..." \\\n  ' + `${location.origin}/api/sessions`}
                    />
                </section>
            </article>
        </section>
    );
}
function CopyBlock({ label, value }: { label: string; value: string }): React.JSX.Element {
    const copy = () => void navigator.clipboard?.writeText(value);
    return (
        <div className="code-block" data-copy>
            <button className="icon-button copy" type="button" aria-label={label} title={label} onClick={copy}>
                <svg viewBox="0 0 24 24" aria-hidden="true">
                    <rect x="8" y="8" width="11" height="11" rx="2" />
                    <path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" />
                </svg>
            </button>
            <code>{value}</code>
        </div>
    );
}
