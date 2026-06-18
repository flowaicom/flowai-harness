import { describe, expect, test } from "bun:test";

import {
  applyWorkspaceRuntimeScope,
  normalizeWorkspaceRuntimeScopeId,
} from "./workspace-runtime-scope";

describe("workspace runtime scope", () => {
  test("normalizes missing workspace IDs to default", () => {
    expect(normalizeWorkspaceRuntimeScopeId(null)).toBe("default");
    expect(normalizeWorkspaceRuntimeScopeId(undefined)).toBe("default");
    expect(normalizeWorkspaceRuntimeScopeId("customer-a")).toBe("customer-a");
  });

  test("sets storage namespace before API header", () => {
    const calls: string[] = [];
    const normalized = applyWorkspaceRuntimeScope(
      {
        setStorageNamespace: (workspaceId) => calls.push(`storage:${workspaceId}`),
        setWorkspaceHeader: (workspaceId) => calls.push(`header:${workspaceId}`),
      },
      "customer-b"
    );

    expect(normalized).toBe("customer-b");
    expect(calls).toEqual(["storage:customer-b", "header:customer-b"]);
  });

  test("does not require global workspace headers for explicit harness routing", () => {
    const calls: string[] = [];
    const normalized = applyWorkspaceRuntimeScope(
      {
        setStorageNamespace: (workspaceId) => calls.push(`storage:${workspaceId}`),
      },
      "customer-c"
    );

    expect(normalized).toBe("customer-c");
    expect(calls).toEqual(["storage:customer-c"]);
  });
});
