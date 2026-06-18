import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import type { SampleResult } from "~/lib/domain/eval";
import { TrajectoryThread } from "./trajectory-thread";

const sample: SampleResult = {
  sampleIndex: 1,
  passed: true,
  aggregateScore: 0.42,
  scores: [
    {
      scorerName: "custom_catalog_scorer",
      score: 0.42,
      details: {
        reason: "Matched catalog answer",
        nested: { table: "products" },
      },
    },
  ],
  actualTrajectory: [],
  durationMs: 1234,
  tokenUsage: {
    inputTokens: 10,
    outputTokens: 5,
    cachedTokens: 0,
    cacheCreationTokens: 0,
  },
  error: null,
  retryCount: 0,
};

describe("TrajectoryThread scorer rendering", () => {
  test("renders unknown scorer names and details generically", () => {
    const html = renderToStaticMarkup(
      <TrajectoryThread input="Find the answer" steps={[]} sample={sample} />
    );

    expect(html).toContain("custom_catalog_scorer");
    expect(html).toContain("0.420");
    expect(html).toContain("reason");
    expect(html).toContain("Matched catalog answer");
    expect(html).toContain("nested");
    expect(html).toContain("products");
  });

  test("renders current planner action scorer details without legacy product diffs", () => {
    const plannerSample: SampleResult = {
      ...sample,
      passed: false,
      aggregateScore: 0.5,
      scores: [
        {
          scorerName: "planned_actions",
          score: 0,
          details: {
            actions: [
              {
                signature:
                  'price_change:{"newPrice":6.49,"productId":"p-001","reason":"Wholesale demand is strong."}',
                status: "missing",
              },
              {
                signature:
                  'price_change:{"newPrice":6.49,"productId":"p-001","reason":"Wholesale demand is strong"}',
                status: "extra",
              },
            ],
            issues: ["Missing action", "Extra action"],
            pass: false,
            payloadMatch: "exact",
            summary: {
              exactCount: 0,
              extraCount: 1,
              missingCount: 1,
              passRate: 0,
              totalActual: 1,
              totalExpected: 1,
            },
          },
        },
      ],
    };

    const html = renderToStaticMarkup(
      <TrajectoryThread input="Create a plan" steps={[]} sample={plannerSample} />
    );

    expect(html).toContain("0%");
    expect(html).toContain("FAIL");
    expect(html).toContain("price_change");
    expect(html).toContain("Missing action");
    expect(html).toContain("Extra action");
  });
});
