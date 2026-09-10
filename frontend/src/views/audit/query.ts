export function auditQueryString(filters: {
    q: string;
    source: string;
    status: string;
    node: string;
    session: string;
    task: string;
    operation: string;
    page: number;
    pageSize: number;
}): string {
    const q = new URLSearchParams({ page: String(filters.page), page_size: String(filters.pageSize) });
    for (const [k, v] of Object.entries(filters))
        if (k !== "page" && k !== "pageSize" && v && v !== "all") q.set(k, String(v));
    return q.toString();
}
