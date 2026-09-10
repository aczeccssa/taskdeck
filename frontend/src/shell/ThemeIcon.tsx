import {useEffect, useState} from "react";
import {Computer, Moon, Sun} from "reicon-react";
import {UI_ICON_STROKE_WIDTH} from "../components/ReiconNavigation";

type ThemeMode = "system" | "light" | "dark";
const isThemeMode = (value: unknown): value is ThemeMode => value === "system" || value === "light" || value === "dark";

export function ThemeIcon(): React.JSX.Element {
    const stored = localStorage.getItem("taskdeck-theme");
    const [mode, setMode] = useState<ThemeMode>(isThemeMode(stored) ? stored : "system");
    useEffect(() => {
        const changed = (event: Event): void => {
            if (event instanceof CustomEvent && isThemeMode(event.detail)) setMode(event.detail);
        };
        window.addEventListener("taskdeck:theme-changed", changed);
        return () => window.removeEventListener("taskdeck:theme-changed", changed);
    }, []);
    const Icon = mode === "light" ? Sun : mode === "dark" ? Moon : Computer;
    return <Icon className="ui-icon ui-icon--theme" size={18} strokeWidth={UI_ICON_STROKE_WIDTH} weight="Outline" aria-hidden/>;
}
