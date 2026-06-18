/**
 * Eval domain types — TypeScript mirror of Rust `core::eval`.
 *
 * All types are readonly interfaces with discriminated unions.
 * Algebraic sum types with exhaustive matching.
 *
 * @module domain/eval
 */

import { assertNever } from "~/lib/utils";
import type { DataReadiness } from "./data";
import type { ResponseValidation } from "./message";

// =============================================================================
// Literal Unions
// =============================================================================

/**
 * Evaluation mode — determines which agent role(s) to test.
 *
 * Built-in modes: "single" (one agent), "sequential" (multi-agent pipeline).
 * String-based so users can define custom modes for their agent topology.
 */
export type EvalMode = string;

/** Trajectory matching mode. */
export type TrajectoryMode = "strict" | "unordered" | "subset" | "superset" | "subsequence";

export type ScorerKey = "trajectory" | "planned_actions" | "executed_actions" | "final_response";

export type ScoreWeights = Partial<Record<ScorerKey, number>>;

/** Eval status discriminant (7 variants). */
export type EvalStatusKey =
  | "queued"
  | "running"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled"
  | "skipped";

// =============================================================================
// Status (Discriminated Union)
// =============================================================================

export type EvalStatus =
  | { readonly status: "queued" }
  | { readonly status: "running"; readonly progress: EvalProgress }
  | { readonly status: "paused"; readonly progress: EvalProgress }
  | { readonly status: "completed"; readonly summary: EvalSummary }
  | { readonly status: "failed"; readonly error: string }
  | { readonly status: "cancelled" }
  | { readonly status: "skipped"; readonly reason: string };

// =============================================================================
// Per-Test-Case State Machine (production-quality progress)
// =============================================================================

export type TestCaseState =
  | { readonly state: "queued" }
  | {
      readonly state: "running";
      readonly startedAtMs: number;
      readonly completedSamples: number;
      readonly totalSamples: number;
    }
  | {
      readonly state: "completed";
      readonly durationMs: number;
      readonly passed: boolean;
      readonly aggregateScore: number;
    }
  | { readonly state: "failed"; readonly durationMs: number; readonly error: string };

export interface TestCaseStateEntry {
  readonly testCaseId: string;
  readonly state: TestCaseState;
}

// =============================================================================
// Progress & Scoring Types
// =============================================================================

export interface EvalProgress {
  readonly completedSamples: number;
  readonly totalSamples: number;
  readonly completedTestCases: number;
  readonly totalTestCases: number;
  readonly currentTestCaseId: string | null;
  readonly elapsedMs: number;
  readonly estimatedRemainingMs: number | null;
  readonly testCaseStates: readonly TestCaseStateEntry[];
}

export interface TestCase {
  readonly id: string;
  readonly tags: readonly string[];
  readonly input: string;
  readonly expectedTrajectory: readonly string[];
  readonly trajectoryMode: TrajectoryMode;
  readonly groundTruth: string | null;
}

export interface TestCaseSet {
  readonly id: string;
  readonly name: string;
  readonly description: string;
  readonly testCases: readonly TestCase[];
  readonly createdAt: string;
}

export interface FBetaScore {
  readonly precision: number;
  readonly recall: number;
  readonly fScore: number;
  readonly beta: number;
}

// =============================================================================
// Action Comparison Types (from core::action_comparison)
// =============================================================================

/** Action match status — 6 exhaustive variants. */
export type ActionStatus =
  | "exact"
  | "products_wrong"
  | "scope_wrong"
  | "both_wrong"
  | "missing"
  | "extra";

export interface ProductComparison {
  readonly expectedCount: number;
  readonly actualCount: number;
  readonly matchedCount: number;
  readonly jaccard: number;
  readonly exactMatch: boolean;
}

export interface ScopeComparison {
  readonly expectedChannels: readonly string[];
  readonly actualChannels: readonly string[];
  readonly channelsMatch: boolean;
}

/** Evidence type used for product comparison (total order: fingerprints > uuids > vacuous). */
export type ProductEvidence = "fingerprints" | "uuids" | "vacuous";

export interface ActionComparisonDetail {
  readonly index?: number;
  readonly signature: string;
  readonly status: ActionStatus;
  readonly products?: ProductComparison;
  readonly scope?: ScopeComparison;
  readonly productEvidence?: ProductEvidence;
}

