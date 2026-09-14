import { describe, expect, test } from "bun:test";
import { configSaveFailure, decodeMetrics } from "./session";

describe("session config save failure", () => {
    test("classifies configuration save response kinds", () => {
        expect(configSaveFailure({ kind: "stale_revision" })).toEqual({ kind: "stale_revision", saved: undefined, current_revision: undefined });
        expect(configSaveFailure({ kind: "reconciliation_error", saved: true, current_revision: "rev-2" })).toEqual({ kind: "reconciliation_error", saved: true, current_revision: "rev-2" });
        expect(configSaveFailure({ kind: "validation_error" })).toBeUndefined();
        expect(configSaveFailure(null)).toBeUndefined();
    });
});


describe("task metrics decoding", () => {
    test("accepts backend process snapshots without aggregate process_count", () => {
        const metrics = decodeMetrics({
            sample_interval_ms: 1000,
            window_seconds: 600,
            cpu_percent_unit: "100.0 = one fully utilized logical CPU",
            running: true,
            current: {cpu_percent: 1.5, memory_bytes: 4096, process_count: 2},
            samples: [{timestamp_ms: 1234, cpu_percent: 1.5, memory_bytes: 4096, process_count: 2}],
            processes: [{
                pid: 42,
                ppid: 1,
                name: "dotnet",
                cpu_percent: 1.5,
                memory_bytes: 4096,
                status: "run",
                run_time_seconds: 12,
            }],
            restart_markers_ms: [],
        });

        expect(metrics.processes).toEqual([{
            pid: 42,
            ppid: 1,
            name: "dotnet",
            cpu_percent: 1.5,
            memory_bytes: 4096,
            status: "run",
            run_time_seconds: 12,
        }]);
    });
});
