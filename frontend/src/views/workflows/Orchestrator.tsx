import { useEffect, useRef, useState } from "react";
import { formatTimestamp } from "../../lib/helpers";
import { normalizedGraph } from "./graph";
import type { WorkflowGraph, WorkflowRevision } from "../../domain/models";
import type { DraftMember, Summary } from "./types";
import { WorkflowResult } from "./WorkflowResult";

export function Orchestrator({
    members,
    graph,
    setGraph,
    active,
    onRun,
    onSave,
    onHistory,
    summary,
    revisions,
    showRevisionsBox,
    onRestore,
}: {
    members: DraftMember[];
    graph: WorkflowGraph;
    setGraph: React.Dispatch<React.SetStateAction<WorkflowGraph>>;
    active: boolean;
    onRun: () => void;
    onSave: () => void;
    onHistory: () => void;
    summary: Summary | null;
    revisions: WorkflowRevision[];
    showRevisionsBox: boolean;
    onRestore: (revision: number) => void;
}): React.JSX.Element {
    const ref = useRef<HTMLDivElement>(null);
    const [connectMode, setConnectMode] = useState(false);
    const [connectFrom, setConnectFrom] = useState<number | null>(null);
    const [drag, setDrag] = useState<{
        index: number;
        x: number;
        y: number;
        sx: number;
        sy: number;
        moved: boolean;
    } | null>(null);
    useEffect(() => {
        const up = () => setDrag(null);
        window.addEventListener("pointerup", up);
        return () => window.removeEventListener("pointerup", up);
    }, []);
    const positions = normalizedGraph(graph, members.length).positions;
    const writePosition = (index: number, x: number, y: number): void =>
        setGraph((g: WorkflowGraph) => {
            const base = normalizedGraph(g, Math.max(g.positions.length, index + 1));
            return {
                positions: base.positions.map((position, i) => (i === index ? { x, y } : position)),
                edges: base.edges,
            };
        });
    const nodeCenter = (index: number): { x: number; y: number } => {
        const node = ref.current?.querySelectorAll<HTMLElement>(".orchestrator-node")[index];
        return node
            ? { x: node.offsetLeft + node.offsetWidth / 2, y: node.offsetTop + node.offsetHeight / 2 }
            : { x: (positions[index]?.x ?? 0) + 80, y: (positions[index]?.y ?? 0) + 28 };
    };
    const line = (edge: { from: number; to: number }, i: number) => {
        const from = nodeCenter(edge.from),
            to = nodeCenter(edge.to);
        return (
            <line
                key={i}
                x1={from.x}
                y1={from.y}
                x2={to.x}
                y2={to.y}
                data-orch-edge={i}
                onClick={() => {
                    if (!connectMode) return;
                    setGraph((g: WorkflowGraph) => ({ ...g, edges: g.edges.filter((_, n) => n !== i) }));
                }}>
                <title>
                    {edge.from} → {edge.to}
                </title>
            </line>
        );
    };
    return (
        <section className="workflow-card orchestrator-card">
            <header>
                <div>
                    <h2>Orchestrator</h2>
                    <p>Build an execution graph: connect task steps across nodes, then run the flow in order.</p>
                </div>
                <div className="settings-actions">
                    <button
                        className={`button ${connectMode ? "active" : ""}`}
                        type="button"
                        onClick={() => {
                            setConnectMode(!connectMode);
                            setConnectFrom(null);
                        }}
                        disabled={!active}>
                        {connectMode ? "Connecting…" : "Connect"}
                    </button>
                    <button className="button" type="button" onClick={onSave} disabled={!active}>
                        Save layout
                    </button>
                    <button className="button" type="button" onClick={onHistory} disabled={!active}>
                        History
                    </button>
                    <button className="button primary" type="button" onClick={onRun} disabled={!active}>
                        Run in order
                    </button>
                </div>
            </header>
            <div className={`orchestrator${connectMode ? " connect-mode" : ""}`} ref={ref}>
                <svg className="orchestrator-edges" aria-hidden="true">
                    {graph.edges.map(line)}
                </svg>
                {members.length ? (
                    members.map((member: DraftMember, index: number) => {
                        const p = positions[index];
                        const incomplete = !(member.node_id && member.session && member.task);
                        return (
                            <div
                                className={`orchestrator-node${incomplete ? " incomplete" : ""}${connectFrom === index ? " selected" : ""}`}
                                key={index}
                                style={{ left: p.x, top: p.y }}
                                onPointerDown={(event) => {
                                    if (connectMode) {
                                        if (connectFrom === null) setConnectFrom(index);
                                        else if (connectFrom !== index)
                                            setGraph((g: WorkflowGraph) => ({
                                                positions: g.positions,
                                                edges: g.edges.some((e) => e.from === connectFrom && e.to === index)
                                                    ? g.edges
                                                    : [...g.edges, { from: connectFrom, to: index }],
                                            }));
                                        else setConnectFrom(null);
                                        if (connectFrom !== null && connectFrom !== index) setConnectFrom(null);
                                        return;
                                    }
                                    setDrag({
                                        index,
                                        x: p.x,
                                        y: p.y,
                                        sx: event.clientX,
                                        sy: event.clientY,
                                        moved: false,
                                    });
                                }}
                                onPointerMove={(event) => {
                                    if (!drag || drag.index !== index) return;
                                    const dx = event.clientX - drag.sx,
                                        dy = event.clientY - drag.sy;
                                    if (Math.abs(dx) > 4 || Math.abs(dy) > 4) drag.moved = true;
                                    writePosition(index, Math.max(0, drag.x + dx), Math.max(0, drag.y + dy));
                                }}>
                                <strong>{member.task || "?"}</strong>
                                <span>
                                    {member.node_id} · {member.session}
                                </span>
                            </div>
                        );
                    })
                ) : (
                    <div className="orchestrator-empty muted">Select a workflow group to arrange its members.</div>
                )}
            </div>
            {summary && <WorkflowResult summary={summary} />}{" "}
            {showRevisionsBox ? (
                <div className="workflow-revisions">
                    <h3>History</h3>
                    {revisions.length ? (
                        <div className="revision-list">
                            {revisions.map((revision: WorkflowRevision) => (
                                <div className="revision-row" key={revision.revision}>
                                    <span className="revision-number">#{revision.revision}</span>
                                    <span>{revision.name}</span>
                                    <span className="muted">
                                        {formatTimestamp(revision.created_at_ms)}
                                        {revision.note ? ` · ${revision.note}` : ""}
                                    </span>
                                    <button
                                        className="button compact"
                                        type="button"
                                        onClick={() => onRestore(revision.revision)}>
                                        Restore
                                    </button>
                                </div>
                            ))}
                        </div>
                    ) : (
                        <div className="muted">No revisions recorded yet.</div>
                    )}
                </div>
            ) : null}
        </section>
    );
}
