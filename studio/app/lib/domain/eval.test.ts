import { describe, expect, test } from "bun:test";

import { parseScorerDetails } from "./eval";

describe("parseScorerDetails", () => {
  test("recognizes current planned action scorer details", () => {
    const parsed = parseScorerDetails({
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
    });

    expect(parsed.kind).toBe("actionMatch");
    if (parsed.kind !== "actionMatch") return;
    expect(parsed.actionMatch.actions).toHaveLength(2);
    expect(parsed.actionMatch.actions[0]?.status).toBe("missing");
  });

  test("recognizes final response scorer details", () => {
    const parsed = parseScorerDetails({
      scorerName: "final_response",
      score: 1,
      details: {
        passed: true,
        score: 1,
        responseScorers: [
          {
            id: "mentions_email",
            method: "contains",
            passed: true,
            reason: "The final response contained the required text.",
          },
        ],
      },
    });

    expect(parsed.kind).toBe("finalResponse");
    if (parsed.kind !== "finalResponse") return;
    expect(parsed.result.responseScorers?.[0]?.id).toBe("mentions_email");
    expect(parsed.result.responseScorers?.[0]?.reason).toBe(
      "The final response contained the required text."
    );
  });
});
