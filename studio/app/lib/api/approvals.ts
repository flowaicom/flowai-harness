/**
 * Approvals API — workspace-scoped approval inbox over the harness runtime.
 *
 * Backs the Studio approval inbox surface.
 *
 * @module api/approvals
 */

import type { Result } from "~/lib/domain/result";
import { isOk, ok } from "~/lib/domain/result";
import { getFlowAIStudioConfig } from "~/lib/studio-config/flowai-config";
import type { ApiError } from "./client";
import { get, getApiConfig, post } from "./client";

export type ApprovalOutcome = "approve" | "reject" | "revise";

/** A captured approval reference and its current status. */
export interface ApprovalRef {
  readonly approvalId: string;
  readonly threadId: string;
  readonly runId: string;
  readonly status: string;
  readonly payload: Record<string, unknown>;
  readonly createdAt: string;
  readonly updatedAt: string;
}

export interface RespondToApprovalInput {
  readonly feedback?: string;
  readonly partial?: unknown;
}

function activeWorkspaceKey(): string {
  const header = getApiConfig().headers["X-Workspace-Id"];
  return header || getFlowAIStudioConfig().defaultWorkspaceKey;
}

function workspacePath(...segments: readonly string[]): string {
  const encoded = [activeWorkspaceKey(), ...segments].map((segment) => encodeURIComponent(segment));
  return `/workspaces/${encoded.join("/")}`;
}

/** List approvals (pending first) for the active workspace. */
export const listApprovals = (): Promise<Result<ApprovalRef[], ApiError>> =>
  get<{ approvals: ApprovalRef[] }>(workspacePath("approvals")).then((result) =>
    isOk(result) ? ok(result.value.approvals) : result
  );

/** Get a single approval. */
export const getApproval = (approvalId: string): Promise<Result<ApprovalRef, ApiError>> =>
  get<{ approval: ApprovalRef }>(workspacePath("approvals", approvalId)).then((result) =>
    isOk(result) ? ok(result.value.approval) : result
  );

/** Approve / reject / revise an approval; the runtime resumes on success. */
export const respondToApproval = (
  approvalId: string,
  outcome: ApprovalOutcome,
  input: RespondToApprovalInput = {}
): Promise<Result<{ status: string }, ApiError>> =>
  post<{ status: string }>(workspacePath("approvals", approvalId, "respond"), {
    outcome,
    feedback: input.feedback,
    partial: input.partial,
  });
