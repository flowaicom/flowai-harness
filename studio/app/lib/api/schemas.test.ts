import { describe, expect, test } from "bun:test";
import * as fc from "fast-check";
import { z } from "zod";

import {
  DataReadinessSchema,
  ImportEventSchema,
  ImportStageSchema,
  ResponseValidationError,
  validateResponse,
  WorkspaceListSchema,
  WorkspaceSchema,
} from "./schemas";

interface StudioContractFixture {
  readonly workspace: unknown;
  readonly workspaces: unknown;
  readonly importStage: unknown;
  readonly importEvents: readonly unknown[];
}

const prototypePollutionKeys = new Set(["__proto__", "constructor", "prototype"]);

const SimpleBoundarySchema = z
  .object({
    id: z.string(),
    nested: z
      .object({
        count: z.number(),
      })
      .optional(),
  })
  .passthrough();

function containsPrototypePollutionKey(value: unknown): boolean {
  if (value === null || typeof value !== "object") {
    return false;
  }

  if (Array.isArray(value)) {
    return value.some(containsPrototypePollutionKey);
  }

  return Object.entries(value as Record<string, unknown>).some(
    ([key, child]) => prototypePollutionKeys.has(key) || containsPrototypePollutionKey(child)
  );
}

async function loadStudioContractFixture(): Promise<StudioContractFixture> {
  const fixtureUrl = new URL("./contract-fixtures.json", import.meta.url);
  return JSON.parse(await Bun.file(fixtureUrl).text()) as StudioContractFixture;
}

describe("Studio backend contract fixtures", () => {
  test("workspace bundle fixtures validate and reject the old postgresql workspace type", async () => {
    const fixture = await loadStudioContractFixture();

    const workspace = WorkspaceSchema.parse(fixture.workspace);
    expect(workspace.databaseType).toBe("external");
    expect(workspace.bundle?.complete).toBe(true);
    expect(WorkspaceListSchema.safeParse(fixture.workspaces).success).toBe(true);

    const driftedWorkspace = {
      ...(fixture.workspace as Record<string, unknown>),
      databaseType: "postgresql",
    };
    expect(WorkspaceSchema.safeParse(driftedWorkspace).success).toBe(false);
  });

  test("import stage and stream event fixtures validate at the frontend boundary", async () => {
    const fixture = await loadStudioContractFixture();

    const importStage = ImportStageSchema.parse(fixture.importStage);
    expect(importStage.stage).toBe("completed");

    for (const event of fixture.importEvents) {
      expect(ImportEventSchema.safeParse(event).success).toBe(true);
    }

    expect(
      fixture.importEvents.some(
        (event) =>
          typeof event === "object" &&
          event !== null &&
          (event as { type?: string }).type === "profilingEvent"
      )
    ).toBe(true);
  });
});

describe("DataReadinessSchema", () => {
  test("accepts workspace readiness and rejects unknown readiness status", () => {
    const payload = {
      workspaceId: "customer-a",
      ready: true,
      status: "ready",
      sourceId: "target",
      importJobId: "import-1234",
      profileJobId: null,
      dataBundle: {
        status: "complete",
        complete: true,
        configuredRoles: ["target", "catalog"],
        missingRoles: [],
      },
      tableRowCounts: { sales_snapshot: 2 },
      targetTables: [{ name: "sales_snapshot", rowCount: 2 }],
      documents: { ingested: 1 },
      knowledge: { itemsExtracted: 2, itemIds: ["knowledge-1", "knowledge-2"] },
      catalogProfile: {
        summary: { columnsProfiled: 3 },
        profiledTables: ["sales_snapshot"],
      },
      generatedAt: "2026-04-10T00:00:00Z",
    };

    expect(DataReadinessSchema.safeParse(payload).success).toBe(true);
    expect(DataReadinessSchema.safeParse({ ...payload, status: "stale" }).success).toBe(false);
  });
});

describe("validateResponse", () => {
  test("returns validated data for valid input", () => {
    const payload = { id: "boundary-1", nested: { count: 1 } };
    const result = validateResponse(SimpleBoundarySchema, payload, "simple-boundary");
    expect(result.id).toBe("boundary-1");
    expect(result.nested?.count).toBe(1);
  });

  test("invalid data throws a typed validation error", () => {
    const bogus = { nested: { count: "one" } };
    expect(() => validateResponse(SimpleBoundarySchema, bogus, "simple-invalid")).toThrow(
      ResponseValidationError
    );
  });

  test("strips prototype-polluting passthrough keys before parsing", () => {
    const payload = JSON.parse(`{
      "id": "boundary-1",
      "__proto__": { "polluted": true },
      "constructor": { "prototype": { "polluted": true } },
      "prototype": { "polluted": true },
      "safeExtra": { "kept": true }
    }`);

    const result = validateResponse(SimpleBoundarySchema, payload, "prototype-pollution");

    expect(Object.hasOwn(result, "__proto__")).toBe(false);
    expect(Object.hasOwn(result, "constructor")).toBe(false);
    expect(Object.hasOwn(result, "prototype")).toBe(false);
    expect((result as Record<string, unknown>).safeExtra).toEqual({ kept: true });
    expect((Object.getPrototypeOf(result) as { polluted?: unknown } | null)?.polluted).toBe(
      undefined
    );
    expect(({} as { polluted?: unknown }).polluted).toBe(undefined);
  });

  test("strips nested prototype-polluting passthrough keys before parsing", () => {
    const payload = JSON.parse(`{
      "id": "boundary-1",
      "safeExtra": {
        "kept": true,
        "prototype": []
      },
      "extraList": [
        {
          "kept": 1,
          "constructor": { "prototype": { "polluted": true } }
        }
      ]
    }`);

    const result = validateResponse(SimpleBoundarySchema, payload, "nested-prototype-pollution");

    expect((result as Record<string, unknown>).safeExtra).toEqual({ kept: true });
    expect((result as Record<string, unknown>).extraList).toEqual([{ kept: 1 }]);
  });
});

describe("validateResponse (property-based)", () => {
  test("returns the original reference for safe already-valid objects", () => {
    fc.assert(
      fc.property(fc.record({ safe: fc.string(), count: fc.integer() }), (data) => {
        const result = validateResponse(z.any(), data, "pbt");
        expect(result).toBe(data);
      })
    );
  });

  test("passthrough schema preserves unknown fields", () => {
    const knownKeys = new Set(["id", "nested"]);
    const safeJsonValue = fc.jsonValue().filter((value) => !containsPrototypePollutionKey(value));

    fc.assert(
      fc.property(
        fc
          .string({ minLength: 1 })
          .filter((s) => !knownKeys.has(s) && !prototypePollutionKeys.has(s)),
        safeJsonValue,
        (extraKey, extraValue) => {
          const validPayload = {
            id: "boundary-1",
            [extraKey]: extraValue,
          };
          const result = validateResponse(SimpleBoundarySchema, validPayload, "pbt-boundary");
          expect((result as Record<string, unknown>)[extraKey]).toEqual(extraValue);
        }
      )
    );
  });
});
