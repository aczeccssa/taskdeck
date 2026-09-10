import type { BoardCard, BoardCardMode, BoardView, SessionSnapshot } from "../../domain/models";
import { keyOf, statusClass } from "./helpers";

export function BoardPanel({
    board,
    snapshots,
    onEdit,
    onDelete,
    onMode,
    onPin,
    onRemove,
    onAction,
}: {
    board: BoardView;
    snapshots: Record<string, SessionSnapshot>;
    onEdit: () => void;
    onDelete: () => void;
    onMode: (card: BoardCard, mode: BoardCardMode) => void;
    onPin: (card: BoardCard) => void;
    onRemove: (card: BoardCard) => void;
    onAction: (card: BoardCard, action: string) => void;
}): React.JSX.Element {
    return (
        <article className="workflow-card board-panel">
            <header>
                <div>
                    <h2>{board.name}</h2>
                    <p>
                        {board.cards.length} card{board.cards.length === 1 ? "" : "s"}
                    </p>
                </div>
                <div className="workflow-card-actions">
                    <button className="button compact" type="button" onClick={onEdit}>
                        Edit
                    </button>
                    <button className="button compact danger" type="button" onClick={onDelete}>
                        Delete
                    </button>
                </div>
            </header>
            <div className="board-card-grid">
                {board.cards.length ? (
                    board.cards.map((card) => {
                        const status = snapshots[keyOf(card)]?.tasks?.[card.task]?.status ?? "unknown";
                        return (
                            <article className="board-card" key={card.id}>
                                <header className="board-card-header">
                                    <div className="board-card-title">
                                        <span
                                            className={`task-state-dot board-dot ${statusClass(status)}`}
                                            aria-hidden="true"
                                        />
                                        <div>
                                            <strong>{card.task}</strong>
                                            <span>
                                                {card.session} · {card.node_id}
                                            </span>
                                        </div>
                                    </div>
                                    <div className="board-card-modes" role="group" aria-label="Card view">
                                        {(["status", "logs", "metrics"] as BoardCardMode[]).map((mode) => (
                                            <button
                                                type="button"
                                                className={card.mode === mode ? "active" : ""}
                                                key={mode}
                                                onClick={() => onMode(card, mode)}>
                                                {mode === "metrics" ? "Perf" : mode[0].toUpperCase() + mode.slice(1)}
                                            </button>
                                        ))}
                                    </div>
                                </header>
                                <div className="board-card-body">
                                    <div className="board-status-facts">
                                        <div>
                                            <span>Status</span>
                                            <strong className={`status ${statusClass(status)}`}>{status}</strong>
                                        </div>
                                        <div>
                                            <span>PID</span>
                                            <strong>{snapshots[keyOf(card)]?.tasks?.[card.task]?.pid ?? "-"}</strong>
                                        </div>
                                        <div>
                                            <span>Command</span>
                                            <strong>
                                                {snapshots[keyOf(card)]?.tasks?.[card.task]?.command ?? "-"}
                                            </strong>
                                        </div>
                                    </div>
                                </div>
                                <div className="board-card-actions">
                                    <button
                                        className="button compact"
                                        type="button"
                                        onClick={() => onAction(card, "start")}>
                                        start
                                    </button>
                                    <button
                                        className="button compact"
                                        type="button"
                                        onClick={() => onAction(card, "restart")}>
                                        restart
                                    </button>
                                    <button
                                        className="button compact"
                                        type="button"
                                        onClick={() => onAction(card, "stop")}>
                                        stop
                                    </button>
                                    <button
                                        className="icon-button"
                                        type="button"
                                        aria-pressed={card.pinned}
                                        onClick={() => onPin(card)}>
                                        {card.pinned ? "★" : "☆"}
                                    </button>
                                    <button
                                        className="icon-button"
                                        type="button"
                                        aria-label="Remove card"
                                        onClick={() => onRemove(card)}>
                                        ×
                                    </button>
                                </div>
                            </article>
                        );
                    })
                ) : (
                    <div className="muted">No cards yet. Edit the board to add task cards.</div>
                )}
            </div>
        </article>
    );
}
