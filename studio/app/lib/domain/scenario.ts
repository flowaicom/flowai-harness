/**
 * Generic agent plan/action domain types.
 *
 * These types model the universal concept of an agent producing a plan
 * (sequence of proposed actions) that can be approved, executed, or rejected.
 * The action payload is generic — agents define their own action schemas.
 *
 * @module domain/scenario
 */

import type { FilterSet } from "./filter";

// ============================================================================
// Core IDs (Newtypes)
// ============================================================================

export type ScenarioId = string;
export type ActionId = string;
export type EntitySetId = string;
export type ScopeSetId = string;
export type PlanId = string;
export type EntityId = string;

// ============================================================================
// Scope Types (generic key-value dimensions)
// ============================================================================

/**
 * Agent-defined scope dimensions.
 *
 * Each key is a dimension name (e.g. "region", "channel", "segment"),
 * each value is the selected value for that dimension.
 */
export type ActionScope = Record<string, string | undefined>;

export const isScopeEmpty = (scope: ActionScope): boolean =>
  Object.values(scope).every((v) => v === undefined || v === "");

// ============================================================================
// Entity Set (Stored in KV)
// ============================================================================

export interface NumericRange {
  readonly min: number;
  readonly max: number;
  readonly mean: number;
}

export interface DistributionEntry {
  readonly value: string;
  readonly count: number;
  readonly percentage: number;
}

/**
 * Summary statistics for a set of entities.
 *
 * `distributions` is a map of column name → value distribution.
 * `numericRanges` is a map of column name → numeric range stats.
 */
export interface EntitySetGlimpse {
  readonly entityCount: number;
  readonly distributions: Record<string, DistributionEntry[]>;
  readonly numericRanges: Record<string, NumericRange>;
}

export interface StoredEntitySet {
  readonly id: EntitySetId;
  readonly ownerId: string;
  readonly entityIds: EntityId[];
  readonly filters: FilterSet;
  readonly scope?: ActionScope;
  readonly glimpse?: EntitySetGlimpse;
  readonly count: number;
  readonly createdAt: string;
}

// ============================================================================
// Plan Types
// ============================================================================

/**
 * A proposed action in a plan.
 *
 * `actionType` and `payload` are agent-defined — the framework does not
 * prescribe what actions look like. The entity set and scope set IDs
 * reference KV-stored data for the target entities and scope.
 */
export interface SuggestedAction {
  readonly name: string;
  readonly actionType: string;
  readonly entitySetId: EntitySetId;
  readonly scopeSetId: ScopeSetId;
  readonly payload: Record<string, unknown>;
}

export type PlanStatus = "pending" | "approved" | "executing" | "executed" | "failed";

export interface StoredPlan {
  readonly id: PlanId;
  readonly ownerId: string;
  readonly originalQuery: string;
  readonly interpretedIntent: string;
  readonly suggestedActions: SuggestedAction[];
  readonly assumptions?: string[];
  readonly warnings?: string[];
  readonly totalEntityCount: number;
  readonly status: PlanStatus;
  readonly createdAt: string;
}

// ============================================================================
// Plan Status Transitions
// ============================================================================

const validTransitions: Record<PlanStatus, PlanStatus[]> = {
  pending: ["approved", "failed"],
  approved: ["executing", "failed"],
  executing: ["executed", "failed"],
  executed: [],
  failed: [],
};

export const canTransition = (from: PlanStatus, to: PlanStatus): boolean =>
  validTransitions[from].includes(to);

export const isTerminalStatus = (status: PlanStatus): boolean =>
  validTransitions[status].length === 0;

// ============================================================================
// Glimpse Display Helpers
// ============================================================================

export const formatDistribution = (entries: DistributionEntry[], limit = 3): string => {
  const top = entries.slice(0, limit);
  const remaining = entries.length - limit;

  const formatted = top.map((e) => `${e.value} (${e.count})`).join(", ");
  if (remaining > 0) {
    return `${formatted}, +${remaining} more`;
  }
  return formatted;
};

export const formatNumericRange = (range: NumericRange): string => {
  const format = (n: number) => n.toFixed(2);
  return `${format(range.min)} – ${format(range.max)} (avg: ${format(range.mean)})`;
};

export const summarizeGlimpse = (glimpse: EntitySetGlimpse): string[] => {
  const lines: string[] = [];
  lines.push(`${glimpse.entityCount} entities`);

  for (const [key, range] of Object.entries(glimpse.numericRanges)) {
    lines.push(`${key}: ${formatNumericRange(range)}`);
  }

  for (const [key, dist] of Object.entries(glimpse.distributions)) {
    if (dist.length > 0) {
      lines.push(`${key}: ${formatDistribution(dist)}`);
    }
  }

  return lines;
};
