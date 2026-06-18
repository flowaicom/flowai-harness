import { describe, expect, test } from "bun:test";
import { err, ok } from "@studio/core/domain/result";
import {
  connectDiscoveryApiResultFixture,
  connectDiscoveryErrorResultFixture,
  connectDiscoveryTablesFixture,
} from "@studio/core/test-fixtures/shared-contract-fixtures";
import {
  buildConnectTableExplorePrompt,
  loadConnectDiscoveryTables,
  summarizeConnectTableColumns,
} from "./connect-discovery-model";

describe("connect discovery model", () => {
  test("returns idle when no target is selected", async () => {
    const result = await loadConnectDiscoveryTables({
      hasTarget: false,
      loadTables: async () => ok([]),
    });

    expect(result).toEqual({ kind: "idle" });
  });

  test("returns aborted when the request signal is already aborted", async () => {
    const controller = new AbortController();
    controller.abort();

    const result = await loadConnectDiscoveryTables({
      hasTarget: true,
      signal: controller.signal,
      loadTables: async () => ok([]),
    });

    expect(result).toEqual({ kind: "aborted" });
  });

  test("returns success with loaded tables", async () => {
    const result = await loadConnectDiscoveryTables({
      hasTarget: true,
      loadTables: async () => ok(connectDiscoveryApiResultFixture.value),
    });

    expect(result).toEqual({
      kind: "success",
      tables: connectDiscoveryTablesFixture,
    });
  });

  test("returns aborted when the signal changes during the request", async () => {
    const controller = new AbortController();

    const result = await loadConnectDiscoveryTables({
      hasTarget: true,
      signal: controller.signal,
      loadTables: async () => {
        controller.abort();
        return ok([]);
      },
    });

    expect(result).toEqual({ kind: "aborted" });
  });

  test("surfaces a typed error outcome", async () => {
    const result = await loadConnectDiscoveryTables({
      hasTarget: true,
      loadTables: async () => err(connectDiscoveryErrorResultFixture.error),
    });

    expect(result).toEqual({
      kind: "error",
      message: "boom",
    });
  });

  test("summarizes table columns with an overflow suffix", () => {
    expect(
      summarizeConnectTableColumns({
        ...connectDiscoveryTablesFixture[0],
        totalColumnCount: 14,
      })
    ).toBe("id, customer_id, status and 2 more");
  });

  test("builds a table exploration prompt from table metadata", () => {
    expect(buildConnectTableExplorePrompt(connectDiscoveryTablesFixture[0])).toBe(
      "Describe the table public.orders (columns: id, customer_id, status). What kinds of queries or analyses can I run on it?"
    );
  });
});
