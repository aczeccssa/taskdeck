import { describe, expect, test } from "bun:test";
import { callsQueryString } from "./query";

describe("calls query construction", () => {
    test("includes paging, status, and only non-empty filters", () => {
        const query = callsQueryString({ q: "deploy", operation: "logs", status: "error", session: "", task: "", page: 2, pageSize: 50 });
        const params = new URLSearchParams(query);
        expect(params.get("page")).toBe("2");
        expect(params.get("page_size")).toBe("50");
        expect(params.get("status")).toBe("error");
        expect(params.get("q")).toBe("deploy");
        expect(params.get("operation")).toBe("logs");
        expect(params.has("session")).toBe(false);
        expect(params.has("task")).toBe(false);
    });

    test("omits default status filter only when set to all-like empty", () => {
        const params = new URLSearchParams(callsQueryString({ q: "", operation: "", status: "all", session: "", task: "", page: 1, pageSize: 20 }));
        expect(params.get("status")).toBe("all");
        expect(params.get("page")).toBe("1");
    });
});
