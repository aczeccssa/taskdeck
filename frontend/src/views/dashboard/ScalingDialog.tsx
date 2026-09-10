import { type FormEvent, useEffect, useRef, useState } from "react";
import { post } from "../shared/api";

export function ScalingDialog({
    open,
    close,
    onSaved,
}: {
    open: boolean;
    close: () => void;
    onSaved: () => void;
}): React.JSX.Element {
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
            const r = await post("/api/scaling-policies", {
                name,
                enabled: true,
                metric: "cpu_percent",
                scale_out_threshold: 80,
                scale_in_threshold: 20,
                cooldown_seconds: 300,
            });
            if (!r.ok) throw new Error(r.message);
            onSaved();
        } catch (x) {
            setMessage(x instanceof Error ? x.message : "Unable to save policy");
        }
    };
    return (
        <dialog
            className="drawer settings-drawer"
            ref={dialogRef}
            onClose={close}
            onClick={(event) => {
                if (event.target === dialogRef.current) close();
            }}>
            <div className="drawer-shell">
                <header className="drawer-header">
                    <div>
                        <span>Dashboard</span>
                        <h2>Scaling settings</h2>
                    </div>
                    <button className="icon-button" type="button" onClick={close} aria-label="Close scaling settings">
                        ×
                    </button>
                </header>
                <form className="settings-form" onSubmit={submit}>
                    <label className="field">
                        <span>Policy name</span>
                        <input value={name} onChange={(e) => setName(e.target.value)} required />
                    </label>
                    <p className="settings-message" role="status">
                        {message}
                    </p>
                    <button className="button primary" type="submit">
                        Save
                    </button>
                </form>
            </div>
        </dialog>
    );
}
