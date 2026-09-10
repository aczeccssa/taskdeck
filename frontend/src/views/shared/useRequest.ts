import { useEffect, useState } from "react";
import { api, decode } from "./api";

export function useRequest<T>(
    url: string,
    fallback: T,
    map: (value: unknown) => T = (value) => value as T,
): [T, boolean, string, () => void] {
    const [data, setData] = useState(fallback);
    const [busy, setBusy] = useState(true);
    const [error, setError] = useState("");
    const [tick, refresh] = useState(0);
    useEffect(() => {
        let live = true;
        setBusy(true);
        setError("");
        void api
            .request(url, decode)
            .then((r) => {
                if (!live) return;
                if (!r.ok) throw new Error(r.message);
                setData(r.data === undefined ? fallback : map(r.data));
            })
            .catch((e) => live && setError(e instanceof Error ? e.message : "Unable to load data"))
            .finally(() => live && setBusy(false));
        return () => {
            live = false;
        };
    }, [url, tick]);
    return [data, busy, error, () => refresh((v) => v + 1)];
}
