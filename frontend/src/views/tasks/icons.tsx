import type { ReactNode } from "react";

export function actionIcon(action: "start" | "pause" | "restart" | "stop"): ReactNode {
    const paths: Record<typeof action, string> = {
        start: "m8 5 11 7-11 7z",
        pause: "M8 5v14M16 5v14",
        restart: "M19 8V4m0 0h-4m4 0-3 3a7 7 0 1 0 2 8",
        stop: "",
    };
    if (action === "stop")
        return (
            <svg viewBox="0 0 24 24" aria-hidden="true">
                <rect x="6" y="6" width="12" height="12" rx="1" />
            </svg>
        );
    return (
        <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d={paths[action]} />
        </svg>
    );
}

export const settingsIcon = (
    <svg viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="12" cy="12" r="3" />
        <path d="M19 12a7 7 0 0 0-.1-1l2-1.5-2-3.4-2.4 1A7 7 0 0 0 15 6l-.3-2.6h-4L10.4 6A7 7 0 0 0 9 7.1l-2.4-1-2 3.4 2 1.5a7 7 0 0 0 0 2l-2 1.5 2 3.4 2.4-1A7 7 0 0 0 10.4 18l.3 2.6h4L15 18a7 7 0 0 0 1.5-1.1l2.4 1 2-3.4-2-1.5a7 7 0 0 0 .1-1z" />
    </svg>
);
export const trashIcon = (
    <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M4 7h16M9 7V4h6v3m3 0-1 13H7L6 7m4 4v5m4-5v5" />
    </svg>
);
export const gripIcon = (
    <svg viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="9" cy="7" r="1" />
        <circle cx="15" cy="7" r="1" />
        <circle cx="9" cy="12" r="1" />
        <circle cx="15" cy="12" r="1" />
        <circle cx="9" cy="17" r="1" />
        <circle cx="15" cy="17" r="1" />
    </svg>
);

