import { describe, expect, test } from "bun:test";
import { buildEvalCaseRoute, getAdjacentEvalCaseId, getEvalCaseSampleNavIndex } from "./index";

describe("eval-case-nav-model", () => {
  test("derives sample navigation indices from arrow keys", () => {
    expect(getEvalCaseSampleNavIndex("ArrowLeft", 1, 3)).toBe(0);
    expect(getEvalCaseSampleNavIndex("ArrowRight", 1, 3)).toBe(2);
    expect(getEvalCaseSampleNavIndex("ArrowLeft", 0, 3)).toBeNull();
  });

  test("derives adjacent case ids from j/k navigation", () => {
    expect(getAdjacentEvalCaseId("j", ["tc-a", "tc-b", "tc-c"], "tc-a")).toBe("tc-b");
    expect(getAdjacentEvalCaseId("K", ["tc-a", "tc-b", "tc-c"], "tc-c")).toBe("tc-b");
    expect(getAdjacentEvalCaseId("j", ["tc-a"], "tc-a")).toBeNull();
  });

  test("builds eval case drill-down routes", () => {
    expect(buildEvalCaseRoute("eval-1", "tc-b", 2)).toBe("/evals/eval-1/cases/tc-b?sample=2");
  });
});
