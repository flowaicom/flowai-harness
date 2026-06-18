/**
 * Entity filter algebra for scenario targeting.
 *
 * Algebraic Structure:
 * - FilterSet forms a BoundedSemilattice under intersection (meet)
 * - Top element: empty FilterSet (matches all entities)
 * - Meet operation: combining filters narrows the result
 *
 * Laws:
 * - Identity: meet(top, a) = a = meet(a, top)
 * - Associativity: meet(meet(a, b), c) = meet(a, meet(b, c))
 * - Commutativity: meet(a, b) = meet(b, a)
 * - Idempotence: meet(a, a) = a
 *
 * @module domain/filter
 */

// ============================================================================
// Atomic Filter Types (Discriminated Union)
// ============================================================================

/**
 * Numeric comparison operators.
 */
export type NumericOperator = "=" | ">" | "<" | ">=" | "<=" | "!=" | "BETWEEN";

/**
 * Measure metric name — agent-defined (e.g. "availability", "revenue", "latency").
 */
export type MeasureMetric = string;

/**
 * Aggregation functions for measures.
 */
export type MeasureAggregate = "avg" | "any" | "min" | "max";

/**
 * Categorical filter (column IN ('val1', 'val2')).
 */
export interface MatchedFilter {
  readonly _tag: "matched";
  readonly field: string;
  readonly values: string[];
}

/**
 * Numeric range filter (column > 500, column BETWEEN 300 AND 600).
 */
export interface NumericFilter {
  readonly _tag: "numeric";
  readonly field: string;
  readonly operator: NumericOperator;
  readonly value: number;
  readonly value2?: number; // For BETWEEN
}

/**
 * Boolean attribute filter (is_active = true).
 */
export interface BooleanFilter {
  readonly _tag: "boolean";
  readonly field: string;
  readonly value: boolean;
}

/**
 * Measure filter with aggregation (avg(metric) > 0.5).
 */
export interface MeasureFilter {
  readonly _tag: "measure";
  readonly metric: MeasureMetric;
  readonly aggregate: MeasureAggregate;
  readonly operator: NumericOperator;
  readonly value: number;
  readonly value2?: number;
}

/**
 * Entity filter discriminated union.
 */
export type EntityFilter = MatchedFilter | NumericFilter | BooleanFilter | MeasureFilter;

// ============================================================================
// Type Guards
// ============================================================================

export const isMatchedFilter = (f: EntityFilter): f is MatchedFilter => f._tag === "matched";

export const isNumericFilter = (f: EntityFilter): f is NumericFilter => f._tag === "numeric";

export const isBooleanFilter = (f: EntityFilter): f is BooleanFilter => f._tag === "boolean";

export const isMeasureFilter = (f: EntityFilter): f is MeasureFilter => f._tag === "measure";

// ============================================================================
// Constructors
// ============================================================================

export const matchedFilter = (field: string, values: string[]): MatchedFilter => ({
  _tag: "matched",
  field,
  values,
});

export const numericFilter = (
  field: string,
  operator: NumericOperator,
  value: number,
  value2?: number
): NumericFilter => ({
  _tag: "numeric",
  field,
  operator,
  value,
  value2,
});

export const booleanFilter = (field: string, value: boolean): BooleanFilter => ({
  _tag: "boolean",
  field,
  value,
});

export const measureFilter = (
  metric: MeasureMetric,
  aggregate: MeasureAggregate,
  operator: NumericOperator,
  value: number,
  value2?: number
): MeasureFilter => ({
  _tag: "measure",
  metric,
  aggregate,
  operator,
  value,
  value2,
});

// ============================================================================
// FilterSet (BoundedSemilattice)
// ============================================================================

/**
 * A set of filters that together define entity selection criteria.
 *
 * Forms a BoundedSemilattice:
 * - Top element: empty FilterSet (matches all)
 * - Meet: combining filters (intersection)
 */
export interface FilterSet {
  readonly matched: MatchedFilter[];
  readonly numeric: NumericFilter[];
  readonly boolean: BooleanFilter[];
  readonly measure: MeasureFilter[];
}

/**
 * Top element - matches all entities.
 */
export const filterSetTop: FilterSet = {
  matched: [],
  numeric: [],
  boolean: [],
  measure: [],
};

