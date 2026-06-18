import { describe, expect, test } from "bun:test";

import { backendLatencyToTrace } from "./latency-bridge";

describe("backendLatencyToTrace", () => {
  test("preserves token metrics and product set size from backend summaries", () => {
    const trace = backendLatencyToTrace(
      {
        totalDurationMs: 1_200,
        phases: {
          llmTimeMs: 800,
          toolTimeMs: 250,
          llmCalls: 2,
        },
        toolTimings: [
          {
            toolName: "buildPlan",
            toolCallId: "tool-1",
            durationMs: 250,
            status: "completed",
          },
        ],
        tokenMetrics: {
          inputTokens: 1_000,
          outputTokens: 250,
          cachedTokens: 400,
          cacheCreationTokens: 80,
        },
        productSetSize: 26,
        planPayloadBytes: 2_048,
        retryCount: 1,
        hadTimeout: false,
      },
      {
        startedAt: 10_000,
        timeToFirstChunk: 120,
        streamingDuration: 900,
      },
      "trace-1"
    );

    expect(trace.traceId).toBe("trace-1");
    expect(trace.startedAt).toBe(10_000);
    expect(trace.completedAt).toBe(11_200);
    expect(trace.tokens).toEqual({
      inputTokens: 1_000,
      outputTokens: 250,
      cachedTokens: 400,
      cacheCreationTokens: 80,
    });
    expect(trace.productSetSize).toBe(26);
    expect(trace.planPayloadBytes).toBe(2_048);
  });
});
