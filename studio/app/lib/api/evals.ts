/**
 * Eval API — workspace-scoped, harness-owned evals.
 *
 * Maps harness eval rows + `EvalArtifact` + `HarnessEvalEventEnvelope` to the
 * frontend `EvalRun` / `EvalEvent` domain so existing stores/components are
 * unchanged. Features the harness does not own (test-case sets, forks,
 * pause/resume/expand, failure export) return an explicit error and their UI
 * controls are gated off.
 *
 * @module api/evals
 */

import type {
  EvalCapabilities,
  EvalCapabilityMode,
  EvalConfig,
  EvalEvent,
  EvalFork,
  EvalRun,
  EvalRunSummary,
  EvalSummary,
  SampleResult,
  ScorerResult,
  TestCase,
  TestCaseResult,
  TestCaseSet,
} from "~/lib/domain/eval";
import type { Result } from "~/lib/domain/result";
import { err, isOk, map, ok } from "~/lib/domain/result";
import { getFlowAIStudioConfig } from "~/lib/studio-config/flowai-config";
import type { ApiError } from "./client";
import { del, get, getApiConfig, post } from "./client";
import type { JsonSSEHandlers } from "./sse";
import { startJsonSSEStream } from "./sse";

export type { EvalCapabilities, EvalCapabilityMode };

// =============================================================================
// Workspace path + harness wire shapes
// =============================================================================

function activeWorkspaceKey(): string {
  const header = getApiConfig().headers["X-Workspace-Id"];
  return header || getFlowAIStudioConfig().defaultWorkspaceKey;
}

function workspacePath(...segments: readonly string[]): string {
  const encoded = [activeWorkspaceKey(), ...segments].map((segment) => encodeURIComponent(segment));
  return `/workspaces/${encoded.join("/")}`;
}

function evalsPath(...segments: readonly string[]): string {
  return workspacePath("evals", ...segments);
}

interface HarnessEvalRow {
  readonly id: string;
  readonly config: Record<string, unknown>;
  readonly scorerPreset: string | null;
  readonly testCaseIds: readonly string[];
  readonly status: string;
  readonly latestSummary?: Record<string, unknown> | null;
  readonly createdAt: string;
  readonly updatedAt: string;
}

interface HarnessScorerResult {
  readonly scorerName: string;
  readonly score: number;
  readonly details?: Record<string, unknown> | null;
}

interface HarnessSampleArtifact {
  readonly sampleIndex: number;
  readonly passed: boolean;
  readonly aggregateScore: number;
  readonly componentScores: readonly HarnessScorerResult[];
  readonly actualTrajectory: readonly string[];
  readonly durationMs: number;
  readonly tokenUsage?: Record<string, number> | null;
  readonly threadId?: string | null;
  readonly responseText?: string | null;
  readonly finalResponseEval?: Record<string, unknown> | null;
  readonly trace?: {
    readonly traceId: string;
    readonly threadId?: string | null;
    readonly url?: string | null;
    readonly metadata?: Record<string, unknown> | null;
  } | null;
  readonly error?: string | null;
}

interface HarnessTestCaseArtifact {
  readonly testCaseId: string;
  readonly input?: string | null;
  readonly samples: readonly HarnessSampleArtifact[];
  readonly aggregateScore: number;
}

interface HarnessEvalArtifact {
  readonly runId: string;
  readonly summary: Record<string, unknown>;
  readonly testCases: readonly HarnessTestCaseArtifact[];
}

interface HarnessArtifactRow {
  readonly evalId: string;
  readonly runId: string;
  readonly artifact: HarnessEvalArtifact;
  readonly createdAt: string;
}

const ZERO_USAGE = {
  inputTokens: 0,
  outputTokens: 0,
  cachedTokens: 0,
  cacheCreationTokens: 0,
};

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

function scoresFromHarness(scores: readonly HarnessScorerResult[]): ScorerResult[] {
  return scores.map((s) => ({
    scorerName: s.scorerName,
    score: s.score,
    details: s.details ?? undefined,
  }));
}

