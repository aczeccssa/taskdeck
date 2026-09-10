import type {
    EditableTask, LogLine, NodeSummary, ServiceEndpoint, ServiceObservation, SessionConfigSnapshot,
    SessionSnapshot, TaskMetricsSnapshot, TaskSnapshot, TaskStatus, TechnologyProfile, WorkspaceSummary,
} from "../domain/models";
import {BoundaryError, isRecord, type Decoder} from "../lib/narrow";

const statuses: readonly TaskStatus[] = ["idle", "running", "paused", "exited", "failed"];

function assertArray(value: unknown, path = "value"): readonly unknown[] {
    if (!Array.isArray(value)) throw new BoundaryError(`${path} must be an array`);
    return value;
}
function assertString(value: unknown, path = "value"): string {
    if (typeof value !== "string") throw new BoundaryError(`${path} must be a string`);
    return value;
}
function assertNumber(value: unknown, path = "value"): number {
    if (typeof value !== "number" || !Number.isFinite(value)) throw new BoundaryError(`${path} must be a finite number`);
    return value;
}
function assertBoolean(value: unknown, path = "value"): boolean {
    if (typeof value !== "boolean") throw new BoundaryError(`${path} must be a boolean`);
    return value;
}
function record(value: unknown, path = "value"): Record<string, unknown> {
    if (!isRecord(value)) throw new BoundaryError(`${path} must be an object`);
    return value;
}
function optionalNumber(value: unknown, path = "value"): number | null {
    return value == null ? null : assertNumber(value, path);
}
function optionalString(value: unknown, path = "value"): string | null {
    return value == null ? null : assertString(value, path);
}

