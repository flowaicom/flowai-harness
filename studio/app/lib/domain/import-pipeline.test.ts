import { describe, expect, test } from "bun:test";

import type { ImportEvent, ImportSummary } from "./data";
import { createImportSessionModel, reduceImportSessionEvent } from "./import-pipeline";

const summary: ImportSummary = {
  sourceRowCount: 30,
  tableRowCounts: { products: 10, orders: 20 },
  durationMs: 123,
};

describe("import pipeline reducer", () => {
  test("updates table counts without mutating the input session", () => {
    const session = createImportSessionModel("job-1", 1000);

    const result = reduceImportSessionEvent(session, {
      type: "tableLoaded",
      tableName: "products",
      rowCount: 10,
    });

    expect(session.tableCounts.size).toBe(0);
    expect(result.session.tableCounts.get("products")).toBe(10);
    expect(result.effects).toEqual([]);
  });

  test("counts each profiling terminal table once", () => {
    const session = createImportSessionModel("job-1", 1000);
    const tableCompleted: ImportEvent = {
      type: "profilingEvent",
      inner: {
        type: "tableCompleted",
        tableName: "products",
        summary: {
          tablesDiscovered: 1,
          columnsProfiled: 3,
          enumsExtracted: 0,
          relationshipsFound: 0,
          catalogItemsIndexed: 1,
          durationMs: 5,
        },
      },
    };

    const first = reduceImportSessionEvent(session, tableCompleted);
    const second = reduceImportSessionEvent(first.session, tableCompleted);

    expect(first.session.profilingCompleted).toBe(1);
    expect(second.session.profilingCompleted).toBe(1);
    expect(second.session.profilingTerminalTables).toEqual(["products"]);
  });

  test("describes import completion effects instead of performing IO", () => {
    const session = createImportSessionModel("job-1", 1000);

    const result = reduceImportSessionEvent(session, {
      type: "completed",
      summary,
    });

    expect(result.session.importStage).toEqual({ stage: "completed", summary });
    expect(result.effects).toEqual([{ type: "importComplete", tableCount: 2, totalRowCount: 30 }]);
  });

  test("error is terminal for the session and keeps the reducer pure", () => {
    const session = createImportSessionModel("job-1", 1000);

    const result = reduceImportSessionEvent(session, {
      type: "error",
      message: "bad upload",
    });

    expect(result.session.isRunning).toBe(false);
    expect(result.session.importStage).toEqual({ stage: "failed", error: "bad upload" });
    expect(result.session.error).toBe("bad upload");
    expect(session.isRunning).toBe(true);
  });
});
