/**
 * Tests API — workspace-scoped, harness-owned test cases.
 *
 * Maps the harness `EvalTestCase` wire shape to/from the rich frontend
 * `AuthoredTestCase` so existing stores/components are unchanged. Features the
 * harness does not own (builder sessions, KV index rebuild) return an explicit
 * error; status is not persisted by the harness (test cases are always draft).
 *
 * @module api/tests
 */

import type { TrajectoryMode } from "~/lib/domain/eval";
import type { Result } from "~/lib/domain/result";
import { err, isOk, map, ok } from "~/lib/domain/result";
import type {
  AuthoredTestCase,
  GroundTruth,
  TestCaseBuilderSession,
  TestCaseStatus,
  ToolCallEntry,
  ToolCatalogEntry,
  TraceResponse,
} from "~/lib/domain/test-case";
import { getFlowAIStudioConfig } from "~/lib/studio-config/flowai-config";
import type { ApiError } from "./client";
import { del, get, getApiConfig, post, put } from "./client";

// =============================================================================
// Workspace path + harness wire shapes
// =============================================================================

function activeWorkspaceKey(): string {
  const header = getApiConfig().headers["X-Workspace-Id"];
  return header || getFlowAIStudioConfig().defaultWorkspaceKey;
}

function testsPath(...segments: readonly string[]): string {
  const encoded = [activeWorkspaceKey(), "tests", ...segments].map((segment) =>
    encodeURIComponent(segment)
  );
  return `/workspaces/${encoded.join("/")}`;
}

function makeTestId(): string {
  const random =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID().slice(0, 8)
      : Math.random().toString(36).slice(2, 10);
  return `tc-${random}`;
}

/** Harness `EvalTestCase` wire shape (camelCase). */
interface HarnessTestCase {
  readonly id: string;
  readonly input: string;
  readonly tags?: readonly string[];
  readonly expectedTrajectory?: readonly string[];
  readonly trajectoryMode?: string;
  readonly structuredGroundTruth?: unknown | null;
  readonly finalResponse?: unknown | null;
  readonly sourceThreadId?: string | null;
}

interface HarnessTestRow {
  readonly id: string;
  readonly testCase: HarnessTestCase;
  readonly createdAt: string;
  readonly updatedAt: string;
}

const harnessUnsupported = <T>(feature: string): Promise<Result<T, ApiError>> =>
  Promise.resolve(
    err<ApiError>({
      code: "VALIDATION_ERROR",
      message: `${feature} is not supported by the harness Studio.`,
    })
  );

// =============================================================================
// Wire adapters
// =============================================================================

function fromHarness(row: HarnessTestRow): AuthoredTestCase {
  const tc = row.testCase;
  return {
    id: row.id,
    name: tc.id || row.id,
    description: null,
    input: tc.input ?? "",
    status: "draft",
    expectedTrajectory: tc.expectedTrajectory ?? [],
    trajectoryMode: (tc.trajectoryMode ?? "unordered") as TrajectoryMode,
    groundTruth: null,
    structuredGroundTruth: (tc.structuredGroundTruth ?? null) as GroundTruth | null,
    finalResponse: tc.finalResponse ?? null,
    tags: tc.tags ?? [],
    trajectoryProvenance: [],
    trajectorySources: [],
    sourceThreadId: tc.sourceThreadId ?? null,
    sourceSessionId: null,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  };
}

interface HarnessTestCaseFields {
  readonly input?: string;
  readonly tags?: readonly string[];
  readonly expectedTrajectory?: readonly string[];
  readonly trajectoryMode?: TrajectoryMode;
  readonly structuredGroundTruth?: GroundTruth | null;
  readonly finalResponse?: unknown | null;
  readonly sourceThreadId?: string | null;
}

function toHarness(id: string, fields: HarnessTestCaseFields): Record<string, unknown> {
  const wire: Record<string, unknown> = {
    id,
    input: fields.input ?? "",
    tags: fields.tags ? [...fields.tags] : [],
    expectedTrajectory: fields.expectedTrajectory ? [...fields.expectedTrajectory] : [],
    trajectoryMode: fields.trajectoryMode ?? "unordered",
  };
  if (fields.structuredGroundTruth != null) {
    wire.structuredGroundTruth = fields.structuredGroundTruth;
  }
  if (fields.finalResponse != null) {
    wire.finalResponse = fields.finalResponse;
  }
  if (fields.sourceThreadId != null) {
    wire.sourceThreadId = fields.sourceThreadId;
  }
  return wire;
}

