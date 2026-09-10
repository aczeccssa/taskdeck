import {describe, expect, test} from "bun:test";
import {
    chartGeometry, highlightSegments, logMatches, matchOffsets, normalizeMatchIndex,
    orderedTaskLabels, reconcileLogs, taskStateDot, taskStatusAllowsAction,
    validateConfigTasks, validateWorkspaceEnv,
} from "./helpers";
import type {LogLine, TaskSnapshot} from "../../domain/models";

const task = (status: TaskSnapshot["status"], generation = 0): TaskSnapshot => ({label: "task", status, command: "echo", cwd: ".", run_generation: generation, service: null});
const line = (seq: number, text: string): LogLine => ({seq, stream: "stdout", text});

describe("task ordering", () => {
    test("uses explicit order and appends missing tasks once", () => {
        const snapshot = {task_order: ["web", "worker", "ghost"], tasks: {worker: task("idle"), web: task("idle"), api: task("idle")} as Record<string, TaskSnapshot>};
        expect(orderedTaskLabels(snapshot)).toEqual(["web", "worker", "api"]);
    });
    test("shows unseen exits only", () => {
        expect(taskStateDot("web", task("running", 1), {})).toEqual({className: "running", label: "Running"});
        expect(taskStateDot("web", task("exited", 2), {web: 2})).toBeNull();
        expect(taskStateDot("web", task("exited", 2), {web: 1})).toEqual({className: "exited", label: "Finished"});
    });
    test("preserves the action disabled matrix", () => {
        expect(taskStatusAllowsAction("idle", "start")).toBe(true);
        expect(taskStatusAllowsAction("running", "start")).toBe(false);
        expect(taskStatusAllowsAction("running", "pause")).toBe(true);
        expect(taskStatusAllowsAction("paused", "resume")).toBe(true);
        expect(taskStatusAllowsAction("idle", "stop")).toBe(false);
    });
});

describe("log matching and stale response reconciliation", () => {
    test("finds case-insensitive offsets and segments", () => {
        const text = "Abc abc ABC";
        expect(matchOffsets(text, "abc")).toEqual([0, 4, 8]);
        const segments = highlightSegments(text, "abc", 1);
        expect(segments.map((segment) => segment.text)).toEqual(["Abc", " ", "abc", " ", "ABC"]);
        expect(segments[2].current).toBe(true);
        expect(segments[3].current).toBe(false);
    });
    test("counts matches across lines", () => {
        const {matches} = logMatches([line(1, "one two"), line(2, "two")], "TWO");
        expect(matches).toEqual([{lineIndex: 0, occurrenceIndex: 0}, {lineIndex: 1, occurrenceIndex: 0}]);
    });
    test("normalizes negative indexes and empty totals", () => {
        expect(normalizeMatchIndex(-1, 3)).toBe(2);
        expect(normalizeMatchIndex(1, 0)).toBe(0);
    });
    test("replaces logs on reset or generation change and trims by tail", () => {
        const previous = [line(1, "old")];
        const replacement = reconcileLogs(previous, {generation: 1, reset: true, lines: [line(9, "new")]}, 1, 1, 1);
        expect(replacement.lines).toEqual([line(9, "new")]);
        const generation = reconcileLogs(previous, {generation: 2, reset: false, lines: [line(10, "new")]}, 10, 1, 1);
        expect(generation.staleReset).toBe(true);
        const append = reconcileLogs(previous, {generation: 1, reset: false, lines: [line(2, "new")]}, 2, 1, 1);
        expect(append.lines).toEqual([line(1, "old"), line(2, "new")]);
        const trim = reconcileLogs([line(1, "one"), line(2, "two")], {generation: 1, reset: false, lines: [line(3, "three")]}, 2, 1, 2);
        expect(trim.lines.map((item) => item.seq)).toEqual([2, 3]);
    });
});

describe("configuration validation", () => {
    const task = {label: "web", command: "bun run dev", args: [], cwd: ".", envRows: [{key: "A", value: "1"}], shell: true, auto_start: false, stop_timeout_ms: 3000, clear_logs_on_restart: false, schedule: " * * * * * ", origin: {imported: false, has_yaml_override: false}};
    test("trims values and rejects invalid configuration", () => {
        const [validated] = validateConfigTasks([{...task, _key: "one"}]);
        expect(validated.schedule).toBe("* * * * *");
        expect(() => validateConfigTasks([{...task, _key: "one", command: " "}])).toThrow("web: command is required");
        expect(() => validateConfigTasks([{...task, _key: "one"}, {...task, _key: "two"}])).toThrow("Duplicate task label: web");
        expect(() => validateConfigTasks([{...task, _key: "one", stop_timeout_ms: 0}])).toThrow("web: stop timeout must be 1-300000 ms");
        expect(() => validateWorkspaceEnv([{key: "", value: "1"}])).toThrow("Workspace environment key is required");
    });
    test("rejects duplicate environment keys", () => {
        expect(validateWorkspaceEnv([{key: "A", value: "1"}])).toEqual({A: "1"});
        expect(() => validateWorkspaceEnv([{key: "A", value: "1"}, {key: "A", value: "2"}])).toThrow("Duplicate workspace environment key A");
    });
});

test("chart geometry preserves scale and in-window restart markers", () => {
    const geometry = chartGeometry([{timestamp_ms: 1, cpu_percent: 50, memory_bytes: 10}, {timestamp_ms: 3, cpu_percent: 100, memory_bytes: 20}], "cpu_percent", [1, 2, 4]);
    expect(geometry.points).toBe("0.00,38.00 300.00,6.00");
    expect(geometry.markers).toEqual(["0.00", "150.00"]);
    expect(geometry.max).toBe(100);
});