function sampleFromHarness(sample: HarnessSampleArtifact): SampleResult {
  return {
    sampleIndex: sample.sampleIndex,
    passed: sample.passed,
    aggregateScore: sample.aggregateScore,
    scores: scoresFromHarness(sample.componentScores),
    actualTrajectory: sample.actualTrajectory,
    durationMs: sample.durationMs,
    tokenUsage: { ...ZERO_USAGE, ...(sample.tokenUsage ?? {}) },
    error: sample.error ?? null,
    retryCount: 0,
    threadId: sample.threadId ?? undefined,
    responseText: sample.responseText ?? undefined,
    parsedOutput: sample.finalResponseEval ?? undefined,
    trace: sample.trace
      ? {
          traceId: sample.trace.traceId,
          threadId: sample.trace.threadId ?? null,
          url: sample.trace.url ?? null,
          metadata: sample.trace.metadata ?? {},
        }
      : undefined,
  };
}

function testCaseResultFromHarness(tc: HarnessTestCaseArtifact): TestCaseResult {
  return {
    testCaseId: tc.testCaseId,
    input: tc.input ?? undefined,
    samples: tc.samples.map(sampleFromHarness),
    passAtK: [],
    aggregateScore: tc.aggregateScore,
  };
}

function summaryFromHarness(summary: Record<string, unknown>): EvalSummary {
  const num = (key: string): number =>
    typeof summary[key] === "number" ? (summary[key] as number) : 0;
  return {
    totalTestCases: num("totalTestCases"),
    passed: num("passed"),
    failed: num("failed"),
    skipped: num("skipped"),
    aggregateScore: num("aggregateScore"),
    passAtK: Array.isArray(summary.passAtK) ? (summary.passAtK as EvalSummary["passAtK"]) : [],
    totalDurationMs: num("totalDurationMs"),
    totalUsage: { ...ZERO_USAGE, ...((summary.totalUsage as object) ?? {}) },
  };
}

function evalRunFromRow(row: HarnessEvalRow, artifact?: HarnessArtifactRow): EvalRun {
  const config = row.config as unknown as EvalConfig;
  const results = artifact ? artifact.artifact.testCases.map(testCaseResultFromHarness) : [];
  const status: EvalRun["status"] = artifact
    ? {
        status: "completed",
        summary: summaryFromHarness(artifact.artifact.summary),
      }
    : statusFromRow(row.status);
  return {
    id: row.id,
    activityRunId: artifact?.runId ?? artifact?.artifact.runId ?? null,
    config,
    status,
    results,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  };
}

function statusFromRow(status: string): EvalRun["status"] {
  switch (status) {
    case "running":
      return { status: "running", progress: emptyProgress() };
    case "cancelled":
      return { status: "cancelled" };
    case "failed":
      return { status: "failed", error: "Eval run failed" };
    case "completed":
      return { status: "completed", summary: summaryFromHarness({}) };
    default:
      return { status: "queued" };
  }
}

function emptyProgress() {
  return {
    completedSamples: 0,
    totalSamples: 0,
    completedTestCases: 0,
    totalTestCases: 0,
    currentTestCaseId: null,
    elapsedMs: 0,
    estimatedRemainingMs: null,
    testCaseStates: [],
  };
}

// =============================================================================
// CRUD
// =============================================================================

/** List eval runs (lightweight summaries). */
export async function listEvalRuns(_params?: {
  testCaseId?: string;
}): Promise<Result<EvalRunSummary[], ApiError>> {
  const result = await get<{ evals: HarnessEvalRow[] }>(evalsPath());
  return map(result, (value) =>
    value.evals.map((row) => ({
      id: row.id,
      config: row.config as unknown as EvalConfig,
      // Use the latest artifact's summary for the score (the eval row alone has
      // none), unless the run is currently in progress.
      status:
        row.status !== "running" && row.latestSummary
          ? {
              status: "completed",
              summary: summaryFromHarness(row.latestSummary),
            }
          : statusFromRow(row.status),
      resultCount: row.testCaseIds.length,
      createdAt: row.createdAt,
      updatedAt: row.updatedAt,
    }))
  );
}

