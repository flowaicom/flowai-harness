import { describe, expect, test } from "bun:test";
import type { SharedTrajectoryMode } from "./domain";

describe("shared tests trajectory mode contract", () => {
  test("matches the harness Studio trajectory modes", () => {
    const modes = [
      "strict",
      "unordered",
      "subset",
      "superset",
      "subsequence",
    ] satisfies readonly SharedTrajectoryMode[];

    expect(modes).toEqual(["strict", "unordered", "subset", "superset", "subsequence"]);
  });
});
