import type {ApiToken, AuditListItem, AuditRecord, BoardTemplate, BoardView, EditableTask, EventRecord, McpCallListItem, McpCallRecord, NodeMetricsView, NodeSettingsView, Notification, NotificationRule, ScalingPolicy, ServiceStatus, SessionConfigSnapshot, SessionSnapshot, TaskDependencyView, TaskLogsSnapshot, TaskMetricsSnapshot, TaskRunRecord, WorkflowGraph, WorkflowGroup, WorkflowGroupActionSummary, WorkflowRevision, WorkflowTargetView, WorkspaceQuota, WorkspaceSummary, NodeSummary, Page} from "./models";
export type View = "tasks" | "dashboard" | "workflows" | "boards" | "alerts" | "calls" | "audit" | "docs" | "settings";
export type Language = "en" | "zh";
export type Theme = "system" | "light" | "dark";
export type WorkspaceMode = "split" | "monitor" | "log";
export interface ListFilters { q: string; operation: string; status: string; session: string; task: string; page: number; pageSize: number }
export interface AuditFilters extends ListFilters { source: string; node: string }
export interface TaskdeckState {
 view: View; nodes: NodeSummary[]; nodesSignature: string; sessions: string[]; sessionsSignature: string;
 snapshot: SessionSnapshot | null; snapshotNode: string | null; currentTask: string | null; renderedTask: string | null;
 tabsSignature: string; headerSignature: string; nodesRequest: number; sessionsRequest: number;
 workspaces: WorkspaceSummary[]; workspacesRequest: number; workflowGroups: WorkflowGroup[]; workflowTargets: WorkflowTargetView[];
 workflowUngrouped: WorkflowTargetView[]; workflowRequest: number; workflowEditingId: string | null; workflowEditorActive: boolean;
 workflowDraftMembers: WorkflowTargetView[]; workflowLastResults: WorkflowGroupActionSummary | null;
 boards: BoardView[]; boardTargets: WorkflowTargetView[]; boardRequest: number; boardsSignature: string; boardEditingId: string | null;
 boardEditorActive: boolean; boardDraftCards: BoardView["cards"]; boardSnapshots: Record<string, SessionSnapshot>;
 boardCardData: Record<string, TaskLogsSnapshot | TaskMetricsSnapshot>; boardLiveBusy: boolean;
 nodeSettings: NodeSettingsView | null; nodeSettingsRequest: number; serviceStatus: ServiceStatus | null;
 snapshotRequest: number; metricsRequest: number; logsRequest: number; callsRequest: number; callDetailRequest: number;
 callDetailId: string | null; callDetailMode: "result" | "raw"; configRequest: number; tail: 100 | 500 | 1000 | 5000;
 logLines: TaskLogsSnapshot["lines"]; logContext: string; logGeneration: number | null; lastLogSeq: number | null;
 follow: boolean; search: string; matchIndex: number; scrollToMatch: boolean; suppressScroll: boolean; workspaceMode: WorkspaceMode;
 splitPosition: number; splitSnapping: boolean; metrics: TaskMetricsSnapshot | null; calls: McpCallListItem[]; callsSignature: string;
 callPage: Page; callFilters: ListFilters; callsDebounce: ReturnType<typeof setTimeout> | null; selectedCall?: McpCallRecord | null;
 auditRequest: number; auditDetailRequest: number; auditDetailId: string | null; auditDetailMode: "summary" | "raw";
 audits: AuditListItem[]; auditSignature: string; auditPage: Page; auditFilters: AuditFilters;
 auditDebounce: ReturnType<typeof setTimeout> | null; selectedAudit?: AuditRecord | null;
 config: SessionConfigSnapshot | null; configSession: string | null; configNode: string | null; configTasks: EditableTask[];
 configWorkspaceEnvRows: Array<{key: string; value: string}>; runs: TaskRunRecord[]; runPage: Page;
 runFilters: Omit<ListFilters, "q" | "operation"> & {trigger: string}; events: EventRecord[]; eventPage: Page;
 configSaving: boolean; configDirty: boolean; tabOrderSaving: boolean; suppressTabClick: boolean; seenExits: Record<string, number>;
 toastTimer: ReturnType<typeof setTimeout> | null; nodeMetrics: NodeMetricsView | null; nodeMetricsRequest: number;
 scalingPolicies: ScalingPolicy[]; scalingTargets: WorkflowTargetView[]; scalingRequest: number; scalingEditingId: string | null;
 notifications: Notification[]; notificationsRequest: number; notificationsSignature: string; unreadCount: number;
 notificationRules: NotificationRule[]; ruleEditingId: string | null; orchestratorDraft: WorkflowGraph;
 orchestratorConnectMode: boolean; orchestratorConnectFrom: number | null; orchestratorDrag: {member: number; x: number; y: number} | null;
 workflowRevisions: WorkflowRevision[]; revisionsVisible: boolean; dependencies: TaskDependencyView[];
 dependencyTargets: WorkflowTargetView[]; dependencyRequest: number; boardTemplates: BoardTemplate[]; selectedTemplateId: string | null;
 quotas: WorkspaceQuota[]; quotaSessions: WorkspaceSummary[]; quotaRequest: number; apiTokens: ApiToken[]; apiTokenRequest: number; lang: Language;
}