/** Get a single eval run, hydrated with its latest artifact's results. */
export async function getEvalRun(id: string): Promise<Result<EvalRun, ApiError>> {
  const result = await get<{
    eval: HarnessEvalRow;
    runs: HarnessArtifactRow[];
  }>(evalsPath(id));
  return map(result, (value) => {
    const latest = value.runs.length > 0 ? value.runs[value.runs.length - 1] : undefined;
    return evalRunFromRow(value.eval, latest);
  });
}

/** Delete an eval run and its artifacts. */
export const deleteEvalRun = (id: string): Promise<Result<void, ApiError>> =>
  del<void>(evalsPath(id));

/** Tier-1 cancel: records cancellation intent and lets the stream disconnect cooperatively. */
export const cancelEval = (id: string): Promise<Result<{ status: string }, ApiError>> =>
  post<{ status: string }>(evalsPath(id, "cancel"), {});

/** Test-case sets are not a harness concept; test cases are first-class. */
export const listTestCaseSets = (): Promise<Result<TestCaseSet[], ApiError>> =>
  Promise.resolve(ok([]));

export const uploadTestCaseSet = (
  _set: Omit<TestCaseSet, "id" | "createdAt"> & {
    id?: string;
    createdAt?: string;
  }
): Promise<Result<TestCaseSet, ApiError>> => harnessUnsupported("Test-case sets");

export async function listEvalCapabilities(): Promise<Result<EvalCapabilities, ApiError>> {
  const result = await get<EvalCapabilities>(workspacePath("eval-capabilities"));
  return map(result, (value) => ({
    workspaceKey: value.workspaceKey,
    modes: value.modes.map((mode): EvalCapabilityMode => ({ ...mode })),
  }));
}

// =============================================================================
// Streaming
// =============================================================================

export type EvalStreamHandlers = JsonSSEHandlers<EvalEvent>;

interface HarnessEvalEnvelope {
  readonly runId: string;
  readonly sequence: number;
  readonly type: string;
  readonly data?: Record<string, unknown>;
}

const isHarnessTerminal = (env: HarnessEvalEnvelope) => {
  if (env.type === "evalCompleted") return {};
  if (env.type === "evalFailed") {
    const error = (env.data?.error as string) ?? "Eval run failed";
    return { error };
  }
  return null;
};

/** Translate a harness eval envelope into zero or more frontend EvalEvents. */
function translateEnvelope(env: HarnessEvalEnvelope, ctx: { current: string | null }): EvalEvent[] {
  const data = env.data ?? {};
  switch (env.type) {
    case "testCaseStarted": {
      const testCaseId = String(data.testCaseId ?? "");
      ctx.current = testCaseId;
      return [{ type: "testCaseStarted", testCaseId, testCaseIndex: 0 }];
    }
    case "sampleCompleted": {
      const sample = data.sample as HarnessSampleArtifact | undefined;
      if (!sample) return [];
      return [
        {
          type: "sampleComplete",
          testCaseId: ctx.current ?? "",
          sample: sampleFromHarness(sample),
        },
      ];
    }
    case "testCaseCompleted": {
      const tc = data.testCase as HarnessTestCaseArtifact | undefined;
      if (!tc) return [];
      return [{ type: "testCaseComplete", result: testCaseResultFromHarness(tc) }];
    }
    case "evalCompleted": {
      const artifact = data.artifact as HarnessEvalArtifact | undefined;
      return [
        {
          type: "completed",
          summary: summaryFromHarness(artifact?.summary ?? {}),
        },
      ];
    }
    case "evalFailed":
      return [{ type: "error", message: String(data.error ?? "Eval run failed") }];
    case "evalCancelled":
      return [{ type: "error", message: String(data.reason ?? "Eval run cancelled") }];
    default:
      return [];
  }
}

