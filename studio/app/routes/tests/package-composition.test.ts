import { describe, expect, test } from "bun:test";
import { readFile } from "node:fs/promises";

describe("Tests package-backed route composition", () => {
  test("test detail route renders the shared tests detail package page", async () => {
    const source = await readFile("app/routes/tests/$testCaseId.tsx", "utf8");

    expect(source).toContain("@studio/features-tests");
    expect(source).toContain("SharedTestCaseDetailPage");
  });
});
