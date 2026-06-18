/**
 * Runs API — workspace-scoped run inspection over the harness RunEventStore.
 *
 * Backs the Studio runs/traces surface.
 *
 * @module api/runs
 */

import type { Result } from "~/lib/domain/result";
import { isOk, ok } from "~/lib/domain/result";
import { getFlowAIStudioConfig } from "~/lib/studio-config/flowai-config";
import type { ApiError } from "./client";
import { get, getApiConfig } from "./client";

/** Summary of a single run, one per `runId`. */
export interface RunSummary {
  readonly runId: string;
  readonly operation: string;
  readonly threadId: string;
  readonly agentId: string;
  readonly status: string;
  readonly firstSeq: number;
  readonly lastSeq: number;
  readonly eventCount: number;
  readonly createdAt: string;
  readonly updatedAt: string;
}

/** A persisted, normalized run event. */
export interface RunEventEnvelope {
  readonly seq: number;
  readonly kind: string;
  readonly event: Record<string, unknown>;
  readonly raw: Record<string, unknown>;
  readonly createdAt: string;
}

export type TracePayload =
  | { readonly kind: "inline"; readonly value: unknown }
  | { readonly kind: "omitted"; readonly reason: string }
  | {
      readonly kind: "redacted";
      readonly redaction: {
        readonly sha256: string;
        readonly originalBytes: number;
        readonly policy: string;
        readonly summary?: string | null;
      };
    };

export interface TraceStep {
  readonly ordinal: number;
  readonly actor?: string | null;
  readonly toolName: string;
  readonly toolCallId?: string | null;
  readonly arguments: TracePayload;
  readonly result?: TracePayload | null;
  readonly startedAt?: string | null;
  readonly completedAt?: string | null;
  readonly error?: string | null;
  readonly correlationId?: string | null;
}

export interface TraceRecord {
  readonly traceId: string;
  readonly workspaceId: string;
  readonly stage: string;
  readonly status: string;
  readonly scope: {
    readonly sessionId?: string | null;
    readonly candidateId?: string | null;
    readonly attemptId?: string | null;
    readonly evalRunId?: string | null;
    readonly testCaseId?: string | null;
    readonly threadId?: string | null;
    readonly sampleIndex?: number | null;
  };
  readonly steps: readonly TraceStep[];
  readonly startedAt?: string | null;
  readonly completedAt?: string | null;
  readonly provenance: unknown;
}

export interface TraceRow {
  readonly traceId: string;
  readonly evalRunId?: string | null;
  readonly testCaseId?: string | null;
  readonly threadId?: string | null;
  readonly sampleIndex?: number | null;
  readonly trace: TraceRecord;
  readonly createdAt: string;
  readonly updatedAt: string;
}

function activeWorkspaceKey(): string {
  const header = getApiConfig().headers["X-Workspace-Id"];
  return header || getFlowAIStudioConfig().defaultWorkspaceKey;
}

function workspacePath(...segments: readonly string[]): string {
  const encoded = [activeWorkspaceKey(), ...segments].map((segment) => encodeURIComponent(segment));
  return `/workspaces/${encoded.join("/")}`;
}

function qs(params: Record<string, string | number | undefined>): string {
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== null && v !== "") p.set(k, String(v));
  }
  const s = p.toString();
  return s ? `?${s}` : "";
}

/** List run summaries for the active workspace. */
export const listRuns = (): Promise<Result<RunSummary[], ApiError>> =>
  get<{ runs: RunSummary[] }>(workspacePath("runs")).then((result) =>
    isOk(result) ? ok(result.value.runs) : result
  );

/** Get a single run summary. */
export const getRun = (runId: string): Promise<Result<RunSummary, ApiError>> =>
  get<{ run: RunSummary }>(workspacePath("runs", runId)).then((result) =>
    isOk(result) ? ok(result.value.run) : result
  );

/** List a run's events, optionally only those after `sinceSeq` (reconnect). */
export const getRunEvents = (
  runId: string,
  sinceSeq?: number
): Promise<Result<RunEventEnvelope[], ApiError>> =>
  get<{ events: RunEventEnvelope[] }>(
    workspacePath("runs", runId, "events") + qs({ since_seq: sinceSeq })
  ).then((result) => (isOk(result) ? ok(result.value.events) : result));

/** List persisted runtime/eval traces for the active workspace. */
export const listTraces = (
  params: { evalRunId?: string; testCaseId?: string; threadId?: string } = {}
): Promise<Result<TraceRow[], ApiError>> =>
  get<{ traces: TraceRow[] }>(
    workspacePath("traces") +
      qs({
        evalRunId: params.evalRunId,
        testCaseId: params.testCaseId,
        threadId: params.threadId,
      })
  ).then((result) => (isOk(result) ? ok(result.value.traces) : result));

/** Get a full persisted trace record. */
export const getTrace = (traceId: string): Promise<Result<TraceRow, ApiError>> =>
  get<{ trace: TraceRow }>(workspacePath("traces", traceId)).then((result) =>
    isOk(result) ? ok(result.value.trace) : result
  );
