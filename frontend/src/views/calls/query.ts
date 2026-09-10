export function callsQueryString(filters: {
    q: string;
    operation: string;
    status: string;
    session: string;
    task: string;
    page: number;
    pageSize: number;
}): string {
    const q = new URLSearchParams({
        page: String(filters.page),
        page_size: String(filters.pageSize),
        status: filters.status,
    });
    for (const [k, v] of Object.entries(filters))
        if (k !== "page" && k !== "pageSize" && k !== "status" && v) q.set(k, String(v));
    return q.toString();
}