/**
 * Check if filter set is empty (top element).
 */
export const isFilterSetEmpty = (fs: FilterSet): boolean =>
  fs.matched.length === 0 &&
  fs.numeric.length === 0 &&
  fs.boolean.length === 0 &&
  fs.measure.length === 0;

/**
 * Canonical sort key for a filter (for deterministic ordering).
 */
const filterSortKey = (f: EntityFilter): string => {
  switch (f._tag) {
    case "matched":
      return `1:${f.field}:${f.values.sort().join(",")}`;
    case "numeric":
      return `2:${f.field}:${f.operator}:${f.value}:${f.value2 ?? ""}`;
    case "boolean":
      return `3:${f.field}:${f.value}`;
    case "measure":
      return `4:${f.metric}:${f.aggregate}:${f.operator}:${f.value}:${f.value2 ?? ""}`;
  }
};

/**
 * Normalize a filter set (canonical ordering).
 *
 * Law: normalize(normalize(a)) = normalize(a) (idempotent)
 */
export const normalizeFilterSet = (fs: FilterSet): FilterSet => ({
  matched: [...fs.matched].sort((a, b) => filterSortKey(a).localeCompare(filterSortKey(b))),
  numeric: [...fs.numeric].sort((a, b) => filterSortKey(a).localeCompare(filterSortKey(b))),
  boolean: [...fs.boolean].sort((a, b) => filterSortKey(a).localeCompare(filterSortKey(b))),
  measure: [...fs.measure].sort((a, b) => filterSortKey(a).localeCompare(filterSortKey(b))),
});

/**
 * Meet operation (combine filter sets).
 *
 * Laws:
 * - meet(top, a) = a
 * - meet(a, top) = a
 * - meet(a, b) = meet(b, a)
 * - meet(meet(a, b), c) = meet(a, meet(b, c))
 */
export const meetFilterSets = (a: FilterSet, b: FilterSet): FilterSet =>
  normalizeFilterSet({
    matched: [...a.matched, ...b.matched],
    numeric: [...a.numeric, ...b.numeric],
    boolean: [...a.boolean, ...b.boolean],
    measure: [...a.measure, ...b.measure],
  });

/**
 * Create a filter set from a single filter.
 */
export const singletonFilterSet = (filter: EntityFilter): FilterSet => {
  switch (filter._tag) {
    case "matched":
      return { ...filterSetTop, matched: [filter] };
    case "numeric":
      return { ...filterSetTop, numeric: [filter] };
    case "boolean":
      return { ...filterSetTop, boolean: [filter] };
    case "measure":
      return { ...filterSetTop, measure: [filter] };
  }
};

/**
 * Get total filter count.
 */
export const filterSetCount = (fs: FilterSet): number =>
  fs.matched.length + fs.numeric.length + fs.boolean.length + fs.measure.length;

// ============================================================================
// Filter Display Helpers
// ============================================================================

/**
 * Human-readable description of a filter.
 */
export const describeFilter = (f: EntityFilter): string => {
  switch (f._tag) {
    case "matched":
      return f.values.length === 1
        ? `${f.field} = "${f.values[0]}"`
        : `${f.field} IN (${f.values.map((v) => `"${v}"`).join(", ")})`;
    case "numeric":
      if (f.operator === "BETWEEN" && f.value2 !== undefined) {
        return `${f.field} BETWEEN ${f.value} AND ${f.value2}`;
      }
      return `${f.field} ${f.operator} ${f.value}`;
    case "boolean":
      return `${f.field} = ${f.value}`;
    case "measure": {
      const agg = f.aggregate === "any" ? "" : `${f.aggregate}(`;
      const close = f.aggregate === "any" ? "" : ")";
      if (f.operator === "BETWEEN" && f.value2 !== undefined) {
        return `${agg}${f.metric}${close} BETWEEN ${f.value} AND ${f.value2}`;
      }
      return `${agg}${f.metric}${close} ${f.operator} ${f.value}`;
    }
  }
};

/**
 * Describe all filters in a filter set.
 */
export const describeFilterSet = (fs: FilterSet): string[] => {
  const all: EntityFilter[] = [...fs.matched, ...fs.numeric, ...fs.boolean, ...fs.measure];
  return all.map(describeFilter);
};
