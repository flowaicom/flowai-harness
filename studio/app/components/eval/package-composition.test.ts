import { describe, expect, test } from "bun:test";
import { readFile } from "node:fs/promises";

describe("Eval package-backed component composition", () => {
  test("EvalConfigForm delegates rendering to the shared eval feature package", async () => {
    const source = await readFile("app/components/eval/eval-config-form.tsx", "utf8");

    expect(source).toContain("@studio/features-evals");
    expect(source).toContain("SharedEvalConfigForm");
  });
});
