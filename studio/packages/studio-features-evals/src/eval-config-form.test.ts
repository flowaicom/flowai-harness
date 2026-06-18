import { describe, expect, test } from "bun:test";
import { updateSharedScoreWeight } from "./eval-config-form";

type CanonicalScorerKey = "trajectory" | "planned_actions" | "executed_actions" | "final_response";

describe("eval config form scorer weights", () => {
  test("updates one scorer key without dropping the others", () => {
    const next = updateSharedScoreWeight<CanonicalScorerKey>(
      {
        trajectory: 1,
        planned_actions: 0.5,
        final_response: 0.25,
      },
      "executed_actions",
      0.75
    );

    expect(next).toEqual({
      trajectory: 1,
      planned_actions: 0.5,
      executed_actions: 0.75,
      final_response: 0.25,
    });
  });

  test("removes one scorer key without dropping the others", () => {
    const next = updateSharedScoreWeight<CanonicalScorerKey>(
      {
        trajectory: 1,
        planned_actions: 0.5,
        executed_actions: 0.75,
        final_response: 0.25,
      },
      "planned_actions",
      null
    );

    expect(next).toEqual({
      trajectory: 1,
      executed_actions: 0.75,
      final_response: 0.25,
    });
  });
});