// =============================================================================
// Test Case CRUD
// =============================================================================

export interface ListTestCasesParams {
  status?: string;
  tags?: string;
  sourceThreadId?: string;
}

/** List all test cases (filters are applied client-side). */
export async function listTestCases(
  params?: ListTestCasesParams
): Promise<Result<AuthoredTestCase[], ApiError>> {
  const result = await get<{ tests: HarnessTestRow[] }>(testsPath());
  return map(result, (value) => {
    let cases = value.tests.map(fromHarness);
    if (params?.tags) {
      const wanted = params.tags
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean);
      cases = cases.filter((c) => wanted.every((tag) => c.tags.includes(tag)));
    }
    if (params?.sourceThreadId) {
      cases = cases.filter((c) => c.sourceThreadId === params.sourceThreadId);
    }
    return cases;
  });
}

/** Get a single test case. */
export async function getTestCase(id: string): Promise<Result<AuthoredTestCase, ApiError>> {
  const result = await get<{ test: HarnessTestRow }>(testsPath(id));
  return map(result, (value) => fromHarness(value.test));
}

export type TestCaseUpdatePayload = Partial<
  Pick<
    AuthoredTestCase,
    | "name"
    | "description"
    | "input"
    | "expectedTrajectory"
    | "trajectoryMode"
    | "groundTruth"
    | "tags"
    | "status"
    | "structuredGroundTruth"
    | "finalResponse"
    | "expectedResponse"
  >
>;

/** Create a new test case. The harness has no separate name, so the provided
 * name becomes the id (the title); a random id is generated when blank. */
export async function createTestCase(
  data: Omit<AuthoredTestCase, "id" | "createdAt" | "updatedAt">
): Promise<Result<AuthoredTestCase, ApiError>> {
  const id = data.name && data.name.trim() ? data.name.trim() : makeTestId();
  const result = await post<{ test: HarnessTestRow }>(testsPath(), toHarness(id, data));
  return map(result, (value) => fromHarness(value.test));
}

/** Update a test case (read-merge-write; the harness PUT expects a full case). */
export async function updateTestCase(
  id: string,
  data: TestCaseUpdatePayload
): Promise<Result<AuthoredTestCase, ApiError>> {
  const current = await getTestCase(id);
  if (!isOk(current)) return current;
  const merged: HarnessTestCaseFields = {
    input: data.input ?? current.value.input,
    tags: data.tags ?? current.value.tags,
    expectedTrajectory: data.expectedTrajectory ?? current.value.expectedTrajectory,
    trajectoryMode: data.trajectoryMode ?? current.value.trajectoryMode,
    structuredGroundTruth:
      data.structuredGroundTruth !== undefined
        ? data.structuredGroundTruth
        : current.value.structuredGroundTruth,
    finalResponse:
      data.finalResponse !== undefined ? data.finalResponse : current.value.finalResponse,
    sourceThreadId: current.value.sourceThreadId,
  };
  const result = await put<{ test: HarnessTestRow }>(testsPath(id), toHarness(id, merged));
  return map(result, (value) => fromHarness(value.test));
}

/** Delete a test case. */
export const deleteTestCase = (id: string): Promise<Result<void, ApiError>> =>
  del<void>(testsPath(id));

// =============================================================================
// Builder (Session) — not supported by the harness; use createFromChat instead.
// =============================================================================

export const getTestCaseBuilderSession = (
  _sessionId: string
): Promise<Result<TestCaseBuilderSession, ApiError>> => harnessUnsupported("Test builder sessions");

export const clearTestCaseBuilderSession = (_sessionId: string): Promise<Result<void, ApiError>> =>
  harnessUnsupported("Test builder sessions");

export const saveFromBuilder = (
  _sessionId: string,
  _status?: TestCaseStatus,
  _userPrompt?: string,
  _structuredGroundTruth?: GroundTruth,
  _groundTruth?: string
): Promise<Result<AuthoredTestCase, ApiError>> => harnessUnsupported("Test builder sessions");

