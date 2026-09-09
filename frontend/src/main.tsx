import {createRoot} from "react-dom/client";
import {App} from "./App";

async function bootstrap(): Promise<void> {
    if (import.meta.env.MODE === "mock") {
        const {installMockApi} = await import("./mocks/browser");
        installMockApi();
    }
    const element = document.getElementById("root");
    if (!element) throw new Error("Taskdeck root element is missing");
    createRoot(element).render(<App/>);
}

void bootstrap();
