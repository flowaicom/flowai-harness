import { describe, expect, test } from "bun:test";
import {
  deriveEvalCaseContentView,
  type EvalCaseThreadForkLike,
  type EvalCaseViewMode,
  getSelectedEvalCaseForkId,
  resolveEffectiveEvalCaseViewMode,
} from "./index";

const forks: readonly EvalCaseThreadForkLike[] = [
  { id: "fork-1", threadId: "thread-fork-1", forkAtMessageIndex: 4 },
];

describe("eval-case-thread-model", () => {
  test("falls back to trajectory when the selected fork disappears", () => {
    const mode: EvalCaseViewMode = { kind: "fork", forkId: "missing" };
    expect(resolveEffectiveEvalCaseViewMode(mode, new Set(["fork-1"]))).toEqual({
      kind: "trajectory",
    });
  });

  test("derives trajectory and chat content views", () => {
    expect(
      deriveEvalCaseContentView({ kind: "trajectory" }, { threadId: "thread-sample" }, forks)
    ).toEqual({
      view: "trajectory",
      sample: { threadId: "thread-sample" },
    });

    expect(
      deriveEvalCaseContentView({ kind: "sampleChat" }, { threadId: "thread-sample" }, forks)
    ).toEqual({
      view: "chat",
      threadId: "thread-sample",
    });

    expect(deriveEvalCaseContentView({ kind: "fork", forkId: "fork-1" }, undefined, forks)).toEqual(
      {
        view: "chat",
        threadId: "thread-fork-1",
        forkAtIndex: 4,
      }
    );
  });

  test("returns the selected fork id for fork view only", () => {
    expect(getSelectedEvalCaseForkId({ kind: "fork", forkId: "fork-1" })).toBe("fork-1");
    expect(getSelectedEvalCaseForkId({ kind: "trajectory" })).toBeNull();
  });
});
