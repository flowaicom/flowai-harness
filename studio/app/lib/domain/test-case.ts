/**
 * Test case domain types.
 *
 * All types are readonly interfaces with discriminated unions.
 *
 * @module domain/test-case
 */

import type { DataReadiness } from "./data";
import type { TrajectoryMode } from "./eval";

// =============================================================================
// Literal Unions
// =============================================================================

export type TestCaseStatus = "draft" | "active" | "archived";

// =============================================================================
// Trajectory Composition
// =============================================================================

export interface TrajectoryStep {
  readonly toolName: string;
  readonly source: TrajectoryStepSource;
  readonly position: number;
}

export type TrajectoryStepSource =
  | { readonly type: "fromThread"; readonly threadId: string; readonly originalIndex: number }
  | { readonly type: "fromAgent"; readonly runId: string }
  | { readonly type: "manual"; readonly reason: string | null };

export type TrajectorySource =
  | {
      readonly type: "threadSegment";
      readonly threadId: string;
      readonly fromIndex: number;
      readonly toIndex: number;
    }
  | { readonly type: "agentRun"; readonly runId: string }
  | { readonly type: "manual" };

// =============================================================================
// Builder Session (ephemeral KV state for the test case builder)
// =============================================================================

export interface TestCaseBuilderSession {
  readonly sessionId: string;
  readonly userPrompt: string | null;
  readonly composedTrajectory: readonly TrajectoryStep[];
  readonly trajectorySources: readonly TrajectorySource[];
  readonly trajectoryMode: TrajectoryMode | null;
  readonly tags: readonly string[];
  /** Legacy text ground truth (freeform notes). */
  readonly groundTruth?: string | null;
  /** Structured ground truth for eval scoring (set by builder agent). */
  readonly structuredGroundTruth?: GroundTruth | null;
  readonly createdAt: string;
  readonly updatedAt: string;
}

// =============================================================================
// Structured Ground Truth
// =============================================================================

/**
 * Comparison operators for numeric assertions in ground truth.
 *
 * These use semantic names (not SQL symbols) to be unambiguous in JSON.
 */
export type GroundTruthComparisonOp =
  | "greaterThan"
  | "greaterThanOrEqual"
  | "lessThan"
  | "lessThanOrEqual"
  | "equal"
  | "notEqual";

export interface GroundTruthNumericFilter {
  readonly operator: GroundTruthComparisonOp;
  readonly value: number;
}

export interface GroundTruthMeasureFilter {
  readonly column: string;
  readonly operator: GroundTruthComparisonOp;
  readonly value: number;
}

/**
 * Expected filter assertions — agent-agnostic.
 *
 * - `matchedFilters`: categorical column → expected values
 * - `numericFilters`: numeric column → comparison
 * - `booleanFilters`: boolean column → expected value
 * - `measureFilters`: measure column → comparison
 */
export interface ExpectedFilters {
  readonly matchedFilters: Record<string, string[]>;
  readonly numericFilters: Record<string, GroundTruthNumericFilter>;
  readonly booleanFilters: Record<string, boolean>;
  readonly measureFilters: Record<string, GroundTruthMeasureFilter>;
}

/**
 * Expected scope — generic key-value dimensions.
 *
 * Each key is a scope dimension name, each value is the expected values
 * for that dimension. Agent defines what dimensions exist.
 */
export type ExpectedScope = Record<string, string[]>;

/**
 * Expected action payload — agent-defined key-value pairs.
 *
 * The framework does not prescribe what an action looks like.
 * Each agent defines its own action types and payloads.
 */
export type ActionPayload = Record<string, unknown>;
export type ActionPayloadMatchMode = "exact" | "subset";

export interface ExpectedAction {
  readonly actionType: string;
  readonly payload: ActionPayload;
  readonly entityIds?: string[];
  /** Content-addressed entity fingerprints (preferred over entityIds). */
  readonly entityFingerprints?: string[];
  readonly scope?: ExpectedScope;
  readonly entitySql?: string;
  readonly entityDescription?: string;
  readonly expectedFilters?: ExpectedFilters;
}

export interface ExpectedGroup {
  readonly filters: ExpectedFilters;
  readonly scope?: ExpectedScope;
  readonly actions: ExpectedAction[];
  readonly entityIds?: string[];
  readonly entitySql?: string;
  readonly entityDescription?: string;
}

export type GroundTruth =
  | { readonly kind: "text"; readonly text: string }
  | {
      readonly kind: "structured";
      readonly data?: unknown;
      readonly payload?: unknown;
      readonly schema?: string | null;
    }
  | { readonly kind: "textOnly"; readonly text: string }
  | {
      readonly kind: "flat";
      readonly payloadMatch?: ActionPayloadMatchMode;
      readonly expectedActions: ExpectedAction[];
      readonly expectedFilters: ExpectedFilters;
      readonly expectedScope: ExpectedScope;
      readonly groundTruthSql?: string;
      readonly groundTruthEntityIds?: string[];
      readonly groundTruthEntityFingerprints?: string[];
    }
  | { readonly kind: "multiGroup"; readonly groups: ExpectedGroup[] };

// -- Factory helpers --

