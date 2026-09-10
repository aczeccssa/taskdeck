export type WorkspaceMode = "log" | "split" | "monitor";
export type SeenExits = Record<string, number>;
export type SortDrag = {
    pointerId: number;
    key: string;
    element: HTMLElement;
    startX: number;
    startY: number;
    active: boolean;
    original: string[];
};