function translatingHandlers(handlers: EvalStreamHandlers): JsonSSEHandlers<HarnessEvalEnvelope> {
  const ctx = { current: null as string | null };
  return {
    onEvent: (env) => {
      for (const event of translateEnvelope(env, ctx)) {
        handlers.onEvent(event);
      }
    },
    onError: handlers.onError,
    onComplete: handlers.onComplete,
  };
}

/** Build the harness create body from a frontend EvalConfig. */
export function createEvalCreateBody(config: EvalConfig): {
  config: Record<string, unknown>;
  testCaseIds: string[];
} {
  const configWire: Record<string, unknown> = {
    mode: config.mode,
    targetAgentId: config.targetAgentId ?? null,
    testCaseSetId: config.testCaseSetId,
    samplesPerCase: config.samplesPerCase,
    passThreshold: config.passThreshold,
    concurrency: config.concurrency,
    kValues: [...config.kValues],
    provider: config.provider,
    model: config.model,
    timeoutPerSampleSecs: config.timeoutPerSampleSecs,
    tagsFilter: config.tagsFilter ?? null,
    aggregationStrategy: config.aggregationStrategy,
  };
  if (config.scoreWeights != null) {
    configWire.scoreWeights = config.scoreWeights;
  }
  if (config.scorerConfig != null) {
    configWire.scorerConfig = config.scorerConfig;
  }
  if (config.requestOverrides != null) {
    configWire.requestOverrides = config.requestOverrides;
  }
  return {
    config: configWire,
    testCaseIds: config.testCaseIds ? [...config.testCaseIds] : [],
  };
}

/** Create an eval and stream a run (harness create + GET /stream executes it). */
export async function startEvalStream(
  config: EvalConfig,
  handlers: EvalStreamHandlers
): Promise<Result<{ abort: () => void }, ApiError>> {
  const createResult = await post<{ eval: { id: string } }>(
    evalsPath(),
    createEvalCreateBody(config)
  );
  if (!isOk(createResult)) return createResult;
  const evalId = createResult.value.eval.id;

  handlers.onEvent({ type: "started", runId: evalId, config });

  return startJsonSSEStream(
    "GET",
    evalsPath(evalId, "stream"),
    undefined,
    translatingHandlers(handlers),
    isHarnessTerminal
  );
}

/** Reconnect to / re-run an eval's stream. */
export const connectEvalStream = (
  evalId: string,
  handlers: EvalStreamHandlers
): Promise<Result<{ abort: () => void }, ApiError>> =>
  startJsonSSEStream(
    "GET",
    evalsPath(evalId, "stream"),
    undefined,
    translatingHandlers(handlers),
    isHarnessTerminal
  );

/** Re-run an eval (harness rerun streams a fresh run). */
export const rerunEvalCases = (
  evalId: string,
  _testCaseIds: string[],
  handlers: EvalStreamHandlers
): Promise<Result<{ abort: () => void }, ApiError>> =>
  startJsonSSEStream(
    "GET",
    evalsPath(evalId, "stream"),
    undefined,
    translatingHandlers(handlers),
    isHarnessTerminal
  );

// =============================================================================
// Pause / Resume / Expand — deferred to pause and resume support / eval fork tracking (UI gated off)
// =============================================================================

export const pauseEval = (_id: string): Promise<Result<void, ApiError>> =>
  harnessUnsupported("Pausing evals");

export const resumeEval = (_id: string): Promise<Result<void, ApiError>> =>
  harnessUnsupported("Resuming evals");

export const expandEval = (
  _id: string,
  _testCases: readonly TestCase[]
): Promise<Result<void, ApiError>> => harnessUnsupported("Expanding evals");

// =============================================================================
// Fork Tracking — deferred to eval fork tracking (UI gated off)
// =============================================================================

