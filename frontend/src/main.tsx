import {createRoot} from "react-dom/client";
import {App} from "./App";
import {applyI18n, storedLanguage} from "./lib/i18n";
import {preferenceKey} from "./lib/helpers";

async function bootstrap(): Promise<void> {
    const theme = localStorage.getItem(preferenceKey("theme"));
    if (theme === "light" || theme === "dark") document.documentElement.dataset.theme = theme;
    applyI18n(storedLanguage());
    if (import.meta.env.MODE === "mock") {
        const {installMockApi} = await import("./mocks/browser");
        installMockApi();
    }
    const element = document.getElementById("root");
    if (!element) throw new Error("Taskdeck root element is missing");
    createRoot(element).render(<App/>);
}

void bootstrap();
