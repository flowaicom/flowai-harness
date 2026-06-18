import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import type { SampleResult } from "~/lib/domain/eval";
import { SampleTabBar } from "./sample-tab-bar";

const sample: SampleResult = {
  sampleIndex: 0,
  passed: true,
  aggregateScore: 1,
  scores: [{ scorerName: "trajectory", score: 1 }],
  actualTrajectory: ["call_agent"],
  durationMs: 10,
  tokenUsage: {
    inputTokens: 0,
    outputTokens: 0,
    cachedTokens: 0,
    cacheCreationTokens: 0,
  },
  error: null,
  retryCount: 0,
};

describe("SampleTabBar", () => {
  test("does not render deferred run-chat or fork controls", () => {
    const html = renderToStaticMarkup(
      <SampleTabBar samples={[sample]} selectedIndex={0} onSelect={() => {}} />
    );

    expect(html).toContain("Sample 1");
    expect(html).not.toContain("Run Chat");
    expect(html).not.toContain("Fork");
  });
});
