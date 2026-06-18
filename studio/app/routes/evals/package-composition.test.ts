import { describe, expect, test } from "bun:test";
import { readFile } from "node:fs/promises";

describe("Eval package-backed route composition", () => {
  test("eval detail route renders the shared eval detail package page", async () => {
    const source = await readFile("app/routes/evals/$evalId.tsx", "utf8");

    expect(source).toContain("@studio/features-evals");
    expect(source).toContain("SharedEvalDetailPage");
  });

  test("eval case drilldown route renders the shared eval case thread package view", async () => {
    const source = await readFile("app/routes/evals/$evalId.cases.$testCaseId.tsx", "utf8");

    expect(source).toContain("@studio/features-evals");
    expect(source).toContain("SharedEvalCaseThreadView");
  });

  test("eval case drilldown preserves host-owned persisted trace actions", async () => {
    const source = await readFile("app/routes/evals/$evalId.cases.$testCaseId.tsx", "utf8");

    expect(source).toContain("renderSampleAccessory");
    expect(source).toContain("Open Trace");
  });
});
