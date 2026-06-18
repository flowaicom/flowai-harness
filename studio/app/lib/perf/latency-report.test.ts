import { describe, expect, test } from "bun:test";

import { generateLatencyReport, type RequestTrace } from "./latency-report";

describe("generateLatencyReport", () => {
  test("retains cache creation token statistics", () => {
    const traces: RequestTrace[] = [
      {
        traceId: "t-1",
        startedAt: 0,
        completedAt: 100,
        totalDuration: 100,
        phases: { waiting: 20, streaming: 40, toolExecution: 0, llmThinking: 0 },
        toolRecords: [],
        totalToolTime: 0,
        wallClockToolTime: 0,
        retryCount: 0,
        hadTimeout: false,
        tokens: {
          inputTokens: 100,
          outputTokens: 20,
          cachedTokens: 30,
          cacheCreationTokens: 10,
        },
      },
      {
        traceId: "t-2",
        startedAt: 0,
        completedAt: 140,
        totalDuration: 140,
        phases: { waiting: 30, streaming: 50, toolExecution: 0, llmThinking: 0 },
        toolRecords: [],
        totalToolTime: 0,
        wallClockToolTime: 0,
        retryCount: 0,
        hadTimeout: false,
        tokens: {
          inputTokens: 120,
          outputTokens: 25,
          cachedTokens: 40,
          cacheCreationTokens: 20,
        },
      },
    ];

    const report = generateLatencyReport(traces);
    expect(report?.tokenStats?.cacheCreation.mean).toBe(15);
    expect(report?.tokenStats?.cacheCreation.max).toBe(20);
  });
});