// =============================================================================
// Thread Trace Extraction + Chat Harvesting
// =============================================================================

interface HarnessTrace {
  readonly trajectory: readonly string[];
  readonly toolCalls: readonly {
    readonly toolCallId?: string;
    readonly toolName?: string;
    readonly arguments?: unknown;
    readonly result?: unknown;
  }[];
}

/** Extract tool calls from a chat thread's run events. */
export const getThreadTrace = (threadId: string): Promise<Result<TraceResponse, ApiError>> =>
  get<{ threadId: string; trace: HarnessTrace }>(
    testsPath("builder", "threads", threadId, "trace")
  ).then((result) =>
    map(result, (value) => {
      const toolCalls: ToolCallEntry[] = value.trace.toolCalls.map((call, index) => ({
        index,
        toolName: call.toolName ?? "",
        args: call.arguments ?? null,
        result: call.result ?? null,
        invocationId: call.toolCallId ?? `call-${index}`,
        messageIndex: 0,
      }));
      return { threadId: value.threadId, toolCalls, total: toolCalls.length };
    })
  );

/** Auto-extract from a chat thread and create a draft test case. */
export const createFromChat = (threadId: string): Promise<Result<AuthoredTestCase, ApiError>> =>
  post<{ test: HarnessTestRow }>(testsPath("from-chat"), { threadId }).then((result) =>
    map(result, (value) => fromHarness(value.test))
  );

// =============================================================================
// Index + Tool Catalog
// =============================================================================

/** Index rebuild is internal to the harness workspace store. */
export const rebuildTestCaseIndex = (): Promise<Result<{ caseIds: string[] }, ApiError>> =>
  harnessUnsupported("Test index rebuild");

interface HarnessTool {
  readonly name: string;
  readonly agents?: readonly string[];
}

/** Get the agent-scoped tool catalog. */
export const getToolCatalog = (): Promise<Result<ToolCatalogEntry[], ApiError>> =>
  get<{ tools: HarnessTool[] }>(testsPath("tools")).then((result) =>
    map(result, (value) =>
      value.tools.map((tool) => ({
        name: tool.name,
        description:
          tool.agents && tool.agents.length > 0 ? `Used by ${tool.agents.join(", ")}` : "",
        category: "general" as const,
      }))
    )
  );

// =============================================================================
// Validation (client-side pre-eval check)
// =============================================================================

export interface ValidationIssue {
  severity: "error" | "warning" | "info";
  message: string;
}

export interface ValidationResult {
  valid: boolean;
  issues: ValidationIssue[];
}

/** Validate a test case's trajectory/ground truth before running an eval. */
export const validateTestCase = async (id: string): Promise<Result<ValidationResult, ApiError>> => {
  const result = await getTestCase(id);
  if (!isOk(result)) return result;
  const tc = result.value;
  const issues: ValidationIssue[] = [];
  if (!tc.input.trim()) {
    issues.push({ severity: "error", message: "Input prompt is empty." });
  }
  if (tc.expectedTrajectory.length === 0 && !tc.structuredGroundTruth) {
    issues.push({
      severity: "warning",
      message: "No expected trajectory or ground truth to score against.",
    });
  }
  return ok({
    valid: !issues.some((issue) => issue.severity === "error"),
    issues,
  });
};

// =============================================================================
// Batch Operations (client-side fan-out)
// =============================================================================

export interface BatchResult {
  succeeded: number;
  failed: number;
  total: number;
  failedIds: string[];
}

type BatchAction =
  | { action: "updateStatus"; ids: string[]; status: TestCaseStatus }
  | { action: "delete"; ids: string[] };

/** Execute a batch operation across test cases (one request per id). */
export const batchTestCases = async (
  action: BatchAction
): Promise<Result<BatchResult, ApiError>> => {
  let succeeded = 0;
  let failed = 0;
  const failedIds: string[] = [];
  for (const id of action.ids) {
    const succeededOne =
      action.action === "delete"
        ? isOk(await deleteTestCase(id))
        : isOk(await updateTestCase(id, { status: action.status }));
    if (succeededOne) {
      succeeded += 1;
    } else {
      failed += 1;
      failedIds.push(id);
    }
  }
  return ok({ succeeded, failed, total: action.ids.length, failedIds });
};
