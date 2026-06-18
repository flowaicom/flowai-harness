import { describe, expect, test } from "bun:test";
import { formatEvalScoreBreakdown, getEvalScoreIntensityColor } from "./index";

describe("eval-matrix-model", () => {
  test("returns queued color for pending samples and colored hsl for completed ones", () => {
    expect(
      getEvalScoreIntensityColor(undefined, {
        passThreshold: 0.7,
        queuedColor: "gray",
        extractSampleScore: () => 0,
      })
    ).toBe("gray");

    expect(
      getEvalScoreIntensityColor(
        { passed: true, scores: [{ scorerName: "trajectory", score: 0.9 }] },
        {
          passThreshold: 0.7,
          queuedColor: "gray",
          extractSampleScore: (scores) => scores[0]?.score ?? 0,
        }
      )
    ).toContain("hsl(145");
  });

  test("formats parsed scorer breakdowns", () => {
    const scores = [{ scorerName: "trajectory", score: 0.8 }];
    expect(
      formatEvalScoreBreakdown(scores, {
        parseScorerDetails: () => ({
          kind: "trajectory",
          fBeta: { fScore: 0.8123, precision: 0.7, recall: 0.9 },
        }),
      })
    ).toBe("F-β: 0.812 (P=0.70, R=0.90)");

    expect(formatEvalScoreBreakdown([{ scorerName: "generic", score: 0.55 }])).toBe(
      "generic: 0.550"
    );
  });
});
