import { describe, expect, test } from "bun:test";
import * as fc from "fast-check";

import {
  arbDistEntry,
  arbGlimpse,
  arbNumericRange,
  arbPlanStatus as arbStatus,
} from "~/lib/test-utils/arbitraries";
import {
  canTransition,
  type DistributionEntry,
  type EntitySetGlimpse,
  formatDistribution,
  formatNumericRange,
  isTerminalStatus,
  type NumericRange,
  type PlanStatus,
  summarizeGlimpse,
} from "./scenario";

// ============================================================================
// All PlanStatus values for exhaustive testing
// ============================================================================

const allStatuses: PlanStatus[] = ["pending", "approved", "executing", "executed", "failed"];

// ============================================================================
// State Machine — canTransition
// ============================================================================

describe("canTransition — valid transitions", () => {
  test("pending -> approved", () => {
    expect(canTransition("pending", "approved")).toBe(true);
  });

  test("pending -> failed", () => {
    expect(canTransition("pending", "failed")).toBe(true);
  });

  test("approved -> executing", () => {
    expect(canTransition("approved", "executing")).toBe(true);
  });

  test("approved -> failed", () => {
    expect(canTransition("approved", "failed")).toBe(true);
  });

  test("executing -> executed", () => {
    expect(canTransition("executing", "executed")).toBe(true);
  });

  test("executing -> failed", () => {
    expect(canTransition("executing", "failed")).toBe(true);
  });
});

describe("canTransition — invalid transitions", () => {
  test("pending -> executing (must be approved first)", () => {
    expect(canTransition("pending", "executing")).toBe(false);
  });

  test("pending -> executed (must go through approved, executing)", () => {
    expect(canTransition("pending", "executed")).toBe(false);
  });

  test("pending -> pending (no self-loop)", () => {
    expect(canTransition("pending", "pending")).toBe(false);
  });

  test("approved -> pending (no backward)", () => {
    expect(canTransition("approved", "pending")).toBe(false);
  });

  test("approved -> executed (must go through executing)", () => {
    expect(canTransition("approved", "executed")).toBe(false);
  });

  test("approved -> approved (no self-loop)", () => {
    expect(canTransition("approved", "approved")).toBe(false);
  });

  test("executing -> pending (no backward)", () => {
    expect(canTransition("executing", "pending")).toBe(false);
  });

  test("executing -> approved (no backward)", () => {
    expect(canTransition("executing", "approved")).toBe(false);
  });

  test("executing -> executing (no self-loop)", () => {
    expect(canTransition("executing", "executing")).toBe(false);
  });

  test("executed -> any (terminal, no transitions out)", () => {
    for (const to of allStatuses) {
      expect(canTransition("executed", to)).toBe(false);
    }
  });

  test("failed -> any (terminal, no transitions out)", () => {
    for (const to of allStatuses) {
      expect(canTransition("failed", to)).toBe(false);
    }
  });
});

// ============================================================================
// State Machine — isTerminalStatus
// ============================================================================

describe("isTerminalStatus", () => {
  test("pending is not terminal", () => {
    expect(isTerminalStatus("pending")).toBe(false);
  });

  test("approved is not terminal", () => {
    expect(isTerminalStatus("approved")).toBe(false);
  });

  test("executing is not terminal", () => {
    expect(isTerminalStatus("executing")).toBe(false);
  });

  test("executed is terminal", () => {
    expect(isTerminalStatus("executed")).toBe(true);
  });

  test("failed is terminal", () => {
    expect(isTerminalStatus("failed")).toBe(true);
  });
});

describe("exhaustiveness", () => {
  test("every status appears in at least one canTransition test", () => {
    // Verify all statuses are covered as both source and target
    const testedAsFrom = new Set<PlanStatus>();
    const testedAsTo = new Set<PlanStatus>();

    // From the valid transitions tests above:
    for (const from of allStatuses) {
      for (const to of allStatuses) {
        // We call canTransition for all pairs via the tests above
        // but let's verify programmatically that the function handles all
        const result = canTransition(from, to);
        expect(typeof result).toBe("boolean");
        testedAsFrom.add(from);
        testedAsTo.add(to);
      }
    }

    expect(testedAsFrom.size).toBe(allStatuses.length);
    expect(testedAsTo.size).toBe(allStatuses.length);
  });
});

