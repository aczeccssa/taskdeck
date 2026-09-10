import type {Language, Theme, View} from "../domain/state";

export const routePaths: Readonly<Record<View, string>> = {
 tasks: "/", dashboard: "/dashboard", workflows: "/workflows", boards: "/boards", alerts: "/alerts",
 calls: "/calls", audit: "/audit", docs: "/docs", settings: "/settings",
};
const views = Object.entries(routePaths) as Array<[View, string]>;
export function viewForPath(pathname: string): View {
 const path = pathname.replace(/\/+$/, "") || "/";
 return views.find(([, route]) => route === path)?.[0] ?? "tasks";
}
export function escapeHtml(value: unknown): string {
 return String(value ?? "").replace(/[&<>"']/g, (character) => ({"&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;"})[character] ?? character);
}
export const escapeAttr = escapeHtml;
export function formatBytes(bytes: number | null | undefined): string {
 if (!Number.isFinite(bytes) || !bytes || bytes < 0) return "0 B";
 const units = ["B", "KB", "MB", "GB", "TB"];
 const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
 return `${(bytes / 1024 ** index).toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}
export function formatRuntime(seconds: number | null | undefined): string {
 if (!Number.isFinite(seconds) || !seconds || seconds < 0) return "0s";
 const total = Math.floor(seconds);
 const hours = Math.floor(total / 3600);
 const minutes = Math.floor((total % 3600) / 60);
 const remaining = total % 60;
 return hours ? `${hours}h ${minutes}m` : minutes ? `${minutes}m ${remaining}s` : `${remaining}s`;
}
export function formatTimestamp(milliseconds: number | null | undefined, language: Language = "en"): string {
 if (!milliseconds) return "";
 return new Date(milliseconds).toLocaleString(language === "zh" ? "zh-CN" : "en");
}
export function paginate(page: number, pageSize: number, total: number): {page: number; pageSize: number; total: number; totalPages: number; hasNext: boolean; hasPrevious: boolean} {
 const safeSize = Number.isInteger(pageSize) && pageSize > 0 ? pageSize : 20;
 const safeTotal = Number.isFinite(total) && total >= 0 ? Math.floor(total) : 0;
 const totalPages = safeTotal === 0 ? 0 : Math.ceil(safeTotal / safeSize);
 const safePage = Number.isInteger(page) && page > 0 ? page : 1;
 return {page: safePage, pageSize: safeSize, total: safeTotal, totalPages, hasNext: safePage < totalPages, hasPrevious: safePage > 1 && totalPages > 0};
}
export function queryString(values: Record<string, string | number | boolean | null | undefined>): string {
 const query = new URLSearchParams();
 for (const [key, value] of Object.entries(values)) if (value !== undefined && value !== null && value !== "") query.set(key, String(value));
 return query.toString();
}
export function preferenceKey(name: "theme" | "lang" | "sidebar-collapsed" | "alert-rules-collapsed" | "log-tail" | "worker-mode" | "seen-exits"): `taskdeck-${string}` { return `taskdeck-${name}`; }
export function parseStoredBoolean(value: string | null, fallback: boolean): boolean { return value === null ? fallback : value === "true"; }
export function parseStoredTheme(value: string | null): Theme { return value === "light" || value === "dark" || value === "system" ? value : "system"; }
export function parseStoredLanguage(value: string | null): Language { return value === "zh" ? "zh" : "en"; }
export function parseStoredJson<T>(value: string | null, decode: (value: unknown) => T, fallback: T): T { if (value === null) return fallback; try { return decode(JSON.parse(value)); } catch { return fallback; } }

export type MessageTable = Readonly<Record<string, string>>;
export function translate(key: string, language: Language, tables: Readonly<{en: MessageTable; zh?: MessageTable}>, fallback?: string): string {
 const localized = language === "zh" ? tables.zh?.[key] : undefined;
 return localized ?? tables.en[key] ?? fallback ?? key;
}

export function reorder<T>(items: readonly T[], from: number, to: number): T[] {
    if (to < 0 || to >= items.length) return items as T[];
    const next = [...items];
    const [item] = next.splice(from, 1);
    next.splice(to, 0, item);
    return next;
}