export const decodeLogLine: Decoder<LogLine> = (value, path = "log") => {
    const item = record(value, path);
    return {seq: assertNumber(item.seq, `${path}.seq`), stream: assertString(item.stream, `${path}.stream`), text: assertString(item.text, `${path}.text`)};
};
export const decodeTechnology: Decoder<TechnologyProfile> = (value, path = "technology") => {
    const item = record(value, path);
    return {
        runtime: optionalString(item.runtime, `${path}.runtime`),
        framework: optionalString(item.framework, `${path}.framework`),
        confidence: ["high", "medium", "low", "unknown"].includes(assertString(item.confidence, `${path}.confidence`))
            ? item.confidence as TechnologyProfile["confidence"] : "unknown",
        evidence: assertArray(item.evidence, `${path}.evidence`).map((entry, index) => assertString(entry, `${path}.evidence[${index}]`)),
    };
};
export const decodeServiceEndpoint: Decoder<ServiceEndpoint> = (value, path = "endpoint") => {
    const item = record(value, path);
    return {
        bind_host: assertString(item.bind_host, `${path}.bind_host`),
        port: assertNumber(item.port, `${path}.port`),
        protocol: assertString(item.protocol, `${path}.protocol`),
        pid: optionalNumber(item.pid, `${path}.pid`),
        source: assertString(item.source, `${path}.source`),
        state: assertString(item.state, `${path}.state`),
    };
};
export const decodeService: Decoder<ServiceObservation> = (value, path = "service") => {
    const item = record(value, path);
    const classification = assertString(item.classification, `${path}.classification`);
    const inspection = assertString(item.inspection, `${path}.inspection`);
    return {
        classification: ["service", "process", "unknown"].includes(classification) ? classification as ServiceObservation["classification"] : "unknown",
        technology: item.technology === undefined || item.technology === null ? {runtime: null, framework: null, confidence: "unknown", evidence: []} : decodeTechnology(item.technology, `${path}.technology`),
        endpoints: assertArray(item.endpoints ?? [], `${path}.endpoints`).map((entry, index) => decodeServiceEndpoint(entry, `${path}.endpoints[${index}]`)),
        inspection: ["listening", "no_listener", "not_running", "unsupported", "pending"].includes(inspection) ? inspection as ServiceObservation["inspection"] : "pending",
    };
};
export const decodeTaskSnapshot: Decoder<TaskSnapshot> = (value, path = "task") => {
    const item = record(value, path);
    const status = assertString(item.status, `${path}.status`);
    if (!statuses.includes(status as TaskStatus)) throw new BoundaryError(`${path}.status is invalid`);
    return {
        label: assertString(item.label, `${path}.label`),
        status: status as TaskStatus,
        pid: optionalNumber(item.pid, `${path}.pid`),
        command: assertString(item.command, `${path}.command`),
        cwd: assertString(item.cwd, `${path}.cwd`),
        auto_start: item.auto_start == null ? undefined : assertBoolean(item.auto_start, `${path}.auto_start`),
        last_exit: optionalString(item.last_exit, `${path}.last_exit`),
        exit_code: optionalNumber(item.exit_code, `${path}.exit_code`),
        logs: item.logs === undefined ? undefined : assertArray(item.logs, `${path}.logs`).map((entry, index) => decodeLogLine(entry, `${path}.logs[${index}]`)),
        run_generation: optionalNumber(item.run_generation, `${path}.run_generation`),
        started_at_ms: optionalNumber(item.started_at_ms, `${path}.started_at_ms`),
        schedule: optionalString(item.schedule, `${path}.schedule`),
        service: item.service === undefined ? undefined : decodeService(item.service, `${path}.service`),
    };
};
export const decodeSnapshot: Decoder<SessionSnapshot> = (value, path = "snapshot") => {
    const item = record(value, path);
    const tasks: Record<string, TaskSnapshot> = {};
    const rawTasks = record(item.tasks, `${path}.tasks`);
    for (const [label, entry] of Object.entries(rawTasks)) tasks[label] = decodeTaskSnapshot(entry, `${path}.tasks.${label}`);
    return {
        name: assertString(item.name, `${path}.name`),
        project: assertString(item.project, `${path}.project`),
        source: assertString(item.source, `${path}.source`),
        alias: optionalString(item.alias, `${path}.alias`),
        tasks,
        task_order: assertArray(item.task_order ?? [], `${path}.task_order`).map((entry, index) => assertString(entry, `${path}.task_order[${index}]`)),
    };
};
export const decodeNodes: Decoder<NodeSummary[]> = (value, path = "nodes") => assertArray(value, path).map((entry, index) => {
    const item = record(entry, `${path}[${index}]`);
    return {
        id: assertString(item.id, `${path}[${index}].id`),
        name: assertString(item.name, `${path}[${index}].name`),
        role: assertString(item.role, `${path}[${index}].role`),
        mode: assertString(item.mode, `${path}[${index}].mode`),
        online: assertBoolean(item.online, `${path}[${index}].online`),
        is_self: assertBoolean(item.is_self, `${path}[${index}].is_self`),
        last_seen_ms: optionalNumber(item.last_seen_ms, `${path}[${index}].last_seen_ms`),
        sessions: assertArray(item.sessions ?? [], `${path}[${index}].sessions`).map((session, sessionIndex) => assertString(session, `${path}[${index}].sessions[${sessionIndex}]`)),
    };
});
export const decodeWorkspaces: Decoder<WorkspaceSummary[]> = (value, path = "workspaces") => assertArray(value, path).map((entry, index) => {
    const item = record(entry, `${path}[${index}]`);
    return {
        session: assertString(item.session, `${path}[${index}].session`),
        alias: optionalString(item.alias, `${path}[${index}].alias`),
        display_name: assertString(item.display_name, `${path}[${index}].display_name`),
        project: assertString(item.project, `${path}[${index}].project`),
    };
});
export const decodeStrings: Decoder<string[]> = (value, path = "strings") => assertArray(value, path).map((entry, index) => assertString(entry, `${path}[${index}]`));
export const decodeLogs: Decoder<{generation: number | null; reset: boolean; lines: LogLine[]}> = (value, path = "logs") => {
    const item = record(value, path);
    return {
        generation: optionalNumber(item.generation, `${path}.generation`),
        reset: item.reset == null ? false : assertBoolean(item.reset, `${path}.reset`),
        lines: assertArray(item.lines ?? [], `${path}.lines`).map((entry, index) => decodeLogLine(entry, `${path}.lines[${index}]`)),
    };
};
function decodeAggregate(value: unknown, path = "aggregate") {
    const item = record(value, path);
    return {cpu_percent: assertNumber(item.cpu_percent, `${path}.cpu_percent`), memory_bytes: assertNumber(item.memory_bytes, `${path}.memory_bytes`), process_count: assertNumber(item.process_count, `${path}.process_count`)};
}
export const decodeMetrics: Decoder<TaskMetricsSnapshot> = (value, path = "metrics") => {
    const item = record(value, path);
    return {
        sample_interval_ms: assertNumber(item.sample_interval_ms, `${path}.sample_interval_ms`),
        window_seconds: assertNumber(item.window_seconds, `${path}.window_seconds`),
        cpu_percent_unit: assertString(item.cpu_percent_unit, `${path}.cpu_percent_unit`),
        running: assertBoolean(item.running, `${path}.running`),
        current: decodeAggregate(item.current, `${path}.current`),
        samples: assertArray(item.samples, `${path}.samples`).map((entry, index) => {
            const sample = record(entry, `${path}.samples[${index}]`);
            return {...decodeAggregate(sample, `${path}.samples[${index}]`), timestamp_ms: assertNumber(sample.timestamp_ms, `${path}.samples[${index}].timestamp_ms`)};
        }),
        processes: assertArray(item.processes ?? [], `${path}.processes`).map((entry, index) => {
            const process = record(entry, `${path}.processes[${index}]`);
            return {...decodeAggregate(process, `${path}.processes[${index}]`), pid: assertNumber(process.pid, `${path}.processes[${index}].pid`), ppid: optionalNumber(process.ppid, `${path}.processes[${index}].ppid`), name: assertString(process.name, `${path}.processes[${index}].name`), status: assertString(process.status, `${path}.processes[${index}].status`), run_time_seconds: assertNumber(process.run_time_seconds, `${path}.processes[${index}].run_time_seconds`)};
        }),
        restart_markers_ms: assertArray(item.restart_markers_ms ?? [], `${path}.restart_markers_ms`).map((entry, index) => assertNumber(entry, `${path}.restart_markers_ms[${index}]`)),
    };
};
function decodeEditableTask(value: unknown, path = "task"): EditableTask {
    const item = record(value, path);
    const env: Record<string, string> = {};
    const rawEnv = record(item.env ?? {}, `${path}.env`);
    for (const [key, entry] of Object.entries(rawEnv)) env[key] = assertString(entry, `${path}.env.${key}`);
    const origin = record(item.origin ?? {imported: false, has_yaml_override: false}, `${path}.origin`);
    return {
        label: assertString(item.label, `${path}.label`),
        command: assertString(item.command, `${path}.command`),
        args: assertArray(item.args ?? [], `${path}.args`).map((entry, index) => assertString(entry, `${path}.args[${index}]`)),
        cwd: assertString(item.cwd, `${path}.cwd`),
        env,
        shell: assertBoolean(item.shell, `${path}.shell`),
        auto_start: assertBoolean(item.auto_start, `${path}.auto_start`),
        stop_timeout_ms: assertNumber(item.stop_timeout_ms, `${path}.stop_timeout_ms`),
        clear_logs_on_restart: assertBoolean(item.clear_logs_on_restart, `${path}.clear_logs_on_restart`),
        schedule: optionalString(item.schedule, `${path}.schedule`),
        origin: {imported: assertBoolean(origin.imported, `${path}.origin.imported`), has_yaml_override: assertBoolean(origin.has_yaml_override, `${path}.origin.has_yaml_override`)},
    };
}
export const decodeConfig: Decoder<SessionConfigSnapshot> = (value, path = "config") => {
    const item = record(value, path);
    const workspaceEnv: Record<string, string> = {};
    for (const [key, entry] of Object.entries(record(item.workspace_env ?? {}, `${path}.workspace_env`))) workspaceEnv[key] = assertString(entry, `${path}.workspace_env.${key}`);
    return {
        session: assertString(item.session, `${path}.session`),
        project: assertString(item.project, `${path}.project`),
        source: assertString(item.source, `${path}.source`),
        revision: assertString(item.revision, `${path}.revision`),
        workspace_env: workspaceEnv,
        tasks: assertArray(item.tasks, `${path}.tasks`).map((entry, index) => decodeEditableTask(entry, `${path}.tasks[${index}]`)),
    };
};
export type ConfigSaveFailure = {kind?: "stale_revision" | "reconciliation_error"; saved?: boolean; current_revision?: string};
export type ConfigSaveResponseData = SessionConfigSnapshot | ConfigSaveFailure;
export function decodeConfigOrFailure(value: unknown, path = "config"): ConfigSaveResponseData {
    if (isRecord(value) && (value.kind === "stale_revision" || value.kind === "reconciliation_error")) return configSaveFailure(value) as ConfigSaveFailure;
    return decodeConfig(value, path);
}
export function configSaveFailure(value: unknown): ConfigSaveFailure | undefined {
    if (!isRecord(value)) return undefined;
    const kind = typeof value.kind === "string" ? value.kind : undefined;
    if (kind !== "stale_revision" && kind !== "reconciliation_error") return undefined;
    return {
        kind,
        saved: typeof value.saved === "boolean" ? value.saved : undefined,
        current_revision: typeof value.current_revision === "string" ? value.current_revision : undefined,
    };
}
