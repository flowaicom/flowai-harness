import { describe, expect, test } from "bun:test";
import { readFile } from "node:fs/promises";

const packageBackedRoutes = [
  { file: "app/routes/connect/discovery.tsx", component: "ConnectDiscoveryPage" },
  { file: "app/routes/connect/search.tsx", component: "ConnectSearchPage" },
  { file: "app/routes/connect/tools.tsx", component: "ConnectToolsPage" },
  { file: "app/routes/connect/knowledge.tsx", component: "ConnectKnowledgePage" },
  { file: "app/routes/connect/profiling.tsx", component: "ConnectProfilingPage" },
  { file: "app/routes/connect/import.tsx", component: "ConnectImportPage" },
] as const;

describe("Connect package-backed route composition", () => {
  for (const route of packageBackedRoutes) {
    test(`${route.file} renders the shared ${route.component}`, async () => {
      const source = await readFile(route.file, "utf8");

      expect(source).toContain("@studio/features-connect");
      expect(source).toContain(route.component);
    });
  }

  test("discovery route passes stable store adapter callbacks into the shared page", async () => {
    const source = await readFile("app/routes/connect/discovery.tsx", "utf8");

    expect(source).toContain("const handleSetTables = useCallback");
    expect(source).toContain("const handleSetTableDetail = useCallback");
    expect(source).toContain("setTables={handleSetTables}");
    expect(source).toContain("setTableDetail={handleSetTableDetail}");
  });
});
