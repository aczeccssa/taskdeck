import { isRecord } from "../../lib/narrow";
import type { BoardCard, BoardTemplateExport, TaskStatus } from "../../domain/models";

export const as = <T,>(value: unknown, fallback: T): T => (value as T) ?? fallback;
export const keyOf = (card: BoardCard): string => `${card.node_id}\u0000${card.session}`;
export const statusClass = (status?: TaskStatus | string | null): string =>
    status === "running"
        ? "running"
        : status === "failed"
          ? "failed"
          : status === "exited"
            ? "exited"
            : status === "paused"
              ? "suspected"
              : "";

export type TemplateMessage = { text: string; kind: "" | "error" | "success" | "warning" };
export const quiet: TemplateMessage = { text: "", kind: "" };

export function decodeExport(value: unknown): BoardTemplateExport {
    if (!isRecord(value)) throw new Error("Invalid template export");
    return value as unknown as BoardTemplateExport;
}
