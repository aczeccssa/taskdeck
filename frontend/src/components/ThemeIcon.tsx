import {Computer, Moon, Sun} from "reicon-react";
import {createRoot, type Root} from "react-dom/client";
import {UI_ICON_STROKE_WIDTH} from "./ReiconNavigation";

let themeRoot: Root | undefined;

type ThemeMode = "system" | "light" | "dark";

/** Renders the selected theme mode, rather than an icon for the next action. */
export function mountThemeIcon(target: Element | null): void {
    if (!(target instanceof HTMLElement)) return;
    themeRoot?.unmount();
    themeRoot = createRoot(target);
    const render = (mode: ThemeMode): void => {
        const Icon = mode === "light" ? Sun : mode === "dark" ? Moon : Computer;
        themeRoot?.render(<Icon className="ui-icon ui-icon--theme" size={18} strokeWidth={UI_ICON_STROKE_WIDTH} weight="Outline" aria-hidden />);
    };
    const initial = (localStorage.getItem("taskdeck-theme") as ThemeMode | null) ?? "system";
    render(["system", "light", "dark"].includes(initial) ? initial : "system");
    window.addEventListener("taskdeck:theme-changed", (event) => {
        const mode = (event as CustomEvent<ThemeMode>).detail;
        if (["system", "light", "dark"].includes(mode)) render(mode);
    });
}
