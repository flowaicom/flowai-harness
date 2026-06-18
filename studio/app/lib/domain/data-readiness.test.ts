import { describe, expect, test } from "bun:test";
import * as fc from "fast-check";

import type { DataReadiness } from "./data";
import {
  dataReadinessCaption,
  dataReadinessGeneratedAtLabel,
  selectDataReadinessForTestSelection,
  summarizeDataReadiness,
} from "./data-readiness";

function readinessFixture(overrides: Partial<DataReadiness> = {}): DataReadiness {
  return {
    workspaceId: "customer-a",
    ready: true,
    status: "ready",
    sourceId: "target",
    importJobId: "import-1234",
    profileJobId: null,
    dataBundle: { status: "complete", complete: true },
    tableRowCounts: { sales: 2 },
    targetTables: [{ name: "sales", rowCount: 2 }],
    documents: { ingested: 1 },
    knowledge: { itemsExtracted: 3, itemIds: ["k1", "k2", "k3"] },
    catalogProfile: { summary: {}, profiledTables: ["sales"] },
    generatedAt: "2026-04-10T10:00:00Z",
    ...overrides,
  };
}

describe("data readiness description", () => {
  test("summarizes the readiness metrics used by Studio cards", () => {
    expect(
      summarizeDataReadiness(
        readinessFixture({
          tableRowCounts: { a: 2, b: 3 },
          targetTables: [
            { name: "a", rowCount: 2 },
            { name: "b", rowCount: 3 },
          ],
          documents: { ingested: 4 },
          knowledge: { itemsExtracted: 5, itemIds: ["k1", "k2"] },
          catalogProfile: { summary: {}, profiledTables: ["a", "b"] },
        })
      )
    ).toEqual({
      tableCount: 2,
      totalRows: 5,
      documentCount: 4,
      knowledgeCount: 5,
      profiledTableCount: 2,
      bundleStatus: "complete",
      statusLabel: "Ready",
    });
  });

  test("captions distinguish current workspace context from bound artifacts", () => {
    const readiness = readinessFixture();

    expect(dataReadinessCaption(readiness, "current")).toContain("This workspace uses");
    expect(dataReadinessCaption(readiness, "bound")).toContain("This artifact was created with");
    expect(dataReadinessCaption(readinessFixture({ ready: false, status: "empty" }))).toContain(
      "has no ingested data bundle yet"
    );
  });

  test("invalid generatedAt values are displayed as-is", () => {
    const readiness = readinessFixture({ generatedAt: "not-a-date" });

    expect(dataReadinessGeneratedAtLabel(readiness)).toBe("not-a-date");
  });

  test("selects readiness only from explicit test case ids", () => {
    const selected = readinessFixture({ workspaceId: "customer-selected" });
    const unrelated = readinessFixture({ workspaceId: "customer-unrelated" });

    expect(
      selectDataReadinessForTestSelection({
        testCases: [
          { id: "tc-a", dataReadiness: unrelated },
          { id: "tc-b", dataReadiness: selected },
        ],
        selectedTestCaseIds: ["tc-b"],
      })
    ).toMatchObject({
      readiness: selected,
      status: "ready",
      selectedCaseCount: 1,
      workspaceIds: ["customer-selected"],
    });
  });

  test("resolves uploaded test-case set ids before selecting readiness", () => {
    const readiness = readinessFixture({ workspaceId: "set-workspace" });

    expect(
      selectDataReadinessForTestSelection({
        testCases: [
          { id: "tc-a", dataReadiness: null },
          { id: "tc-b", dataReadiness: readiness },
        ],
        testCaseSets: [{ id: "set-a", testCases: [{ id: "tc-b" }] }],
        selectedTestCaseSetId: "set-a",
      })
    ).toMatchObject({
      readiness,
      status: "ready",
      selectedCaseCount: 1,
      workspaceIds: ["set-workspace"],
    });
  });

  test("refuses mixed workspace or snapshot selections", () => {
    const selection = selectDataReadinessForTestSelection({
      testCases: [
        { id: "tc-a", dataReadiness: readinessFixture({ workspaceId: "customer-a" }) },
        { id: "tc-b", dataReadiness: readinessFixture({ workspaceId: "customer-b" }) },
      ],
      selectedTestCaseIds: ["tc-a", "tc-b"],
    });

    expect(selection.readiness).toBeNull();
    expect(selection.status).toBe("mixed");
    expect(selection.workspaceIds).toEqual(["customer-a", "customer-b"]);
  });

  test("does not fall back to unrelated cases when nothing is selected", () => {
    const readiness = readinessFixture();

    expect(
      selectDataReadinessForTestSelection({
        testCases: [{ id: "tc-a", dataReadiness: readiness }],
        selectedTestCaseIds: [],
      })
    ).toMatchObject({
      readiness: null,
      status: "noSelection",
      selectedCaseCount: 0,
    });
  });

  test("fails closed when selected test case ids are not loaded", () => {
    const readiness = readinessFixture();

    expect(
      selectDataReadinessForTestSelection({
        testCases: [{ id: "tc-a", dataReadiness: readiness }],
        selectedTestCaseIds: ["tc-a", "tc-missing"],
      })
    ).toMatchObject({
      readiness: null,
      status: "noMatchingCases",
      selectedCaseCount: 1,
      workspaceIds: [],
    });
  });

  test("fails closed when selected test-case sets are not loaded", () => {
    expect(
      selectDataReadinessForTestSelection({
        testCases: [],
        testCaseSets: [],
        selectedTestCaseSetId: "set-missing",
      })
    ).toMatchObject({
      readiness: null,
      status: "noMatchingCases",
      selectedCaseCount: 0,
    });
  });

  test("fails closed when a selection mixes bound and unbound test cases", () => {
    const readiness = readinessFixture();

    expect(
      selectDataReadinessForTestSelection({
        testCases: [{ id: "tc-a", dataReadiness: readiness }, { id: "tc-b" }],
        selectedTestCaseIds: ["tc-a", "tc-b"],
      })
    ).toMatchObject({
      readiness: null,
      status: "partiallyUnbound",
      selectedCaseCount: 2,
      workspaceIds: ["customer-a"],
    });
  });

  test("summary row total is the sum of table row counts", () => {
    fc.assert(
      fc.property(
        fc.array(
          fc.tuple(
            fc
              .string({ minLength: 1, maxLength: 24 })
              .filter((key) => !["__proto__", "constructor", "prototype"].includes(key)),
            fc.integer({ min: 0, max: 1_000_000 })
          ),
          { maxLength: 20 }
        ),
        (entries) => {
          const tableRowCounts = Object.fromEntries(entries);
          const expectedTotal = Object.values(tableRowCounts).reduce(
            (total, rowCount) => total + rowCount,
            0
          );

          expect(summarizeDataReadiness(readinessFixture({ tableRowCounts })).totalRows).toBe(
            expectedTotal
          );
        }
      )
    );
  });
});
