export type View = "tasks" | "dashboard" | "workflows" | "boards" | "alerts" | "calls" | "audit" | "docs" | "settings";

export const routes: ReadonlyArray<{path: string; view: View}> = [
    {path: "/", view: "tasks"},
    {path: "/dashboard", view: "dashboard"},
    {path: "/workflows", view: "workflows"},
    {path: "/boards", view: "boards"},
    {path: "/alerts", view: "alerts"},
    {path: "/calls", view: "calls"},
    {path: "/audit", view: "audit"},
    {path: "/docs", view: "docs"},
    {path: "/settings", view: "settings"},
];

export function pathForView(view: View): string {
    return routes.find((route) => route.view === view)?.path ?? "/";
}
