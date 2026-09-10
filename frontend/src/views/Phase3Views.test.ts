import {describe, expect, test} from "bun:test";
import {auditQueryString, callsQueryString} from "./Phase3Views";

describe("phase 3 query construction", () => {
    test("calls query includes paging, status, and only non-empty filters", () => {
        const query = callsQueryString({q: "deploy", operation: "logs", status: "error", session: "", task: "", page: 2, pageSize: 50});
        const params = new URLSearchParams(query);
        expect(params.get("page")).toBe("2");
        expect(params.get("page_size")).toBe("50");
        expect(params.get("status")).toBe("error");
        expect(params.get("q")).toBe("deploy");
        expect(params.get("operation")).toBe("logs");
        expect(params.has("session")).toBe(false);
        expect(params.has("task")).toBe(false);
    });

    test("calls query omits default status filter only when set to all-like empty", () => {
        const params = new URLSearchParams(callsQueryString({q: "", operation: "", status: "all", session: "", task: "", page: 1, pageSize: 20}));
        expect(params.get("status")).toBe("all");
        expect(params.get("page")).toBe("1");
    });

    test("audit query drops 'all' sentinel filters but keeps paging", () => {
        const params = new URLSearchParams(auditQueryString({q: "", source: "api", status: "all", node: "n1", session: "", task: "", operation: "", page: 3, pageSize: 100}));
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
