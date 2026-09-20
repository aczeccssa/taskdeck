import {defineConfig, type ProxyOptions} from "vite";
import react from "@vitejs/plugin-react";

const target = process.env.TASKDECK_API_TARGET ?? "http://127.0.0.1:9837";
const proxy: ProxyOptions = {
    target,
    changeOrigin: true,
    secure: false,
};

export default defineConfig(({mode}) => ({
    plugins: [react()],
    publicDir: "public",
    server: {
        port: 5173,
        allowedHosts: ["windows-company-hp66"],
        proxy: mode === "api" ? {
            "/api": proxy,
            "/mcp": proxy,
            "/me": proxy,
            "/login": proxy,
            "/logout": proxy,
            "/healthz": proxy,
            "/favicon.svg": proxy,
        } : undefined,
    },
    build: {
        outDir: process.env.TASKDECK_FRONTEND_OUT_DIR ?? "dist",
        emptyOutDir: true,
        manifest: true,
        assetsDir: "assets",
        sourcemap: false,
    },
}));
