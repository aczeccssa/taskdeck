import {expect, test} from "bun:test";
import {LegacyApiAdapter} from "./api";
import {stringValue} from "../lib/narrow";

test("compat adapter returns ok:false envelopes without throwing", async () => {
 const adapter = new LegacyApiAdapter({fetch: async () => new Response(JSON.stringify({ok: false, message: "stale revision", data: "retry"}), {status: 409}), redirectToLogin: () => { throw new Error("unexpected redirect"); }});
 expect(await adapter.request("/api/test", stringValue)).toEqual({ok: false, message: "stale revision", data: "retry"});
});

test("compat adapter preserves the legacy 401 redirect and error", async () => {
 let redirected = false;
 const adapter = new LegacyApiAdapter({fetch: async () => new Response("", {status: 401}), redirectToLogin: () => { redirected = true; }});
 await expect(adapter.request("/api/test", stringValue)).rejects.toThrow("Authentication required");
 expect(redirected).toBe(true);
});

test("compat adapter rejects malformed envelopes at the JSON boundary", async () => {
 const adapter = new LegacyApiAdapter({fetch: async () => new Response(JSON.stringify({ok: "yes", message: 1, data: "bad"})), redirectToLogin: () => {}});
 await expect(adapter.request("/api/test", stringValue)).rejects.toThrow("response.ok must be a boolean");
});
