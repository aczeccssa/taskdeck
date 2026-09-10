import {expect, test} from "bun:test";
import {readPreferences} from "./storage";

test("storage boundary narrows persisted preferences and ignores malformed data", () => {
 const values: Record<string, string> = {"taskdeck-theme": "invalid", "taskdeck-sidebar-collapsed": "true", "taskdeck-log-tail": "nope", "taskdeck-seen-exits": '{"ok":2,"bad":"2"}'};
 const prefs = readPreferences({getItem: (key) => values[key] ?? null, setItem: () => {}});
 expect(prefs.theme).toBe("system");
 expect(prefs.sidebarCollapsed).toBe(true);
 expect(prefs.logTail).toBe(1000);
 expect(prefs.seenExits).toEqual({ok: 2});
});