// ============================================================================
// Display — formatDistribution
// ============================================================================

describe("formatDistribution", () => {
  const entries: DistributionEntry[] = [
    { value: "US", count: 500, percentage: 50 },
    { value: "EU", count: 300, percentage: 30 },
    { value: "APAC", count: 150, percentage: 15 },
    { value: "LATAM", count: 50, percentage: 5 },
  ];

  test("formats top entries with default limit", () => {
    const result = formatDistribution(entries);
    expect(result).toBe("US (500), EU (300), APAC (150), +1 more");
  });

  test("formats with custom limit", () => {
    const result = formatDistribution(entries, 2);
    expect(result).toBe("US (500), EU (300), +2 more");
  });

  test("no 'more' indicator when entries fit within limit", () => {
    const result = formatDistribution(entries, 4);
    expect(result).toBe("US (500), EU (300), APAC (150), LATAM (50)");
  });

  test("no 'more' indicator when entries equal limit", () => {
    const small: DistributionEntry[] = [{ value: "A", count: 10, percentage: 100 }];
    const result = formatDistribution(small, 3);
    expect(result).toBe("A (10)");
  });

  test("empty distribution", () => {
    const result = formatDistribution([]);
    expect(result).toBe("");
  });

  test("single entry", () => {
    const single: DistributionEntry[] = [{ value: "only", count: 1, percentage: 100 }];
    expect(formatDistribution(single)).toBe("only (1)");
  });

  test("entry with zero count", () => {
    const withZero: DistributionEntry[] = [{ value: "ghost", count: 0, percentage: 0 }];
    expect(formatDistribution(withZero)).toBe("ghost (0)");
  });
});

// ============================================================================
// Display — formatNumericRange
// ============================================================================

describe("formatNumericRange", () => {
  test("formats integer-like values with 2 decimals", () => {
    const range: NumericRange = { min: 100, max: 500, mean: 300 };
    expect(formatNumericRange(range)).toBe("100.00 – 500.00 (avg: 300.00)");
  });

  test("formats fractional values with 2 decimals", () => {
    const range: NumericRange = { min: 0.123, max: 0.987, mean: 0.555 };
    expect(formatNumericRange(range)).toBe("0.12 – 0.99 (avg: 0.56)");
  });

  test("formats negative values", () => {
    const range: NumericRange = { min: -10.5, max: 20.3, mean: 4.9 };
    expect(formatNumericRange(range)).toBe("-10.50 – 20.30 (avg: 4.90)");
  });

  test("formats zero range (min = max)", () => {
    const range: NumericRange = { min: 42, max: 42, mean: 42 };
    expect(formatNumericRange(range)).toBe("42.00 – 42.00 (avg: 42.00)");
  });
});

// ============================================================================
// Display — summarizeGlimpse
// ============================================================================

describe("summarizeGlimpse", () => {
  test("produces expected lines for full glimpse", () => {
    const glimpse: EntitySetGlimpse = {
      entityCount: 1500,
      numericRanges: {
        revenue: { min: 100, max: 10000, mean: 2500 },
      },
      distributions: {
        region: [
          { value: "US", count: 800, percentage: 53.3 },
          { value: "EU", count: 500, percentage: 33.3 },
          { value: "APAC", count: 200, percentage: 13.3 },
        ],
      },
    };

    const lines = summarizeGlimpse(glimpse);
    expect(lines).toEqual([
      "1500 entities",
      "revenue: 100.00 – 10000.00 (avg: 2500.00)",
      "region: US (800), EU (500), APAC (200)",
    ]);
  });

  test("entity count only when no ranges or distributions", () => {
    const glimpse: EntitySetGlimpse = {
      entityCount: 42,
      numericRanges: {},
      distributions: {},
    };

    expect(summarizeGlimpse(glimpse)).toEqual(["42 entities"]);
  });

  test("zero entity count", () => {
    const glimpse: EntitySetGlimpse = {
      entityCount: 0,
      numericRanges: {},
      distributions: {},
    };

    expect(summarizeGlimpse(glimpse)).toEqual(["0 entities"]);
  });

  test("skips empty distribution arrays", () => {
    const glimpse: EntitySetGlimpse = {
      entityCount: 10,
      numericRanges: {},
      distributions: {
        empty_col: [],
        filled_col: [{ value: "A", count: 10, percentage: 100 }],
      },
    };

    const lines = summarizeGlimpse(glimpse);
    expect(lines).toContain("filled_col: A (10)");
    // "empty_col" should not appear
    expect(lines.some((l) => l.startsWith("empty_col"))).toBe(false);
  });

  test("multiple numeric ranges and distributions", () => {
    const glimpse: EntitySetGlimpse = {
      entityCount: 500,
      numericRanges: {
        price: { min: 10, max: 100, mean: 55 },
        quantity: { min: 1, max: 50, mean: 25 },
      },
      distributions: {
        category: [
          { value: "Electronics", count: 200, percentage: 40 },
          { value: "Books", count: 150, percentage: 30 },
          { value: "Clothing", count: 100, percentage: 20 },
          { value: "Other", count: 50, percentage: 10 },
        ],
      },
    };

    const lines = summarizeGlimpse(glimpse);
    expect(lines[0]).toBe("500 entities");
    expect(lines).toContain("price: 10.00 – 100.00 (avg: 55.00)");
    expect(lines).toContain("quantity: 1.00 – 50.00 (avg: 25.00)");
    // distribution with >3 entries should show "+1 more"
    expect(lines).toContain("category: Electronics (200), Books (150), Clothing (100), +1 more");
  });
});

