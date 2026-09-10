/** Wire-level models mirrored from src/protocol.rs. Nullable and optional fields stay explicit. */
export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | {readonly [key: string]: JsonValue};
export type TaskStatus = "idle" | "running" | "paused" | "exited" | "failed";
export type TaskAction = "start" | "stop" | "restart" | "pause" | "resume";

export interface LogLine { seq: number; stream: string; text: string }
export interface TaskLogsSnapshot { generation: number; reset: boolean; lines: LogLine[] }
export type ServiceConfidence = "high" | "medium" | "low" | "unknown";
export interface TechnologyProfile { runtime?: string | null; framework?: string | null; confidence: ServiceConfidence; evidence: string[] }
export interface ServiceEndpoint { bind_host: string; port: number; protocol: string; pid?: number | null; source: string; state: string }
export type ServiceInspectionState = "listening" | "no_listener" | "not_running" | "unsupported" | "pending";
export type ServiceClassification = "service" | "process" | "unknown";
export interface ServiceObservation { classification: ServiceClassification; technology: TechnologyProfile; endpoints: ServiceEndpoint[]; inspection: ServiceInspectionState }
export interface TaskSnapshot { label: string; status: TaskStatus; pid?: number | null; command: string; cwd: string; auto_start?: boolean; last_exit?: string | null; exit_code?: number | null; logs?: LogLine[]; run_generation?: number | null; started_at_ms?: number | null; schedule?: string | null; service?: ServiceObservation | null }
export interface SessionSnapshot { name: string; project: string; source: string; alias?: string | null; tasks: Record<string, TaskSnapshot>; task_order?: string[]; updated_at_ms?: number }
export interface WorkspaceSummary { session: string; alias?: string | null; display_name: string; project: string }
export interface NodeSummary { id: string; name: string; role: string; mode: string; online: boolean; is_self: boolean; last_seen_ms?: number | null; sessions: string[] }

export interface WorkflowGroupMember { node_id: string; session: string; task: string }
export interface WorkflowGraphNodePosition { x: number; y: number }
export interface WorkflowGraphEdge { from: number; to: number }
export interface WorkflowGraph { positions: WorkflowGraphNodePosition[]; edges: WorkflowGraphEdge[] }
export interface WorkflowGroup { id: string; name: string; members: WorkflowGroupMember[]; graph?: WorkflowGraph; created_at_ms?: number; updated_at_ms?: number }
export interface WorkflowTargetView { node_id: string; node_name: string; node_online: boolean; session: string; workspace_alias?: string | null; workspace_display_name: string; project?: string | null; tasks: string[] }
export interface WorkflowGroupsView { groups: WorkflowGroup[]; targets: WorkflowTargetView[]; ungrouped: WorkflowTargetView[] }
export type WorkflowGroupActionStatus = "success" | "failed" | "skipped";
export interface WorkflowGroupActionItem { node_id: string; node_name?: string | null; session: string; workspace_display_name: string; task: string; status: WorkflowGroupActionStatus; message: string }
export interface WorkflowGroupActionSummary { group_id: string; group_name: string; action: TaskAction; results: WorkflowGroupActionItem[]; success_count: number; failed_count: number; skipped_count: number }
export interface WorkflowRevision { group_id: string; revision: number; name: string; members: WorkflowGroupMember[]; graph: WorkflowGraph; note?: string | null; created_at_ms: number }
export interface WorkflowRevisionsView { group_id: string; group_name: string; revisions: WorkflowRevision[] }

export type BoardCardMode = "status" | "logs" | "metrics";
export interface BoardCardInput { node_id: string; session: string; task: string; mode?: BoardCardMode; pinned?: boolean }
export interface BoardCard { id: string; node_id: string; session: string; task: string; mode: BoardCardMode; pinned: boolean }
export interface Board { id: string; name: string; cards: BoardCard[]; created_at_ms?: number; updated_at_ms?: number }
export interface BoardCardView extends BoardCard { node_name?: string | null; status?: TaskStatus | null }
export interface BoardView extends Omit<Board, "cards"> { cards: BoardCardView[] }
export interface BoardsView { boards: BoardView[]; targets: WorkflowTargetView[] }
export interface BoardTemplate { id: string; name: string; description?: string | null; cards: BoardCardInput[]; created_at_ms: number; updated_at_ms: number }
export interface BoardTemplatesView { templates: BoardTemplate[] }
export interface BoardTemplateExport { kind: string; name: string; description?: string | null; cards: BoardCardInput[]; exported_at_ms: number }

export interface TaskDependency { id: string; task_node_id: string; task_session: string; task: string; depends_node_id: string; depends_session: string; depends_task: string; required_state: string }
export interface TaskDependencyView extends TaskDependency { task_status?: TaskStatus | null; depends_status?: TaskStatus | null }
export interface TaskDependenciesView { dependencies: TaskDependencyView[]; targets: WorkflowTargetView[] }

