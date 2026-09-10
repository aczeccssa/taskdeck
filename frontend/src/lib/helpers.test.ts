import {expect, test} from "bun:test";
import {escapeHtml, formatBytes, formatRuntime, paginate, parseStoredJson, preferenceKey, queryString, translate, viewForPath} from "./helpers";

test("HTML escaping and route mapping preserve legacy behavior", () => {
 expect(escapeHtml(`<a title='x'>&`)).toBe("&lt;a title=&#39;x&#39;&gt;&amp;");
 expect(viewForPath("/dashboard///")).toBe("dashboard");
 expect(viewForPath("/not-a-route")).toBe("tasks");
});
test("formatting and pagination handle empty and malformed values", () => {
 expect(formatBytes(1024 * 1024)).toBe("1.0 MB");
 expect(formatBytes(null)).toBe("0 B");
 expect(formatRuntime(3661)).toBe("1h 1m");
 expect(paginate(1, 20, 41)).toEqual({page: 1, pageSize: 20, total: 41, totalPages: 3, hasNext: true, hasPrevious: false});
 expect(paginate(0, 0, -1).totalPages).toBe(0);
});
test("query and preference helpers retain stable storage names", () => {
 expect(queryString({page: 1, q: "hello world", empty: "", absent: undefined})).toBe("page=1&q=hello+world");
 expect(preferenceKey("seen-exits")).toBe("taskdeck-seen-exits");
 expect(parseStoredJson('{"count":2}', (value) => (value as {count: number}).count, 0)).toBe(2);
 expect(parseStoredJson("bad", () => { throw new Error("bad"); }, 7)).toBe(7);
});

test("i18n fallback matches legacy lookup order", () => {
 expect(translate("known", "zh", {en: {known: "English"}, zh: {known: "中文"}})).toBe("中文");
 expect(translate("missing", "zh", {en: {}}, "Fallback")).toBe("Fallback");
 expect(translate("missing", "en", {en: {}})).toBe("missing");
});