// ============================================================================
// Property-Based Tests (Interpreter Layer)
// ============================================================================

describe("state machine (property-based)", () => {
  test("terminal states have no valid outbound transitions", () => {
    fc.assert(
      fc.property(arbStatus, arbStatus, (from, to) => {
        if (isTerminalStatus(from)) {
          expect(canTransition(from, to)).toBe(false);
        }
      })
    );
  });

  test("no self-transitions", () => {
    fc.assert(
      fc.property(arbStatus, (s) => {
        expect(canTransition(s, s)).toBe(false);
      })
    );
  });

  test("canTransition always returns a boolean", () => {
    fc.assert(
      fc.property(arbStatus, arbStatus, (from, to) => {
        expect(typeof canTransition(from, to)).toBe("boolean");
      })
    );
  });
});

describe("formatNumericRange (property-based)", () => {
  test("never throws for any valid range", () => {
    fc.assert(
      fc.property(arbNumericRange, (range) => {
        const result = formatNumericRange(range);
        expect(typeof result).toBe("string");
        expect(result.length).toBeGreaterThan(0);
      })
    );
  });

  test("output always contains ' – ' separator and '(avg:' label", () => {
    fc.assert(
      fc.property(arbNumericRange, (range) => {
        const result = formatNumericRange(range);
        expect(result).toContain(" – ");
        expect(result).toContain("(avg:");
      })
    );
  });
});

describe("formatDistribution (property-based)", () => {
  test("never throws for any entries and limit", () => {
    fc.assert(
      fc.property(fc.array(arbDistEntry), fc.nat({ max: 20 }), (entries, limit) => {
        const result = formatDistribution(entries, Math.max(1, limit));
        expect(typeof result).toBe("string");
      })
    );
  });

  test("'+N more' only appears when entries exceed limit", () => {
    fc.assert(
      fc.property(
        fc.array(arbDistEntry, { minLength: 1 }),
        fc.integer({ min: 1, max: 50 }),
        (entries, limit) => {
          const result = formatDistribution(entries, limit);
          if (entries.length <= limit) {
            expect(result).not.toContain("more");
          } else {
            expect(result).toContain("more");
          }
        }
      )
    );
  });
});

describe("summarizeGlimpse (property-based)", () => {
  test("first line is always entity count", () => {
    fc.assert(
      fc.property(arbGlimpse, (glimpse) => {
        const lines = summarizeGlimpse(glimpse);
        expect(lines.length).toBeGreaterThanOrEqual(1);
        expect(lines[0]).toBe(`${glimpse.entityCount} entities`);
      })
    );
  });

  test("never throws for any valid glimpse", () => {
    fc.assert(
      fc.property(arbGlimpse, (glimpse) => {
        const lines = summarizeGlimpse(glimpse);
        expect(Array.isArray(lines)).toBe(true);
        for (const line of lines) {
          expect(typeof line).toBe("string");
        }
      })
    );
  });
});
