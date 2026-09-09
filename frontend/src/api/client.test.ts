import {afterEach, expect, test} from "bun:test";
import {ApiClient, ApiError} from "./client";

const originalFetch = globalThis.fetch;
afterEach(() => {
    globalThis.fetch = originalFetch;
});

test("ApiClient preserves the daemon response envelope and same-origin credentials", async () => {
    let path = "";
    let credentials: RequestCredentials | undefined;
    globalThis.fetch = (async (input, init) => {
        path = typeof input === "string" ? input : input.toString();
        credentials = init?.credentials;
        return new Response(JSON.stringify({
            ok: true,
            message: "ok",
            data: {id: "mock"}
        }), {headers: {"content-type": "application/json"}});
    }) as typeof fetch;
    const response = await new ApiClient().get<{ id: string }>("/api/nodes");
    expect(response.data.id).toBe("mock");
    expect(path).toBe("/api/nodes");
    expect(credentials).toBe("same-origin");
});

test("ApiClient raises a typed error for an unsuccessful daemon envelope", async () => {
    globalThis.fetch = (async () => new Response(JSON.stringify({
        ok: false,
        message: "denied",
        data: null
    }), {status: 401, headers: {"content-type": "application/json"}})) as unknown as typeof fetch;
    await expect(new ApiClient().get("/api/nodes")).rejects.toBeInstanceOf(ApiError);
});
