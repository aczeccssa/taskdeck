import {type FormEvent, useEffect, useRef, useState} from "react";
import {House3} from "reicon-react";
import {mountReiconNavigation, UI_ICON_STROKE_WIDTH} from "./components/ReiconNavigation";
import {BrowserRouter, Outlet, Route, Routes, useLocation, useNavigate} from "react-router-dom";
import shell from "./legacy/index.html?raw";
import legacySource from "./legacy/app.js?raw";
import {mountReiconBrand} from "./components/ReiconBrand";
import {mountThemeIcon} from "./components/ThemeIcon";
import "./legacy/styles.css";

type AuthState = "loading" | "ready" | "login";

type View = "tasks" | "dashboard" | "workflows" | "boards" | "alerts" | "calls" | "audit" | "docs" | "settings";
const routes: ReadonlyArray<{ path: string; view: View }> = [
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

export function App(): React.JSX.Element {
    const [state, setState] = useState<AuthState>("loading");

    useEffect(() => {
        void fetch("/me", {credentials: "same-origin"})
            .then(async (response) => response.json() as Promise<{
                data?: { enabled?: boolean; authenticated?: boolean }
            }>)
            .then((response) => setState(response.data?.enabled && !response.data.authenticated ? "login" : "ready"))
            .catch(() => setState("ready"));
    }, []);

    if (state === "loading") return <main className="auth-card" aria-busy="true">Loading Taskdeck…</main>;
    if (state === "login") return <Login onAuthenticated={() => setState("ready")}/>;
    return <BrowserRouter><Routes><Route element={<LegacyLayout/>}>{routes.map(({path, view}) => <Route key={path}
                                                                                                        path={path}
                                                                                                        element={
                                                                                                            <RouteView
                                                                                                                view={view}/>}/>)}</Route></Routes></BrowserRouter>;
}

function Login({onAuthenticated}: { onAuthenticated: () => void }): React.JSX.Element {
    const [accessKey, setAccessKey] = useState("");
    const [error, setError] = useState("");
    const submit = async (event: FormEvent<HTMLFormElement>) => {
        event.preventDefault();
        setError("");
        await fetch("/login", {
            method: "POST",
            credentials: "same-origin",
            headers: {"content-type": "application/x-www-form-urlencoded"},
            body: new URLSearchParams({access_key: accessKey})
        });
        const status = await fetch("/me", {credentials: "same-origin"}).then((response) => response.json() as Promise<{
            data?: { authenticated?: boolean }
        }>);
        if (status.data?.authenticated) onAuthenticated();
        else setError("Invalid access key");
    };
    return <main className="auth-card"><House3 className="ui-icon ui-icon--auth" size={28}
                                               strokeWidth={UI_ICON_STROKE_WIDTH} weight="Outline" aria-hidden="true"/>
        <h1>Taskdeck</h1><p>Enter your access key to continue.</p>
        <form onSubmit={submit}><label htmlFor="access-key">Access key</label><input id="access-key" type="password"
                                                                                     value={accessKey}
                                                                                     onChange={(event) => setAccessKey(event.target.value)}
                                                                                     autoComplete="current-password"
                                                                                     required autoFocus/>
            <button type="submit">Unlock</button>
        </form>
        {error && <p className="form-error" role="alert">{error}</p>}</main>;
}

/** Shared application layout. Individual operational views are route children. */
function LegacyLayout(): React.JSX.Element {
    const root = useRef<HTMLDivElement>(null);
    useEffect(() => {
        const script = document.createElement("script");
        script.type = "text/javascript";
        script.text = legacySource;
        document.body.appendChild(script);
        mountReiconBrand(root.current?.querySelector(".brand-mark") ?? null);
        mountThemeIcon(root.current?.querySelector("#theme-icon") ?? null);
        if (root.current) mountReiconNavigation(root.current);
        return () => {
            script.remove();
        };
    }, []);
    return <>
        <div ref={root} dangerouslySetInnerHTML={{__html: shell}}/>
        <Outlet/></>;
}

/** Bridges the route state into the legacy operational renderer until each view is componentized. */
function RouteView({view}: { view: View }): null {
    const location = useLocation();
    const navigate = useNavigate();
    useEffect(() => {
        const onRouteRequest = (event: Event) => {
            const requested = event instanceof CustomEvent ? event.detail : undefined;
            const route = routes.find((item) => item.view === requested)?.path;
            if (route && route !== location.pathname) navigate(route);
        };
        window.addEventListener("taskdeck:route-request", onRouteRequest);
        return () => window.removeEventListener("taskdeck:route-request", onRouteRequest);
    }, [location.pathname, navigate]);
    useEffect(() => {
        window.dispatchEvent(new CustomEvent<View>("taskdeck:navigate", {detail: view}));
    }, [location.pathname, view]);
    return null;
}
