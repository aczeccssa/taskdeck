import {parseStoredJson, preferenceKey, parseStoredLanguage, parseStoredTheme, parseStoredBoolean} from "../lib/helpers";

export interface StorageLike { getItem(key: string): string | null; setItem(key: string, value: string): void }
export function readPreferences(storage: StorageLike): {theme: "system" | "light" | "dark"; language: "en" | "zh"; sidebarCollapsed: boolean; alertRulesCollapsed: boolean; logTail: 100 | 500 | 1000 | 5000; workerMode: "split" | "monitor" | "log"; seenExits: Record<string, number>} {
 const tail = Number(storage.getItem(preferenceKey("log-tail")));
 const worker = storage.getItem(preferenceKey("worker-mode"));
 const seen = parseStoredJson(storage.getItem(preferenceKey("seen-exits")), (value) => {
  if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("seen exits must be an object");
  const result: Record<string, number> = {};
  for (const [key, item] of Object.entries(value)) if (typeof item === "number" && Number.isFinite(item)) result[key] = item;
  return result;
 }, {});
 return {
  theme: parseStoredTheme(storage.getItem(preferenceKey("theme"))), language: parseStoredLanguage(storage.getItem(preferenceKey("lang"))),
  sidebarCollapsed: parseStoredBoolean(storage.getItem(preferenceKey("sidebar-collapsed")), false),
  alertRulesCollapsed: parseStoredBoolean(storage.getItem(preferenceKey("alert-rules-collapsed")), true),
  logTail: tail === 100 || tail === 500 || tail === 5000 ? tail : 1000,
  workerMode: worker === "monitor" || worker === "log" ? worker : "split", seenExits: seen,
 };
}