export interface ComparisonSummary {
  readonly total: number;
  readonly exact: number;
  readonly productsWrong: number;
  readonly scopeWrong: number;
  readonly bothWrong: number;
  readonly missing: number;
  readonly extra: number;
}

/** Diagnostics from KV enrichment of buildPlan results. */
export interface EnrichmentDiagnostics {
  readonly planFound: boolean;
  readonly productSetFound: boolean;
  readonly scopeSetFound: boolean;
  readonly productCount: number;
  readonly fingerprintCount: number;
  readonly channelCount: number;
  readonly actionCount: number;
  readonly notes: readonly string[];
}

export interface ActionMatchResult {
  readonly pass: boolean;
  readonly actions: readonly ActionComparisonDetail[];
  readonly summary: ComparisonSummary;
  readonly issues: readonly string[];
  /** True when pass was determined via total-product-set fallback. */
  readonly totalSetFallback?: boolean;
  /** Diagnostics from KV enrichment (absent when enrichment was not performed). */
  readonly enrichmentDiagnostics?: EnrichmentDiagnostics;
}

export interface CountMatch {
  readonly tool: string;
  readonly expected: number;
  readonly actual: number;
  readonly match: boolean;
}

export interface ActionMatch {
  readonly expectedType: string;
  readonly expectedValue: number;
  readonly actualType: string;
  readonly actualValue: number;
  readonly overallMatch: boolean;
}

export interface ExecutorEvaluation {
  readonly countMatches: readonly CountMatch[];
  readonly actionMatches: readonly ActionMatch[];
  readonly weightedScore: number;
  readonly pass: boolean;
}

/**
 * Scorer result — flat struct matching Rust `ScorerResult`.
 *
 * Domain-specific scorers attach arbitrary details without modifying the type.
 * The frontend renders details by `scorerName` dispatch.
 */
export interface ScorerResult {
  readonly scorerName: string;
  readonly score: number;
  readonly details?: Record<string, unknown>;
}

export interface EvalTraceRef {
  readonly traceId: string;
  readonly threadId?: string | null;
  readonly url?: string | null;
  readonly metadata?: Record<string, unknown>;
}

/**
 * Extract the primary score [0, 1] from a ScorerResult.
 */
export function extractScore(scorer: ScorerResult): number {
  return scorer.score;
}

/**
 * Extract the authoritative sample score emitted by the harness.
 *
 * Component scorer results are diagnostics only and must not be averaged by the UI.
 */
export function extractSampleScore(sample: { readonly aggregateScore: number }): number {
  return sample.aggregateScore;
}

// =============================================================================
// Scorer Name Dispatch (One Concept, One Place)
// =============================================================================

/** Parsed scorer details — discriminated union for type-safe rendering. */
export type ParsedScorerDetails =
  | {
      readonly kind: "trajectory";
      readonly scorerName: string;
      readonly score: number;
      readonly details?: Record<string, unknown>;
    }
  | { readonly kind: "actionMatch"; readonly actionMatch: ActionMatchResult }
  | { readonly kind: "finalResponse"; readonly result: FinalResponseScoreDetail }
  | {
      readonly kind: "generic";
      readonly scorerName: string;
      readonly score: number;
      readonly details?: Record<string, unknown>;
    };

export interface ResponseScorerDetail {
  readonly id?: string;
  readonly method?: string;
  readonly passed?: boolean;
  readonly score?: number;
  readonly required?: boolean;
  readonly weight?: number;
  readonly reason?: string;
  readonly details?: Record<string, unknown>;
}

export interface FinalResponseScoreDetail {
  readonly passed?: boolean;
  readonly score?: number;
  readonly effectiveScore?: number;
  readonly passThreshold?: number;
  readonly requiredFailed?: readonly string[];
  readonly responseScorers?: readonly ResponseScorerDetail[];
  readonly reason?: string;
  readonly details?: Record<string, unknown>;
}

export interface ResponseContractScoreDetail {
  readonly valid: boolean;
  readonly contractModel?: string | null;
  readonly contractRef?: string | null;
  readonly comparisonMode?: "contract" | "exact";
  readonly matchedExpectedOutput?: boolean | null;
  readonly expectedOutput?: unknown;
  readonly actualOutput?: unknown;
  readonly errors?: readonly string[];
}