export const EMPTY_EXPECTED_FILTERS: ExpectedFilters = {
  matchedFilters: {},
  numericFilters: {},
  booleanFilters: {},
  measureFilters: {},
};

export const EMPTY_SCOPE: ExpectedScope = {};

/** @deprecated Use EMPTY_SCOPE */
export const EMPTY_SCOPE_CODES = EMPTY_SCOPE;

export function createEmptyAction(): ExpectedAction {
  return { actionType: "", payload: {} };
}

export function createEmptyGroup(): ExpectedGroup {
  return { filters: { ...EMPTY_EXPECTED_FILTERS }, actions: [createEmptyAction()] };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasActionBuckets(value: Record<string, unknown>): boolean {
  return "plannedActions" in value || "executedActions" in value;
}

function normalizeFlatGroundTruthPayload(value: Record<string, unknown>): Record<string, unknown> {
  return {
    ...value,
    kind: "flat",
    payloadMatch:
      value.payloadMatch === "subset" || value.payloadMatch === "exact"
        ? value.payloadMatch
        : "exact",
  };
}

function normalizeStructuredPayload(value: Record<string, unknown>): Record<string, unknown> {
  const payload = value.payload ?? value.data;
  if (isRecord(payload) && payload.kind === "flat") {
    const { data: _data, ...rest } = value;
    return { ...rest, payload: normalizeFlatGroundTruthPayload(payload) };
  }
  return value;
}

export function normalizeStructuredGroundTruthJson(value: unknown): GroundTruth {
  if (!isRecord(value)) {
    throw new Error("Structured ground truth must be a JSON object.");
  }

  const kind = typeof value.kind === "string" ? value.kind : null;
  if (kind === "structured") {
    return normalizeStructuredPayload(value) as GroundTruth;
  }

  if (kind === "flat") {
    return { kind: "structured", payload: normalizeFlatGroundTruthPayload(value) };
  }

  if (kind === "text") {
    return { kind: "structured", payload: value };
  }

  if (hasActionBuckets(value)) {
    return { kind: "structured", payload: normalizeFlatGroundTruthPayload(value) };
  }

  return { kind: "structured", payload: value };
}

export function parseStructuredGroundTruthJson(text: string): GroundTruth | null {
  const trimmed = text.trim();
  if (!trimmed) return null;
  return normalizeStructuredGroundTruthJson(JSON.parse(trimmed));
}

export function formatStructuredGroundTruthJson(gt: GroundTruth | null): string {
  return gt ? JSON.stringify(gt, null, 2) : "";
}

// =============================================================================
// Rich Test Case
// =============================================================================

export interface AuthoredTestCase {
  readonly id: string;
  readonly name: string;
  readonly description: string | null;
  readonly input: string;
  readonly status: TestCaseStatus;
  readonly expectedTrajectory: readonly string[];
  readonly trajectoryMode: TrajectoryMode;
  readonly groundTruth: string | null;
  readonly structuredGroundTruth: GroundTruth | null;
  readonly tags: readonly string[];
  readonly trajectoryProvenance: readonly TrajectoryStep[];
  readonly trajectorySources: readonly TrajectorySource[];
  readonly sourceThreadId: string | null;
  readonly sourceSessionId: string | null;
  readonly expectedResponse?: unknown | null;
  readonly finalResponse?: unknown | null;
  readonly dataReadiness?: DataReadiness | null;
  readonly createdAt: string;
  readonly updatedAt: string;
}

// =============================================================================
// Tool Call Extraction
// =============================================================================

export interface ToolCallEntry {
  readonly index: number;
  readonly toolName: string;
  readonly args: unknown;
  readonly result: unknown | null;
  readonly invocationId: string;
  readonly messageIndex: number;
}

export interface TraceResponse {
  readonly threadId: string;
  readonly toolCalls: readonly ToolCallEntry[];
  readonly total: number;
}

// =============================================================================
// Tool Catalog
// =============================================================================

/**
 * Tool category — string-based so agents can define their own categories.
 */
export type ToolCategory = string;

/** Default category display order for built-in categories. */
export const CATEGORY_ORDER: string[] = [
  "discovery",
  "planning",
  "execution",
  "knowledge",
  "delegation",
];

export interface ToolCatalogEntry {
  readonly name: string;
  readonly description: string;
  readonly category: ToolCategory;
}

// =============================================================================
// Status Colors
// =============================================================================

export const TEST_CASE_STATUS_COLORS: Record<TestCaseStatus, string> = {
  draft: "var(--muted-foreground)",
  active: "var(--dot-emerald)",
  archived: "var(--dot-amber)",
};

/** CSS class for the status dot. */
export const TEST_STATUS_DOT_CLASS: Record<TestCaseStatus, string> = {
  draft: "bg-muted-foreground/50",
  active: "bg-[var(--dot-emerald)]",
  archived: "bg-[var(--dot-amber)]",
};

/** CSS classes for status badge (translucent background + solid text). */
export const TEST_STATUS_BADGE_CLASS: Record<TestCaseStatus, string> = {
  draft: "bg-muted text-muted-foreground",
  active: "bg-[var(--accent-emerald)] text-[var(--dot-emerald)]",
  archived: "bg-[var(--accent-amber)] text-[var(--dot-amber)]",
};
