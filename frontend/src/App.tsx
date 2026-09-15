import {type FormEvent, useEffect, useState} from "react";
import {House3} from "reicon-react";
import {BrowserRouter, Navigate, Route, Routes, useLocation, useNavigate} from "react-router-dom";
import {AppShell} from "./shell/AppShell";
import {routes, type View} from "./app/routes";
import {UI_ICON_STROKE_WIDTH} from "./components/ReiconNavigation";
import { TasksView } from "./views/tasks/TasksView";
import { DashboardView } from "./views/dashboard/DashboardView";
import { BoardsView } from "./views/boards/BoardsView";
import { WorkflowsView } from "./views/workflows/WorkflowsView";
import { SettingsView } from "./views/settings/SettingsView";
import { DocsView } from "./views/docs/DocsView";
import { CallsView } from "./views/calls/CallsView";
import { AuditView } from "./views/audit/AuditView";
import { AlertsView } from "./views/alerts/AlertsView";
import {SelectionProvider} from "./shell/SelectionContext";
import "./legacy/styles.css";

type AuthState = "loading" | "ready" | "login";

export function App(): React.JSX.Element {
    const [state, setState] = useState<AuthState>("loading");
    useEffect(() => {
        void fetch("/me", {credentials: "same-origin"})
            .then(async (response) => response.json() as Promise<{data?: {enabled?: boolean; authenticated?: boolean}}>)
            .then((response) => setState(response.data?.enabled && !response.data.authenticated ? "login" : "ready"))
            .catch(() => setState("ready"));
    }, []);
    if (state === "loading") return <main className="auth-card" aria-busy="true">Loading Taskdeck…</main>;
    if (state === "login") return <Login onAuthenticated={() => setState("ready")}/>;
    return <BrowserRouter><SelectionProvider><Routes><Route path="/login" element={<Navigate to="/" replace/>}/><Route element={<ReactLayout/>}>{routes.map(({path, view}) => <Route key={path} path={path} element={<RouteView view={view}/>}/>)}</Route></Routes></SelectionProvider></BrowserRouter>;
}

function Login({onAuthenticated}: {onAuthenticated: () => void}): React.JSX.Element {
    const [accessKey, setAccessKey] = useState("");
    const [error, setError] = useState("");
    const submit = async (event: FormEvent<HTMLFormElement>): Promise<void> => {
        event.preventDefault();
        setError("");
        await fetch("/login", {method: "POST", credentials: "same-origin", headers: {"content-type": "application/x-www-form-urlencoded"}, body: new URLSearchParams({access_key: accessKey})});
        const status = await fetch("/me", {credentials: "same-origin"}).then((response) => response.json() as Promise<{data?: {authenticated?: boolean}}>);
        if (status.data?.authenticated) window.location.assign("/"); else setError("Invalid access key");
    };
    return <main className="auth-card"><House3 className="ui-icon ui-icon--auth" size={28} strokeWidth={UI_ICON_STROKE_WIDTH} weight="Outline" aria-hidden="true"/><h1>Taskdeck</h1><p>Enter your access key to continue.</p><form onSubmit={submit}><label htmlFor="access-key">Access key</label><input id="access-key" type="password" value={accessKey} onChange={(event) => setAccessKey(event.target.value)} autoComplete="current-password" required autoFocus/><button type="submit">Unlock</button></form>{error && <p className="form-error" role="alert">{error}</p>}</main>;
}

function ReactLayout(): React.JSX.Element {
    return <AppShell/>;
}

function RouteView({view}: {view: View}): React.JSX.Element | null {
    const location = useLocation();
    const navigate = useNavigate();
    useEffect(() => {
        const onRouteRequest = (event: Event): void => {
            const requested = event instanceof CustomEvent ? event.detail : undefined;
            const route = routes.find((item) => item.view === requested)?.path;
            if (route && route !== location.pathname) navigate(route);
        };
        window.addEventListener("taskdeck:route-request", onRouteRequest);
        return () => window.removeEventListener("taskdeck:route-request", onRouteRequest);
    }, [location.pathname, navigate]);
    if (view === "tasks") return <TasksView/>;
    if (view === "dashboard") return <DashboardView/>;
    if (view === "boards") return <BoardsView/>;
    if (view === "workflows") return <WorkflowsView/>;
    if (view === "settings") return <SettingsView/>;
    if (view === "docs") return <DocsView/>;
    if (view === "calls") return <CallsView/>;
    if (view === "audit") return <AuditView/>;
    if (view === "alerts") return <AlertsView/>;
    return null;
}
