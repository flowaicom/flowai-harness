import { describe, expect, test } from "bun:test";
import * as fc from "fast-check";

import {
  arbCostSummary,
  arbFinishReason,
  arbKVMetrics,
  arbLatencySummary,
  arbPhaseBreakdown,
  arbRetryEvent,
  arbRetryReason,
  arbStreamPart,
  arbTokenMetrics,
  arbTokenUsage,
  arbTokenUsageWithOptional,
  arbToolState,
  arbToolStatus,
  arbToolTiming,
} from "~/lib/test-utils/arbitraries";

import {
  avgLLMLatencyMs,
  avgToolLatencyMs,
  combineAllTokenUsage,
  combineKVMetrics,
  combineTokenMetrics,
  combineTokenUsage,
  costCacheHitRatePercent,
  isCustomPart,
  isDataCostSummaryPart,
  isDataFileRegisteredPart,
  isDataFlowUIPart,
  isDataLatencySummaryPart,
  isDataToolAgentPart,
  isErrorPart,
  isFinishPart,
  isKVMetricsZero,
  isReasoningPart,
  isStepStartPart,
  isTextPart,
  isTokenMetricsZero,
  isToolAgentPart,
  isToolInvocationPart,
  isToolProgressPart,
  kvMetricsZero,
  llmUtilization,
  outputTokensPerSecond,
  overheadMs,
  parseSSELine,
  retryCountByReason,
  tokenCacheHitRate,
  tokenCacheHitRatePercent,
  tokenMetricsZero,
  tokensPerLLMCall,
  tokensPerSecond,
  tokenUsageZero,
  toolUtilization,
} from "./stream-part";

describe("stream token algebra", () => {
  test("combineTokenUsage preserves cache read and cache creation totals", () => {
    const combined = combineTokenUsage(
      {
        ...tokenUsageZero,
        promptTokens: 100,
        completionTokens: 40,
        cacheReadInputTokens: 25,
        cacheCreationInputTokens: 10,
        totalTokens: 140,
      },
      {
        ...tokenUsageZero,
        promptTokens: 50,
        completionTokens: 20,
        cacheReadInputTokens: 5,
        cacheCreationInputTokens: 15,
        totalTokens: 70,
      }
    );

    expect(combined).toEqual({
      promptTokens: 150,
      completionTokens: 60,
      cacheReadInputTokens: 30,
      cacheCreationInputTokens: 25,
      totalTokens: 210,
    });
  });

  test("combineTokenMetrics keeps cache creation metrics", () => {
    const combined = combineTokenMetrics(
      {
        ...tokenMetricsZero,
        inputTokens: 100,
        outputTokens: 20,
        cachedTokens: 30,
        cacheCreationTokens: 10,
      },
      {
        ...tokenMetricsZero,
        inputTokens: 50,
        outputTokens: 10,
        cachedTokens: 5,
        cacheCreationTokens: 15,
      }
    );

    expect(combined).toEqual({
      inputTokens: 150,
      outputTokens: 30,
      cachedTokens: 35,
      cacheCreationTokens: 25,
    });
  });

  test("costCacheHitRatePercent only measures cache hits against prompt tokens", () => {
    expect(
      costCacheHitRatePercent({
        agents: [],
        totalPromptTokens: 200,
        totalCompletionTokens: 40,
        totalCacheReadInputTokens: 50,
        totalCacheCreationInputTokens: 120,
        totalTokens: 240,
      })
    ).toBe(25);
  });
});

// ============================================================================
// Property-Based Tests (Interpreter Layer)
//
// fc.property() is the description (program-as-value).
// fc.assert() is the interpreter that executes the description.
// One property per test. Every property is falsifiable.
// ============================================================================

// -- TokenUsage Monoid Laws --

describe("TokenUsage monoid laws", () => {
  test("left identity: combine(zero, a) = a", () => {
    fc.assert(
      fc.property(arbTokenUsage, (a) => {
        expect(combineTokenUsage(tokenUsageZero, a)).toEqual(a);
      })
    );
  });

  test("right identity: combine(a, zero) = a", () => {
    fc.assert(
      fc.property(arbTokenUsage, (a) => {
        expect(combineTokenUsage(a, tokenUsageZero)).toEqual(a);
      })
    );
  });

  test("associativity: combine(combine(a,b),c) = combine(a, combine(b,c))", () => {
    fc.assert(
      fc.property(arbTokenUsage, arbTokenUsage, arbTokenUsage, (a, b, c) => {
        expect(combineTokenUsage(combineTokenUsage(a, b), c)).toEqual(
          combineTokenUsage(a, combineTokenUsage(b, c))
        );
      })
    );
  });

  test("commutativity: combine(a,b) = combine(b,a)", () => {
    fc.assert(
      fc.property(arbTokenUsage, arbTokenUsage, (a, b) => {
        expect(combineTokenUsage(a, b)).toEqual(combineTokenUsage(b, a));
      })
    );
  });

  test("combine normalizes optional fields to numbers", () => {
    fc.assert(
      fc.property(arbTokenUsageWithOptional, arbTokenUsageWithOptional, (a, b) => {
        const result = combineTokenUsage(a, b);
        expect(typeof result.cacheReadInputTokens).toBe("number");
        expect(typeof result.cacheCreationInputTokens).toBe("number");
      })
    );
  });
});