const ACTION_STATUSES = new Set<ActionStatus>([
  "exact",
  "products_wrong",
  "scope_wrong",
  "both_wrong",
  "missing",
  "extra",
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

/** Runtime shape guard: ActionMatchResult has scorer action rows. */
function isActionMatchResult(d: object): boolean {
  const r = d as Record<string, unknown>;
  if (!Array.isArray(r.actions)) return false;
  return r.actions.every((action) => {
    if (!isRecord(action)) return false;
    return (
      typeof action.signature === "string" && ACTION_STATUSES.has(action.status as ActionStatus)
    );
  });
}

function isFinalResponseScoreDetail(d: object): boolean {
  const r = d as Record<string, unknown>;
  return (
    typeof r.passed === "boolean" || typeof r.score === "number" || Array.isArray(r.responseScorers)
  );
}

/** Generic fallback that preserves the raw details for display. */
function genericDetail(scorer: ScorerResult): ParsedScorerDetails {
  return {
    kind: "generic",
    scorerName: scorer.scorerName,
    score: scorer.score,
    details: scorer.details && typeof scorer.details === "object" ? scorer.details : undefined,
  };
}

/**
 * Parse a ScorerResult into a typed discriminant.
 *
 * Runtime type guards validate the shape before narrowing — no blind casts.
 * Known harness/legacy scorers get specialized renderers; everything else
 * falls back to a generic view that still surfaces the raw `details`.
 */
export function parseScorerDetails(scorer: ScorerResult): ParsedScorerDetails {
  const d = scorer.details;
  if (!d || typeof d !== "object") {
    return genericDetail(scorer);
  }
  switch (scorer.scorerName) {
    case "trajectory":
      return {
        kind: "trajectory",
        scorerName: scorer.scorerName,
        score: scorer.score,
        details: d as Record<string, unknown>,
      };
    case "planned_actions":
    case "executed_actions":
      if (isActionMatchResult(d)) {
        return { kind: "actionMatch", actionMatch: d as unknown as ActionMatchResult };
      }
      return genericDetail(scorer);
    case "final_response":
      if (isFinalResponseScoreDetail(d)) {
        return {
          kind: "finalResponse",
          result: {
            ...(d as unknown as FinalResponseScoreDetail),
            details: d as Record<string, unknown>,
          },
        };
      }
      return genericDetail(scorer);
    default:
      return genericDetail(scorer);
  }
}

export interface SampleResult {
  readonly sampleIndex: number;
  readonly passed: boolean;
  readonly aggregateScore: number;
  readonly scores: readonly ScorerResult[];
  readonly actualTrajectory: readonly string[];
  readonly durationMs: number;
  readonly tokenUsage: TokenUsageSummary;
  readonly error: string | null;
  readonly retryCount: number;
  readonly threadId?: string;
  readonly trace?: EvalTraceRef;
  readonly responseText?: string;
  readonly parsedOutput?: unknown;
  readonly responseValidation?: ResponseValidation | null;
}

export interface TestCaseResult {
  readonly testCaseId: string;
  readonly input?: string;
  readonly samples: readonly SampleResult[];
  readonly passAtK: readonly PassAtKResult[];
  readonly aggregateScore: number;
}

export interface PassAtKResult {
  readonly k: number;
  readonly simpleEstimate: number;
  readonly unbiasedEstimate: number | null;
  readonly numSamples: number;
  readonly numCorrect: number;
}

export interface TokenUsageSummary {
  readonly inputTokens: number;
  readonly outputTokens: number;
  readonly cachedTokens: number;
  readonly cacheCreationTokens: number;
}

export interface EvalSummary {
  readonly totalTestCases: number;
  readonly passed: number;
  readonly failed: number;
  readonly skipped: number;
  readonly aggregateScore: number;
  readonly passAtK: readonly PassAtKResult[];
  readonly totalDurationMs: number;
  readonly totalUsage: TokenUsageSummary;
}

/** How to aggregate per-sample scores into a test-case aggregate. */
export type AggregationStrategy = "passRate" | "meanScore";

export interface EvalCapabilityMode {
  readonly mode: EvalMode;
  readonly label: string;
  readonly description: string;
  readonly agentId: string;
  readonly role: string;
  readonly targetAgentId?: string | null;
}

export interface EvalCapabilities {
  readonly workspaceKey: string;
  readonly modes: readonly EvalCapabilityMode[];
}

export interface EvalConfig {
  readonly mode: EvalMode;
  readonly targetAgentId: string | null;
  readonly testCaseSetId: string;
  readonly samplesPerCase: number;
  readonly passThreshold: number;
  readonly concurrency: number;
  readonly kValues: readonly number[];
  readonly provider: string | null;
  readonly model: string | null;
  readonly timeoutPerSampleSecs: number;
  readonly tagsFilter: readonly string[] | null;
  readonly testCaseIds: readonly string[] | null;
  readonly retryPolicy?: EvalRetryConfig;
  readonly aggregationStrategy: AggregationStrategy;
  readonly scoreWeights?: ScoreWeights | null;
  readonly scorerConfig?: Record<string, unknown> | null;
  readonly requestOverrides?: Record<string, unknown> | null;
}

export interface EvalRun {
  readonly id: string;
  readonly activityRunId?: string | null;
  readonly config: EvalConfig;
  readonly status: EvalStatus;
  readonly results: readonly TestCaseResult[];
  readonly dataReadiness?: DataReadiness | null;
  readonly createdAt: string;
  readonly updatedAt: string;
  readonly parentRunId?: string;
  readonly rerunTestCaseIds?: readonly string[];
}

/** Lightweight projection of EvalRun for list endpoints (no results array). */
export interface EvalRunSummary {
  readonly id: string;
  readonly activityRunId?: string | null;
  readonly config: EvalConfig;
  readonly status: EvalStatus;
  readonly resultCount: number;
  readonly dataReadiness?: DataReadiness | null;
  readonly createdAt: string;
  readonly updatedAt: string;
  readonly parentRunId?: string;
  readonly rerunTestCaseIds?: readonly string[];
}

// =============================================================================
// EvalEvent (Discriminated Union)
// =============================================================================

export type EvalEvent =
  | {
      readonly type: "started";
      readonly runId: string;
      readonly config: EvalConfig;
      readonly dataReadiness?: DataReadiness | null;
    }
  | { readonly type: "progress"; readonly progress: EvalProgress }
  | {
      readonly type: "testCaseStarted";
      readonly testCaseId: string;
      readonly testCaseIndex: number;
    }
  | {
      readonly type: "sampleProgress";
      readonly testCaseId: string;
      readonly sampleIndex: number;
      readonly completedSamples: number;
      readonly totalSamples: number;
    }
  | { readonly type: "sampleComplete"; readonly testCaseId: string; readonly sample: SampleResult }
  | { readonly type: "testCaseComplete"; readonly result: TestCaseResult }
  | {
      readonly type: "testCaseSkipped";
      readonly testCaseId: string;
      readonly reason: string;
    }
  | { readonly type: "completed"; readonly summary: EvalSummary }
  | { readonly type: "paused"; readonly progress: EvalProgress }
  | { readonly type: "resumed"; readonly progress: EvalProgress }
  | { readonly type: "error"; readonly message: string }
  | { readonly type: "childProgress"; readonly childRunId: string; readonly event: EvalEvent };

// =============================================================================
// RunPhase (Discriminated Union — replaces isRunning/isPaused booleans)
// =============================================================================

export type RunPhase =
  | { readonly phase: "idle" }
  | { readonly phase: "running"; readonly activeEvalId: string }
  | { readonly phase: "paused"; readonly activeEvalId: string }
  | { readonly phase: "error"; readonly message: string };

// =============================================================================
// Retry Config
// =============================================================================

export interface EvalRetryConfig {
  readonly maxRetries: number;
  readonly initialBackoffMs: number;
  readonly backoffMultiplier: number;
}

// =============================================================================
// Type Guards
// =============================================================================

export const isEvalStarted = (e: EvalEvent): e is Extract<EvalEvent, { type: "started" }> =>
  e.type === "started";

export const isEvalProgress = (e: EvalEvent): e is Extract<EvalEvent, { type: "progress" }> =>
  e.type === "progress";

export const isEvalSampleComplete = (
  e: EvalEvent
): e is Extract<EvalEvent, { type: "sampleComplete" }> => e.type === "sampleComplete";

export const isEvalTestCaseComplete = (
  e: EvalEvent
): e is Extract<EvalEvent, { type: "testCaseComplete" }> => e.type === "testCaseComplete";

export const isEvalCompleted = (e: EvalEvent): e is Extract<EvalEvent, { type: "completed" }> =>
  e.type === "completed";

export const isEvalError = (e: EvalEvent): e is Extract<EvalEvent, { type: "error" }> =>
  e.type === "error";

// =============================================================================
// Default Config
// =============================================================================

export const DEFAULT_EVAL_CONFIG: EvalConfig = {
  mode: "sequential",
  targetAgentId: null,
  testCaseSetId: "",
  samplesPerCase: 3,
  passThreshold: 0.7,
  concurrency: 2,
  kValues: [1, 3],
  provider: null,
  model: null,
  timeoutPerSampleSecs: 120,
  tagsFilter: null,
  testCaseIds: null,
  aggregationStrategy: "passRate",
  scoreWeights: null,
  scorerConfig: null,
  requestOverrides: null,
};

// =============================================================================
// Status Color Map
// =============================================================================

export const EVAL_STATUS_COLORS: Record<EvalStatusKey, string> = {
  completed: "var(--dot-emerald)",
  failed: "var(--dot-red)",
  running: "var(--primary)",
  paused: "var(--dot-amber)",
  skipped: "var(--dot-orange)",
  queued: "var(--muted-foreground)",
  cancelled: "var(--muted-foreground)",
};

// =============================================================================
// Eval Fork (fork tracking)
// =============================================================================

export interface EvalFork {
  readonly id: string;
  readonly threadId: string;
  readonly parentThreadId?: string;
  readonly forkAtMessageIndex?: number;
  readonly editedContent?: string;
  readonly label?: string;
  readonly createdAt: string;
}

// =============================================================================
// Trajectory Step View (drill-down)
// =============================================================================

export type TrajectoryMatchStatus = "matched" | "unexpected" | "missing";

export interface TrajectoryStepView {
  readonly index: number;
  readonly toolName: string;
  readonly matchStatus: TrajectoryMatchStatus;
}

/**
 * Total function: trajectory match status -> human-readable label.
 */
export function matchStatusLabel(status: TrajectoryMatchStatus): string {
  switch (status) {
    case "matched":
      return "matched";
    case "unexpected":
      return "unexpected";
    case "missing":
      return "missing";
    default:
      return assertNever(status);
  }
}

/**
 * Pure function: diff actual vs expected trajectory into view steps.
 */
export function synthesizeTrajectorySteps(
  actual: readonly string[],
  expected: readonly string[],
  mode: TrajectoryMode
): readonly TrajectoryStepView[] {
  const steps: TrajectoryStepView[] = [];

  if (mode === "strict" || mode === "subsequence") {
    let actualIdx = 0;
    for (const tool of expected) {
      const found = actual.indexOf(tool, actualIdx);
      if (found !== -1) {
        if (mode === "strict") {
          for (let i = actualIdx; i < found; i++) {
            steps.push({ index: steps.length, toolName: actual[i], matchStatus: "unexpected" });
          }
        }
        steps.push({ index: steps.length, toolName: tool, matchStatus: "matched" });
        actualIdx = found + 1;
      } else {
        steps.push({ index: steps.length, toolName: tool, matchStatus: "missing" });
      }
    }
    if (mode === "strict") {
      for (let i = actualIdx; i < actual.length; i++) {
        steps.push({ index: steps.length, toolName: actual[i], matchStatus: "unexpected" });
      }
    }
  } else {
    const expectedCounts = new Map<string, number>();
    for (const tool of expected) {
      expectedCounts.set(tool, (expectedCounts.get(tool) ?? 0) + 1);
    }
    for (const tool of actual) {
      const remaining = expectedCounts.get(tool) ?? 0;
      if (remaining > 0) {
        expectedCounts.set(tool, remaining - 1);
        steps.push({ index: steps.length, toolName: tool, matchStatus: "matched" });
      } else if (mode === "superset") {
        steps.push({ index: steps.length, toolName: tool, matchStatus: "matched" });
      } else {
        steps.push({ index: steps.length, toolName: tool, matchStatus: "unexpected" });
      }
    }
    for (const [tool, count] of expectedCounts) {
      for (let i = 0; i < count; i++) {
        steps.push({ index: steps.length, toolName: tool, matchStatus: "missing" });
      }
    }
  }

  return steps;
}
