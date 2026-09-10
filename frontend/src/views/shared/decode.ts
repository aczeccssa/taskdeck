import { isRecord } from "../../lib/narrow";
import type { Page } from "../../domain/models";

export const text = (v: unknown, fallback = ""): string => (typeof v === "string" ? v : fallback);
export const num = (v: unknown, fallback = 0): number =>
    typeof v === "number" && Number.isFinite(v) ? v : fallback;
export const bool = (v: unknown): boolean => v === true;

export function objectDecoder<T>(decode: (value: Record<string, unknown>) => T) {
    return (value: unknown): T => {
        if (!isRecord(value)) throw new Error("Invalid server response");
        return decode(value);
    };
}

export function pageDecoder<T>(item: (value: unknown) => T) {
    return objectDecoder<Page & { items: T[] }>((r) => ({
        page: num(r.page, 1),
        page_size: num(r.page_size, 20),
        total: num(r.total),
        total_pages: num(r.total_pages),
        has_next: bool(r.has_next),
        has_previous: bool(r.has_previous),
        items: Array.isArray(r.items) ? r.items.map(item) : [],
    }));
}

export const page = <T,>(data: unknown): T => data as T;
