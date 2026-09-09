export interface ApiEnvelope<T> {
    ok: boolean;
    message: string;
    data: T;
}

export class ApiError extends Error {
    readonly status: number;

    constructor(message: string, status: number) {
        super(message);
        this.name = "ApiError";
        this.status = status;
    }
}

/** A small, strongly typed same-origin client for React code and future feature modules. */
export class ApiClient {
    async request<T>(path: string, init: RequestInit = {}): Promise<ApiEnvelope<T>> {
        const response = await fetch(path, {
            credentials: "same-origin",
            ...init,
            headers: {
                accept: "application/json",
                ...init.headers,
            },
        });
        const body = await response.json() as ApiEnvelope<T>;
        if (!response.ok || !body.ok) {
            throw new ApiError(body.message || `Request failed (${response.status})`, response.status);
        }
        return body;
    }

    get<T>(path: string): Promise<ApiEnvelope<T>> {
        return this.request<T>(path);
    }

    json<T>(path: string, method: "POST" | "PUT" | "DELETE", body?: unknown): Promise<ApiEnvelope<T>> {
        return this.request<T>(path, {
            method,
            headers: body === undefined ? undefined : {"content-type": "application/json"},
            body: body === undefined ? undefined : JSON.stringify(body),
        });
    }
}

export const api = new ApiClient();
