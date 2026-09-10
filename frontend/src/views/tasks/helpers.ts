import type {LogLine, SessionSnapshot, TaskSnapshot, TaskStatus} from "../../domain/models";

export type TaskStateDot = {className: "running" | "failed" | "exited"; label: string};

export function orderedTaskLabels(snapshot: Pick<SessionSnapshot, "tasks" | "task_order">): string[] {
    const labels: string[] = [];
    (snapshot.task_order ?? []).forEach((label) => {
        if (Object.hasOwn(snapshot.tasks, label) && !labels.includes(label)) labels.push(label);
    });
    Object.keys(snapshot.tasks).forEach((label) => {
        if (!labels.includes(label)) labels.push(label);
    });
    return labels;
}

export function taskStateDot(label: string, task: TaskSnapshot, seenExits: Record<string, number>): TaskStateDot | null {
    if (task.status === "running") return {className: "running", label: "Running"};
    if (task.status === "failed") return {className: "failed", label: "Exited with error"};
    const generation = Number(task.run_generation || 0);
    if (task.status === "exited" && Number(seenExits[label] || 0) !== generation) return {className: "exited", label: "Finished"};
    return null;
}

export type TextSegment = {text: string; highlighted: boolean; current: boolean};

export function matchOffsets(text: string, query: string): number[] {
    const value = String(text ?? "");
    const lower = value.toLocaleLowerCase();
    const needle = query.toLocaleLowerCase();
    if (!needle) return [];
    const offsets: number[] = [];
    let index = lower.indexOf(needle);
    while (index >= 0) {
        offsets.push(index);
        index = lower.indexOf(needle, index + Math.max(needle.length, 1));
    }
    return offsets;
}

export function highlightSegments(text: string, query: string, currentOccurrence: number): TextSegment[] {
    const value = String(text ?? "");
    const needleLength = query.toLocaleLowerCase().length;
    const offsets = matchOffsets(value, query);
    if (!offsets.length) return [{text: value, highlighted: false, current: false}];
    const segments: TextSegment[] = [];
    let cursor = 0;
    offsets.forEach((index, occurrence) => {
        if (cursor < index) segments.push({text: value.slice(cursor, index), highlighted: false, current: false});
        segments.push({text: value.slice(index, index + needleLength), highlighted: true, current: occurrence === currentOccurrence});
        cursor = index + needleLength;
    });
    if (cursor < value.length) segments.push({text: value.slice(cursor), highlighted: false, current: false});
    return segments;
}

export type LogMatch = {lineIndex: number; occurrenceIndex: number};

export function logMatches(lines: readonly LogLine[], query: string): {matches: LogMatch[]; offsetsByLine: number[][]} {
    const matches: LogMatch[] = [];
    const offsetsByLine = lines.map((line, lineIndex) => {
        const offsets = matchOffsets(line.text, query);
        offsets.forEach((_, occurrenceIndex) => matches.push({lineIndex, occurrenceIndex}));
        return offsets;
    });
    return {matches, offsetsByLine};
}

export function normalizeMatchIndex(index: number, total: number): number {
    if (total <= 0) return 0;
    return ((index % total) + total) % total;
}

export type LogsPayload = {generation: number | null; reset: boolean; lines: LogLine[]};

export function reconcileLogs(
    previous: readonly LogLine[],
    payload: LogsPayload,
    tail: number,
    previousGeneration: number | null,
    previousLastSeq: number | null,
): {lines: LogLine[]; generation: number | null; lastSeq: number | null; staleReset: boolean} {
    const generationChanged = previousGeneration != null && payload.generation !== previousGeneration;
    if (payload.reset || generationChanged || previousLastSeq == null) {
        const lines = payload.lines.slice(-tail);
        return {lines, generation: payload.generation, lastSeq: lines.at(-1)?.seq ?? null, staleReset: generationChanged || Boolean(payload.reset)};
    }
    const lines = [...previous, ...payload.lines].slice(-tail);
    return {lines, generation: payload.generation, lastSeq: lines.at(-1)?.seq ?? previousLastSeq, staleReset: false};
}

export type ConfigTaskDraft = {
    _key: string;
    label: string;
    command: string;
    args: string[];
    cwd: string;
    envRows: Array<{key: string; value: string}>;
    shell: boolean;
    auto_start: boolean;
    stop_timeout_ms: number;
    clear_logs_on_restart: boolean;
    schedule: string | null;
    origin: {imported: boolean; has_yaml_override: boolean};
};