export interface TaskMetricsAggregate { cpu_percent: number; memory_bytes: number; process_count: number }
export interface TaskMetricsSample extends TaskMetricsAggregate { timestamp_ms: number }
export interface TaskProcessSnapshot extends TaskMetricsAggregate { pid: number; ppid?: number | null; name: string; status: string; run_time_seconds: number }
export interface TaskMetricsSnapshot { sample_interval_ms: number; window_seconds: number; cpu_percent_unit: string; running: boolean; current: TaskMetricsAggregate; samples: TaskMetricsSample[]; processes: TaskProcessSnapshot[]; restart_markers_ms: number[] }
export interface NodeMetricsSample { timestamp_ms: number; cpu_percent: number; memory_bytes: number; memory_total_bytes: number; running_tasks: number }
export interface NodeMetricsEntryView { node_id: string; node_name?: string | null; online: boolean; is_self: boolean; current?: NodeMetricsSample | null; samples: NodeMetricsSample[]; session_count: number; task_status_counts: Record<string, number> }
export interface NodeMetricsView { nodes: NodeMetricsEntryView[]; task_status_counts: Record<string, number> }

export type ScalingMetric = "cpu_percent" | "memory_bytes";
export interface ScalingPolicy { id: string; name: string; enabled: boolean; watch_node_id: string; watch_session: string; watch_task: string; metric: ScalingMetric; scale_out_threshold: number; scale_in_threshold: number; scale_out_node_id: string; scale_out_session: string; scale_out_task: string; cooldown_seconds: number; last_action?: string | null; last_action_ms?: number | null; created_at_ms: number; updated_at_ms: number }
export interface ScalingPoliciesView { policies: ScalingPolicy[]; targets: WorkflowTargetView[] }
export interface WorkspaceQuota { id: string; node_id: string; session?: string | null; max_running_tasks: number; created_at_ms: number; updated_at_ms: number }
export interface WorkspaceQuotasView { quotas: WorkspaceQuota[]; sessions: string[] }

export interface NotificationRule { id: string; name: string; event_types: string[]; scope_session?: string | null; scope_task?: string | null; webhook_url?: string | null; enabled: boolean; created_at_ms: number; updated_at_ms: number }
export interface Notification { id: number; node_id: string; rule_id?: string | null; rule_name?: string | null; event_type: string; severity: string; session?: string | null; task?: string | null; title: string; message: string; read: boolean; created_at_ms: number }
export interface NotificationsView { notifications: Notification[]; unread_count: number }
export interface ApiToken { id: string; name: string; token_prefix: string; created_at_ms: number; last_used_at_ms?: number | null }
export interface ApiTokenCreated extends ApiToken { secret: string }
export interface ApiTokensView { tokens: ApiToken[] }

export interface EditableTaskOrigin { imported: boolean; has_yaml_override: boolean }
export interface EditableTask { label: string; command: string; args: string[]; cwd: string; env: Record<string, string>; shell: boolean; auto_start: boolean; stop_timeout_ms: number; clear_logs_on_restart: boolean; schedule?: string | null; origin: EditableTaskOrigin }
export interface SessionConfigSnapshot { session: string; project: string; source: string; revision: string; workspace_env: Record<string, string>; tasks: EditableTask[] }
export interface EnvironmentOverride { field: string; variable: string }
export interface NodeSettingsView { role: string; leader_mode: string; name: string; leader_url?: string | null; bind_host: string; web_port: number; environment_overrides: EnvironmentOverride[]; [setting: string]: JsonValue | EnvironmentOverride[] | undefined }
export interface ServiceStatus { scope?: "user" | "system"; installed?: boolean; running?: boolean; status?: string; message?: string; [field: string]: JsonValue | undefined }

export interface Page { page: number; page_size: number; total: number; total_pages: number; has_next?: boolean; has_previous?: boolean }
export interface TaskRunRecord { id: number; node_id: string; session: string; task: string; trigger: string; status: string; started_at_ms: number; finished_at_ms?: number | null; duration_ms?: number | null; command: string; cwd: string; pid?: number | null; run_generation: number; exit_code?: number | null; error_message?: string | null }
export interface EventRecord { id: number; timestamp_ms: number; category: string; message: string; details: JsonValue }
export interface TaskRunListPage extends Page { items: TaskRunRecord[] }
export interface EventListPage extends Page { items: EventRecord[] }
export interface McpCallListItem { id: number; operation?: string | null; tool: string; target_node?: string | null; input: JsonValue; success: boolean; started_at_ms: number; duration_ms: number }
export interface McpCallRecord extends McpCallListItem { request: JsonValue; response: JsonValue }
export interface McpCallListPage extends Page { items: McpCallListItem[] }
export interface AuditListItem { audit_id: string; timestamp_ms: number; source: string; operation: string; status: string; node_id?: string | null; session?: string | null; task?: string | null; duration_ms?: number | null; replicated_at_ms?: number | null; summary?: string }
export interface AuditRecord extends AuditListItem { context: JsonValue; request?: JsonValue; response?: JsonValue; error?: string | null }
export interface AuditListPage extends Page { items: AuditListItem[] }
