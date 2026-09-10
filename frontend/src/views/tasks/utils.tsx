import type { ReactNode } from "react";

export function requireData<T>(response: { ok: boolean; message: string; data?: T }): T {
    if (!response.ok) throw new Error(response.message);
    if (response.data === undefined) throw new Error("Response data is missing");
    return response.data;
}

export function decodeAccepted(value: unknown): { accepted?: boolean } {
    return typeof value === "object" &&
        value !== null &&
        !Array.isArray(value) &&
        (value as Record<string, unknown>).accepted === true
        ? { accepted: true }
        : {};
}


export function endpointElement(
    endpoint: { bind_host: string; port: number; protocol: string; source: string; state: string },
    index: number,
): ReactNode {
    const label = `${endpoint.bind_host}:${endpoint.port}`;
    const isLink = endpoint.state === "listening" && ["http", "https"].includes(endpoint.protocol);
    if (isLink)
        return (
            <a
                className="endpoint-chip listening"
                key={`${label}-${index}`}
                href={`${endpoint.protocol}://${endpoint.bind_host}:${endpoint.port}`}
                target="_blank"
                rel="noreferrer">
                {label}
            </a>
        );
    return (
        <span
            className={`endpoint-chip ${endpoint.state || ""}`}
            key={`${label}-${index}`}
            title={`${endpoint.source || "unknown"} · ${endpoint.state || "unknown"}`}>
            {label}
        </span>
    );
}