export const createEvalFork = (
  _evalId: string,
  _testCaseId: string,
  _data: {
    threadId: string;
    parentThreadId?: string;
    forkAtMessageIndex?: number;
    editedContent?: string;
    label?: string;
  }
): Promise<Result<EvalFork, ApiError>> => harnessUnsupported("Eval forks");

export const listEvalForks = (
  _evalId: string,
  _testCaseId: string
): Promise<Result<EvalFork[], ApiError>> => Promise.resolve(ok([]));

export const deleteEvalFork = (
  _evalId: string,
  _testCaseId: string,
  _forkId: string
): Promise<Result<void, ApiError>> => harnessUnsupported("Eval forks");

// =============================================================================
// Run Comparison
// =============================================================================

export interface RunComparisonSummary {
  leftId: string;
  rightId: string;
  testCaseComparisons: TestCaseComparison[];
  scoreDelta: number;
  passRateDelta: number;
  leftAvgScore: number;
  rightAvgScore: number;
  leftPassRate: number;
  rightPassRate: number;
}

export interface TestCaseComparison {
  testCaseId: string;
  leftScore: number | null;
  rightScore: number | null;
  scoreDelta: number;
  leftPass: boolean | null;
  rightPass: boolean | null;
  regression: boolean;
  improvement: boolean;
}

interface HarnessComparison {
  readonly left: {
    runId: string | null;
    summary?: Record<string, unknown> | null;
  };
  readonly right: {
    runId: string | null;
    summary?: Record<string, unknown> | null;
  };
  readonly testCases: readonly {
    testCaseId: string;
    left: number | null;
    right: number | null;
    delta: number;
  }[];
}

/** Compare two eval run artifacts side-by-side. */
export const compareEvalRuns = (
  leftId: string,
  rightId: string
): Promise<Result<RunComparisonSummary, ApiError>> =>
  get<{ comparison: HarnessComparison }>(
    `${evalsPath("compare")}?left=${encodeURIComponent(leftId)}&right=${encodeURIComponent(rightId)}`
  ).then((result) =>
    map(result, ({ comparison }) => {
      const testCaseComparisons: TestCaseComparison[] = comparison.testCases.map((tc) => ({
        testCaseId: tc.testCaseId,
        leftScore: tc.left,
        rightScore: tc.right,
        scoreDelta: tc.delta,
        leftPass: tc.left === null ? null : tc.left >= 1,
        rightPass: tc.right === null ? null : tc.right >= 1,
        regression: tc.delta < 0,
        improvement: tc.delta > 0,
      }));
      const avg = (key: "left" | "right"): number => {
        const vals = comparison.testCases.map((tc) => tc[key] ?? 0);
        return vals.length ? vals.reduce((a, b) => a + b, 0) / vals.length : 0;
      };
      const leftAvgScore = avg("left");
      const rightAvgScore = avg("right");
      return {
        leftId,
        rightId,
        testCaseComparisons,
        scoreDelta: rightAvgScore - leftAvgScore,
        passRateDelta: 0,
        leftAvgScore,
        rightAvgScore,
        leftPassRate: 0,
        rightPassRate: 0,
      };
    })
  );

// =============================================================================
// Export Failures — derive client-side from the artifact (no server endpoint)
// =============================================================================

/** Export failed samples as a JSON Blob, computed from the latest artifact. */
export async function exportEvalFailures(evalId: string): Promise<Result<Blob, ApiError>> {
  const run = await getEvalRun(evalId);
  if (!isOk(run)) return run;
  const failures = run.value.results
    .map((tc) => ({
      testCaseId: tc.testCaseId,
      input: tc.input,
      failedSamples: tc.samples.filter((s) => !s.passed),
    }))
    .filter((entry) => entry.failedSamples.length > 0);
  const blob = new Blob([JSON.stringify({ evalId, failures }, null, 2)], {
    type: "application/json",
  });
  return ok(blob);
}
