import { useRef } from "react";
import type { BoardTemplate, BoardView } from "../../domain/models";
import type { TemplateMessage } from "./helpers";

export function BoardTemplates({open, boards, templates, name, selected, source, message, onClose, onName, onSource, onSelect, onSave, onApply, onExport, onImport, onDelete}: {
    open: boolean; boards: BoardView[]; templates: BoardTemplate[]; name: string; selected: string | null; source: string;
    message: TemplateMessage; onClose: () => void; onName: (value: string) => void; onSource: (value: string) => void;
    onSelect: (value: string) => void; onSave: () => void; onApply: () => void; onExport: () => void;
    onImport: (file: File) => void; onDelete: () => void;
}): React.JSX.Element | null {
    const file = useRef<HTMLInputElement>(null);
    if (!open) return null;
    return <aside className="workflow-editor board-template-editor" aria-labelledby="template-title">
        <header><div><h2 id="template-title">Board templates</h2><p>Reuse a board layout or exchange it as JSON.</p></div><button className="icon-button editor-close" onClick={onClose} aria-label="Close templates">×</button></header>
        <div className="drawer-body">
            <label className="field"><span>Template name</span><input value={name} onChange={(event) => onName(event.target.value)} placeholder="Operations overview" /></label>
            <label className="field"><span>Source board</span><select value={source} onChange={(event) => onSource(event.target.value)}><option value="">Select a board</option>{boards.map((board) => <option key={board.id} value={board.id}>{board.name}</option>)}</select></label>
            <button className="button" onClick={onSave}>Save source as template</button>
            <div className="template-list">{templates.length ? templates.map((template) => <button className={`template-row${selected === template.id ? " selected" : ""}`} key={template.id} onClick={() => onSelect(template.id)}><strong>{template.name}</strong><span>{template.cards?.length ?? 0} cards</span></button>) : <p className="muted">No templates saved.</p>}</div>
            <div className={`settings-message${message.kind ? ` ${message.kind}` : ""}`} role="status">{message.text}</div>
        </div>
        <footer className="settings-footer"><button className="button" onClick={onApply}>Create board</button><button className="button" onClick={onExport}>Export</button><button className="button" onClick={() => file.current?.click()}>Import</button><input ref={file} hidden type="file" accept="application/json,.json" onChange={(event) => { const next = event.target.files?.[0]; if (next) onImport(next); event.target.value = ""; }} /><button className="button danger" onClick={onDelete}>Delete</button></footer>
    </aside>;
}