export type ConfigValidationError = {message: string};

export function taskToDraft(task: Partial<ConfigTaskDraft> & Pick<ConfigTaskDraft, "label" | "command">): ConfigTaskDraft {
    return {
        _key: task._key ?? globalThis.crypto?.randomUUID?.() ?? `task-${Date.now()}-${Math.random()}`,
        label: task.label,
        command: task.command,
        args: [...(task.args ?? [])],
        cwd: task.cwd || ".",
        envRows: task.envRows ? task.envRows.map((row) => ({...row})) : [],
        shell: Boolean(task.shell),
        auto_start: Boolean(task.auto_start),
        stop_timeout_ms: Number(task.stop_timeout_ms || 3000),
        clear_logs_on_restart: Boolean(task.clear_logs_on_restart),
        schedule: task.schedule ?? null,
        origin: task.origin ?? {imported: false, has_yaml_override: false},
    };
}

export type ValidatedTask = {
    label: string; command: string; args: string[]; cwd: string; env: Record<string, string>;
    shell: boolean; auto_start: boolean; stop_timeout_ms: number; clear_logs_on_restart: boolean; schedule: string | null;
};

export function validateConfigTasks(tasks: readonly ConfigTaskDraft[]): ValidatedTask[] {
    const labels = new Set<string>();
    return tasks.map((task, index) => {
        const label = task.label.trim();
        const command = task.command.trim();
        if (!label) throw new Error(`Task ${index + 1}: label is required`);
        if (labels.has(label)) throw new Error(`Duplicate task label: ${label}`);
        labels.add(label);
        if (!command) throw new Error(`${label}: command is required`);
        const timeout = Number(task.stop_timeout_ms);
        if (!Number.isInteger(timeout) || timeout < 1 || timeout > 300000) throw new Error(`${label}: stop timeout must be 1-300000 ms`);
        const env: Record<string, string> = {};
        task.envRows.forEach((row) => {
            const key = row.key.trim();
            if (!key) throw new Error(`${label}: environment key is required`);
            if (Object.hasOwn(env, key)) throw new Error(`${label}: duplicate environment key ${key}`);
            env[key] = row.value;
        });
        return {
            label, command, args: [...task.args], cwd: task.cwd.trim() || ".", env,
            shell: task.shell, auto_start: task.auto_start, stop_timeout_ms: timeout,
            clear_logs_on_restart: Boolean(task.clear_logs_on_restart),
            schedule: task.schedule ? String(task.schedule).trim() : null,
        };
    });
}

export function validateWorkspaceEnv(rows: ReadonlyArray<{key: string; value: string}>): Record<string, string> {
    const values: Record<string, string> = {};
    rows.forEach((row) => {
        const key = row.key.trim();
        if (!key) throw new Error("Workspace environment key is required");
        if (Object.hasOwn(values, key)) throw new Error(`Duplicate workspace environment key ${key}`);
        values[key] = row.value;
    });
    return values;
}

export function taskStatusAllowsAction(status: TaskStatus, action: "start" | "pause" | "resume" | "stop"): boolean {
    if (action === "start") return ["idle", "exited", "failed"].includes(status);
    if (action === "pause") return status === "running";
    if (action === "resume") return status === "paused";
    return ["running", "paused"].includes(status);
}



export type ChartSample = {timestamp_ms: number; cpu_percent: number; memory_bytes: number; process_count?: number | null};
export function chartGeometry(samples: readonly ChartSample[], key: "cpu_percent" | "memory_bytes", restartMarkers: readonly number[] = []): {points: string; markers: string[]; max: number} {
    const values = samples.map((sample) => Number(sample[key] || 0));
    const max = Math.max(...values, 1);
    const points = values.map((value, index) => {
        const x = values.length <= 1 ? 0 : (index / (values.length - 1)) * 300;
        const y = 70 - (value / max) * 64;
        return `${x.toFixed(2)},${y.toFixed(2)}`;
    });
    const firstTimestamp = Number(samples[0]?.timestamp_ms || 0);
    const lastTimestamp = Number(samples.at(-1)?.timestamp_ms || firstTimestamp);
    const span = Math.max(lastTimestamp - firstTimestamp, 1);
    const markers = restartMarkers.map(Number)
        .filter((timestamp) => timestamp >= firstTimestamp && timestamp <= lastTimestamp)
        .map((timestamp) => (((timestamp - firstTimestamp) / span) * 300).toFixed(2));
    return {points: points.join(" "), markers, max};
}

