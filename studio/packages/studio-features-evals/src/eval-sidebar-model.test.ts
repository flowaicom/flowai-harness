import { describe, expect, test } from "bun:test";
import { evalSidebarRunsFixture } from "@studio/core/test-fixtures/shared-contract-fixtures";
import {
  type EvalSidebarRunLike,
  filterEvalSidebarRuns,
  getEvalModeLabel,
  getShortEvalModelLabel,
  matchesEvalSidebarStatusFilter,
} from "./index";

const runs: readonly EvalSidebarRunLike[] = evalSidebarRunsFixture;

describe("eval-sidebar-model", () => {
  test("matches status filters against run state and score", () => {
    expect(matchesEvalSidebarStatusFilter(runs[0], "running")).toBe(true);
    expect(matchesEvalSidebarStatusFilter(runs[1], "passed")).toBe(true);
    expect(matchesEvalSidebarStatusFilter(runs[2], "failed")).toBe(true);
  });

  test("formats eval mode and model labels", () => {
    expect(getEvalModeLabel("testCaseBuilder")).toBe("Test Case Builder");
    expect(getEvalModeLabel("customMode")).toBe("CustomMode");
    expect(getShortEvalModelLabel("claude-opus-4-6")).toBe("opus");
    expect(getShortEvalModelLabel("custom-model-name")).toBe("custom-model");
  });

  test("filters runs by status and search query", () => {
    expect(filterEvalSidebarRuns(runs, "", "all").map((run) => run.id)).toEqual([
      "eval-running",
      "eval-pass",
      "eval-fail",
    ]);
    expect(filterEvalSidebarRuns(runs, "builder", "passed").map((run) => run.id)).toEqual([
      "eval-pass",
    ]);
  });
});
