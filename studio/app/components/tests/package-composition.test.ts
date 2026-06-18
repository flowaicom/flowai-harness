import { describe, expect, test } from "bun:test";
import { readFile } from "node:fs/promises";

describe("Tests package-backed component composition", () => {
  test("TestCaseForm delegates rendering to the shared tests feature package", async () => {
    const source = await readFile("app/components/tests/test-case-form.tsx", "utf8");

    expect(source).toContain("@studio/features-tests");
    expect(source).toContain("SharedTestCaseForm");
  });

  test("TestSidebar delegates reusable rendering to the shared tests feature package", async () => {
    const source = await readFile("app/components/tests/test-sidebar.tsx", "utf8");

    expect(source).toContain("@studio/features-tests");
    expect(source).toContain("TestSidebarPane");
  });
});