// -- KVMetrics Monoid Laws --

describe("KVMetrics monoid laws", () => {
  test("left identity: combine(zero, a) = a", () => {
    fc.assert(
      fc.property(arbKVMetrics, (a) => {
        expect(combineKVMetrics(kvMetricsZero, a)).toEqual(a);
      })
    );
  });

  test("right identity: combine(a, zero) = a", () => {
    fc.assert(
      fc.property(arbKVMetrics, (a) => {
        expect(combineKVMetrics(a, kvMetricsZero)).toEqual(a);
      })
    );
  });

  test("associativity: combine(combine(a,b),c) = combine(a, combine(b,c))", () => {
    fc.assert(
      fc.property(arbKVMetrics, arbKVMetrics, arbKVMetrics, (a, b, c) => {
        expect(combineKVMetrics(combineKVMetrics(a, b), c)).toEqual(
          combineKVMetrics(a, combineKVMetrics(b, c))
        );
      })
    );
  });

  test("commutativity: combine(a,b) = combine(b,a)", () => {
    fc.assert(
      fc.property(arbKVMetrics, arbKVMetrics, (a, b) => {
        expect(combineKVMetrics(a, b)).toEqual(combineKVMetrics(b, a));
      })
    );
  });

  test("isKVMetricsZero characterizes the algebraic zero", () => {
    fc.assert(
      fc.property(arbKVMetrics, (a) => {
        const allZero =
          a.bytesWritten === 0 &&
          a.bytesRead === 0 &&
          a.kvDurationMs === 0 &&
          a.putCount === 0 &&
          a.getCount === 0;
        expect(isKVMetricsZero(a)).toBe(allZero);
      })
    );
  });
});

// -- TokenMetrics Monoid Laws --

describe("TokenMetrics monoid laws", () => {
  test("left identity: combine(zero, a) = a", () => {
    fc.assert(
      fc.property(arbTokenMetrics, (a) => {
        expect(combineTokenMetrics(tokenMetricsZero, a)).toEqual(a);
      })
    );
  });

  test("right identity: combine(a, zero) = a", () => {
    fc.assert(
      fc.property(arbTokenMetrics, (a) => {
        expect(combineTokenMetrics(a, tokenMetricsZero)).toEqual(a);
      })
    );
  });

  test("associativity: combine(combine(a,b),c) = combine(a, combine(b,c))", () => {
    fc.assert(
      fc.property(arbTokenMetrics, arbTokenMetrics, arbTokenMetrics, (a, b, c) => {
        expect(combineTokenMetrics(combineTokenMetrics(a, b), c)).toEqual(
          combineTokenMetrics(a, combineTokenMetrics(b, c))
        );
      })
    );
  });

  test("commutativity: combine(a,b) = combine(b,a)", () => {
    fc.assert(
      fc.property(arbTokenMetrics, arbTokenMetrics, (a, b) => {
        expect(combineTokenMetrics(a, b)).toEqual(combineTokenMetrics(b, a));
      })
    );
  });

  test("isTokenMetricsZero characterizes the algebraic zero", () => {
    fc.assert(
      fc.property(arbTokenMetrics, (a) => {
        const allZero =
          a.inputTokens === 0 &&
          a.outputTokens === 0 &&
          a.cachedTokens === 0 &&
          a.cacheCreationTokens === 0;
        expect(isTokenMetricsZero(a)).toBe(allZero);
      })
    );
  });
});

// -- combineAllTokenUsage Monoid Homomorphism --

describe("combineAllTokenUsage", () => {
  test("list concatenation homomorphism: f(a ++ b) = combine(f(a), f(b))", () => {
    fc.assert(
      fc.property(fc.array(arbTokenUsage), fc.array(arbTokenUsage), (as, bs) => {
        const whole = combineAllTokenUsage([...as, ...bs]);
        const parts = combineTokenUsage(combineAllTokenUsage(as), combineAllTokenUsage(bs));
        expect(whole).toEqual(parts);
      })
    );
  });
});

// -- Cache Hit Rate Bounds --

