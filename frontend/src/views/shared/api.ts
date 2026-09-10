import { LegacyApiAdapter } from "../../compat/api";

export const api = new LegacyApiAdapter();
export const decode = (value: unknown): unknown => value;

export function post(url: string, body?: unknown, method = "POST") {
    return api.request(url, decode, {
        method,
        headers: body ? { "content-type": "application/json" } : undefined,
        body: body ? JSON.stringify(body) : undefined,
    });
}
