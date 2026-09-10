import { useLayoutEffect, useRef } from "react";
import type { LogLine } from "../../domain/models";
import { highlightSegments, logMatches, normalizeMatchIndex } from "./helpers";
import { trashIcon } from "./icons";

const LOG_TAILS = [100, 500, 1000, 5000];

export function LogsPanel({
    lines,
    query,
    matchIndex,
    follow,
    scrollToMatch,
    logContext,
    logGeneration,
    tail,
    onFollowChange,
    onScrolledToMatch,
    onSearchChange,
    onSearchSubmit,
    onTailChange,
}: {
    lines: LogLine[];
    query: string;
    matchIndex: number;
    follow: boolean;
    scrollToMatch: boolean;
    logContext: string;
    logGeneration: number | null;
    tail: number;
    onFollowChange: (follow: boolean) => void;
    onScrolledToMatch: () => void;
    onSearchChange: (value: string) => void;
    onSearchSubmit: (direction: -1 | 1) => void;
    onTailChange: (value: number) => void;
}): React.JSX.Element {
    const containerRef = useRef<HTMLDivElement>(null);
    const countRef = useRef<HTMLSpanElement>(null);
    const matchCountRef = useRef<HTMLSpanElement>(null);
    const previousTop = useRef(0);
    const suppressScroll = useRef(false);
    const trimmed = query.trim();
    const { matches, offsetsByLine } = logMatches(lines, trimmed);
    const normalizedIndex = normalizeMatchIndex(matchIndex, matches.length);
    const currentMatch = matches[normalizedIndex];

    const buildRow = (line: LogLine, lineIndex: number): HTMLDivElement => {
        const offsets = offsetsByLine[lineIndex] ?? [];
        const match = offsets.length > 0;
        const current = match && currentMatch?.lineIndex === lineIndex;
        const row = document.createElement("div");
        row.className = `log-row ${line.stream}${match ? " match" : ""}${current ? " current-match" : ""}`;
        row.dataset.lineIndex = String(lineIndex);
        row.dataset.seq = String(line.seq);
        const number = document.createElement("span");
        number.className = "log-number";
        number.textContent = String(line.seq);
        const text = document.createElement("span");
        text.className = "log-text";
        for (const segment of highlightSegments(line.text, trimmed, current ? currentMatch.occurrenceIndex : -1)) {
            const node = document.createElement(segment.highlighted ? "mark" : "span");
            if (segment.highlighted && segment.current) node.className = "current-hit";
            node.textContent = segment.text;
            text.appendChild(node);
        }
        row.append(number, text);
        return row;
    };

    useLayoutEffect(() => {
        const container = containerRef.current;
        if (!container) return;
        previousTop.current = container.scrollTop;
        const rows = Array.from(container.querySelectorAll<HTMLDivElement>(".log-row"));
        const existingLast = Number(rows.at(-1)?.dataset.seq);
        const firstSeq = lines[0]?.seq;
        const lastSeq = lines.at(-1)?.seq;
        const canPatch =
            !trimmed &&
            container.dataset.logContext === logContext &&
            container.dataset.logGeneration === String(logGeneration ?? "") &&
            container.dataset.search === "" &&
            rows.length > 0 &&
            Number.isFinite(existingLast) &&
            lines.some((line) => line.seq === existingLast);
        let removedHeight = 0;
        if (canPatch) {
            rows.forEach((row) => {
                if (Number(row.dataset.seq) < (firstSeq ?? 0)) {
                    removedHeight += row.getBoundingClientRect().height;
                    row.remove();
                }
            });
            lines.forEach((line, lineIndex) => {
                if (line.seq > existingLast) container.appendChild(buildRow(line, lineIndex));
            });
        } else {
            container.replaceChildren(
                ...(lines.length
                    ? lines.map((line, lineIndex) => buildRow(line, lineIndex))
                    : (() => {
                          const empty = document.createElement("div");
                          empty.className = "log-empty";
                          empty.textContent = "No output yet.";
                          return [empty];
                      })()),
            );
        }
        container.dataset.logContext = logContext;
        container.dataset.logGeneration = String(logGeneration ?? "");
        container.dataset.search = trimmed;
        container.dataset.firstSeq = firstSeq == null ? "" : String(firstSeq);
        container.dataset.lastSeq = lastSeq == null ? "" : String(lastSeq);
        if (countRef.current) countRef.current.textContent = `${lines.length} line${lines.length === 1 ? "" : "s"}`;
        if (matchCountRef.current)
            matchCountRef.current.textContent = matches.length ? `${normalizedIndex + 1} / ${matches.length}` : "0 / 0";
        suppressScroll.current = true;
        if (scrollToMatch && currentMatch) {
            const target = container.querySelector<HTMLDivElement>(`[data-line-index="${currentMatch.lineIndex}"]`);
            target?.scrollIntoView({ block: "center" });
            onScrolledToMatch();
        } else if (follow) {
            container.scrollTop = container.scrollHeight;
        } else {
            container.scrollTop = Math.max(0, previousTop.current - removedHeight);
        }
        window.requestAnimationFrame(() => {
            suppressScroll.current = false;
        });
    });

    return (
        <section className="log-panel" id="log-panel" aria-label="Task logs">
            <div className="log-toolbar">
                <select
                    id="log-tail"
                    aria-label="Maximum log lines"
                    title="Maximum log lines"
                    value={String(tail)}
                    onChange={(event) => onTailChange(Number(event.target.value))}>
                    {LOG_TAILS.map((value) => (
                        <option key={value} value={value}>
                            {value} lines
                        </option>
                    ))}
                </select>
                <div className="log-search">
                    <input
                        id="log-search"
                        type="search"
                        placeholder="Search output"
                        aria-label="Search output"
                        autoComplete="off"
                        value={query}
                        onChange={(event) => onSearchChange(event.target.value)}
                        onKeyDown={(event) => {
                            if (event.key !== "Enter") return;
                            event.preventDefault();
                            onSearchSubmit(event.shiftKey ? -1 : 1);
                        }}
                    />
                    <span id="log-match-count" ref={matchCountRef}>
                        0 / 0
                    </span>
                </div>
                <button
                    className="icon-button"
                    type="button"
                    data-log="previous"
                    aria-label="Previous match"
                    title="Previous match">
                    ↑
                </button>
                <button
                    className="icon-button"
                    type="button"
                    data-log="next"
                    aria-label="Next match"
                    title="Next match">
                    ↓
                </button>
                <button
                    className="icon-button"
                    type="button"
                    data-log="clear-search"
                    aria-label="Clear search"
                    title="Clear search">
                    ×
                </button>
                <button className="icon-button" type="button" data-log="top" aria-label="Go to top" title="Go to top">
                    ⇧
                </button>
                <button
                    className="icon-button"
                    type="button"
                    data-log="bottom"
                    aria-label="Go to bottom"
                    title="Go to bottom">
                    ⇩
                </button>
                <button className="button compact" id="follow-button" type="button" data-log="follow">
                    {follow ? "Unfocus" : "Focus"}
                </button>
                <button
                    className="icon-button"
                    type="button"
                    data-log="fullscreen"
                    aria-label="Full screen logs"
                    title="Full screen logs">
                    ⛶
                </button>
                <button
                    className="icon-button danger-icon"
                    type="button"
                    data-log="clear-history"
                    aria-label="Clear logs and performance history"
                    title="Clear logs and performance history">
                    {trashIcon}
                </button>
                <span className="log-line-count" id="log-line-count" ref={countRef}>
                    0 lines
                </span>
            </div>
            <div
                className="logs"
                id="logs"
                tabIndex={0}
                ref={containerRef}
                onScroll={(event) => {
                    if (suppressScroll.current) return;
                    const container = event.currentTarget;
                    if (container.scrollHeight - container.scrollTop - container.clientHeight >= 8)
                        onFollowChange(false);
                }}
            />
        </section>
    );
}

