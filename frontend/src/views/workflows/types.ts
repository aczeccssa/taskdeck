import type { WorkflowGroupMember } from "../../domain/models";

export type Summary = {
    group_name?: string;
    action?: string;
    success_count?: number;
    failed_count?: number;
    skipped_count?: number;
    results?: Array<{ workspace_display_name?: string; task?: string; status?: string; message?: string }>;
};
export type DraftMember = WorkflowGroupMember;