describe("tokenCacheHitRate bounds", () => {
  test("null iff inputTokens === 0", () => {
    fc.assert(
      fc.property(arbTokenMetrics, (m) => {
        const rate = tokenCacheHitRate(m);
        if (m.inputTokens === 0) {
          expect(rate).toBeNull();
        } else {
          expect(rate).not.toBeNull();
          expect(Number.isFinite(rate)).toBe(true);
        }
      })
    );
  });

  test("percent is exactly rate * 100", () => {
    fc.assert(
      fc.property(arbTokenMetrics, (m) => {
        const rate = tokenCacheHitRate(m);
        const percent = tokenCacheHitRatePercent(m);
        if (rate === null) {
          expect(percent).toBeNull();
        } else {
          expect(percent).toBe(rate * 100);
        }
      })
    );
  });
});

describe("costCacheHitRatePercent", () => {
  test("null iff totalPromptTokens === 0 or no cache reads", () => {
    fc.assert(
      fc.property(arbCostSummary, (cost) => {
        const result = costCacheHitRatePercent(cost);
        const shouldBeNull = cost.totalPromptTokens === 0 || !cost.totalCacheReadInputTokens;
        if (shouldBeNull) {
          expect(result).toBeNull();
        } else {
          expect(result).not.toBeNull();
          expect(Number.isFinite(result)).toBe(true);
        }
      })
    );
  });
});

// -- Derived Latency Metric Bounds --

describe("derived latency metric bounds", () => {
  test("overheadMs >= 0 for any latency summary", () => {
    fc.assert(
      fc.property(arbLatencySummary, (latency) => {
        expect(overheadMs(latency)).toBeGreaterThanOrEqual(0);
      })
    );
  });

  test("tokensPerSecond >= 0 when not null", () => {
    fc.assert(
      fc.property(arbLatencySummary, (latency) => {
        const tps = tokensPerSecond(latency);
        if (tps !== null) {
          expect(tps).toBeGreaterThanOrEqual(0);
        }
      })
    );
  });

  test("llmUtilization: null iff totalDurationMs === 0", () => {
    fc.assert(
      fc.property(arbLatencySummary, (latency) => {
        const util = llmUtilization(latency);
        if (latency.totalDurationMs === 0) {
          expect(util).toBeNull();
        } else {
          expect(util).not.toBeNull();
          expect(Number.isFinite(util)).toBe(true);
        }
      })
    );
  });

  test("toolUtilization: null iff totalDurationMs === 0", () => {
    fc.assert(
      fc.property(arbLatencySummary, (latency) => {
        const util = toolUtilization(latency);
        if (latency.totalDurationMs === 0) {
          expect(util).toBeNull();
        } else {
          expect(util).not.toBeNull();
          expect(Number.isFinite(util)).toBe(true);
        }
      })
    );
  });

  test("all derived metrics return finite numbers or null", () => {
    fc.assert(
      fc.property(arbLatencySummary, (latency) => {
        for (const fn of [
          tokensPerSecond,
          outputTokensPerSecond,
          llmUtilization,
          toolUtilization,
          tokensPerLLMCall,
          avgLLMLatencyMs,
          avgToolLatencyMs,
        ]) {
          const result = fn(latency);
          if (result !== null) {
            expect(Number.isFinite(result)).toBe(true);
          }
        }
      })
    );
  });
});

// -- Parse Robustness --

describe("parseSSELine robustness", () => {
  test("never throws for arbitrary Unicode strings", () => {
    fc.assert(
      fc.property(fc.string(), (line) => {
        const result = parseSSELine(line);
        expect(result === null || typeof result === "object").toBe(true);
      })
    );
  });

  test("returns null for lines not starting with 'data:'", () => {
    fc.assert(
      fc.property(
        fc.string().filter((s) => !s.trim().startsWith("data:")),
        (line) => {
          expect(parseSSELine(line)).toBeNull();
        }
      )
    );
  });
});

// -- Type Guard Mutual Exclusivity --

describe("StreamPart type guards", () => {
  test("exactly one of 14 guards returns true per part", () => {
    const guards = [
      isTextPart,
      isReasoningPart,
      isStepStartPart,
      isToolInvocationPart,
      isToolAgentPart,
      isToolProgressPart,
      isDataToolAgentPart,
      isDataFileRegisteredPart,
      isDataCostSummaryPart,
      isDataFlowUIPart,
      isDataLatencySummaryPart,
      isFinishPart,
      isErrorPart,
      isCustomPart,
    ];

    fc.assert(
      fc.property(arbStreamPart, (part) => {
        const matches = guards.filter((g) => g(part));
        expect(matches).toHaveLength(1);
      })
    );
  });
});

// -- retryCountByReason Consistency --

describe("retryCountByReason", () => {
  test("sum of all reason counts equals array length", () => {
    fc.assert(
      fc.property(fc.array(arbRetryEvent), (events) => {
        const counts = retryCountByReason(events);
        const sum = Object.values(counts).reduce((a, b) => a + b, 0);
        expect(sum).toBe(events.length);
      })
    );
  });
});
