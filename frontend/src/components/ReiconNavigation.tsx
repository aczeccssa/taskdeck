import type {ComponentType} from "react";
import {createRoot} from "react-dom/client";
import {
    Chart3,
    ClipboardText3,
    Code4,
    Diagram,
    DocumentText,
    Kanban,
    Notification2,
    Settings4,
    Task,
} from "reicon-react";

/** One optical weight for every product/navigation glyph. */
export const UI_ICON_STROKE_WIDTH = 1.7;

type ReiconProps = {
    size?: number;
    weight?: "Outline" | "Filled";
    strokeWidth?: number | string;
    className?: string;
    color?: string;
    "aria-hidden"?: boolean;
};

/*
 * Each selected outline has real stroked paths (rather than an expanded filled
 * silhouette), so the shared stroke width is visually meaningful everywhere.
 */
const icons: Record<string, ComponentType<ReiconProps>> = {
    tasks: Task,
    dashboard: Chart3,
    workflows: Diagram,
    boards: Kanban,
    alerts: Notification2,
    calls: Code4,
    audit: ClipboardText3,
    docs: DocumentText,
    settings: Settings4,
};

/** Replaces legacy sidebar SVGs with one Reicon outline family and optical weight. */
export function mountReiconNavigation(root: ParentNode): void {
    root.querySelectorAll<HTMLButtonElement>(".nav-button[data-view]").forEach((button) => {
        const Icon = icons[button.dataset.view ?? ""];
        const oldIcon = button.querySelector(":scope > svg");
        if (!Icon || !oldIcon) return;
        const host = document.createElement("i");
        host.className = "nav-reicon";
        host.setAttribute("aria-hidden", "true");
        oldIcon.replaceWith(host);
        createRoot(host).render(
            <Icon
                aria-hidden
                className="ui-icon ui-icon--navigation"
                size={18}
                strokeWidth={UI_ICON_STROKE_WIDTH}
                weight="Outline"
            />,
        );
    });
}
