export function ConfigWorkspaceEnv({
    rows,
    setRows,
    setDirty,
}: {
    rows: Array<{ key: string; value: string }>;
    setRows: (rows: Array<{ key: string; value: string }>) => void;
    setDirty: (dirty: boolean) => void;
}): React.JSX.Element {
    return (
        <div className="config-form-body">
            <fieldset className="field">
                <legend>Workspace environment</legend>
                <div className="repeater" id="workspace-env-rows">
                    {rows.map((row, index) => (
                        <div className="repeater-row env" key={index}>
                            <input
                                data-workspace-key={index}
                                value={row.key}
                                placeholder="NAME"
                                onChange={(event) =>
                                    setRows(
                                        rows.map((item, position) =>
                                            position === index
                                                ? { ...item, key: event.target.value }
                                                : item,
                                        ),
                                    )
                                }
                            />
                            <input
                                data-workspace-value={index}
                                value={row.value}
                                placeholder="Value"
                                onChange={(event) =>
                                    setRows(
                                        rows.map((item, position) =>
                                            position === index
                                                ? { ...item, value: event.target.value }
                                                : item,
                                        ),
                                    )
                                }
                            />
                            <button
                                className="icon-button"
                                type="button"
                                data-remove-workspace={index}
                                aria-label="Remove workspace environment variable"
                                title="Remove"
                                onClick={() => {
                                    setRows(
                                        rows.filter((_, position) => position !== index),
                                    );
                                    setDirty(true);
                                }}>
                                ×
                            </button>
                        </div>
                    ))}
                </div>
                <button
                    className="button compact"
                    type="button"
                    data-add-workspace
                    onClick={() => {
                        setRows([...rows, { key: "", value: "" }]);
                        setDirty(true);
                    }}>
                    Add workspace variable
                </button>
            </fieldset>
        </div>
    );
}
