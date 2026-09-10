import { describe, expect, test } from "bun:test";
import { auditQueryString } from "./query";

describe("audit query construction", () => {
    test("drops 'all' sentinel filters but keeps paging", () => {
        const params = new URLSearchParams(auditQueryString({ q: "", source: "api", status: "all", node: "n1", session: "", task: "", operation: "", page: 3, pageSize: 100 }));
        expect(params.get("source")).toBe("api");
        expect(params.get("node")).toBe("n1");
        expect(params.has("status")).toBe(false);
        expect(params.has("q")).toBe(false);
        expect(params.has("session")).toBe(false);
        expect(params.has("task")).toBe(false);
        expect(params.has("operation")).toBe(false);
        expect(params.get("page")).toBe("3");
        expect(params.get("page_size")).toBe("100");
    });
});
