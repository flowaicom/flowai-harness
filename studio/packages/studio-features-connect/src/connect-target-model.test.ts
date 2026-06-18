import { describe, expect, test } from "bun:test";
import {
  buildConnectScopeRoute,
  buildConnectTargetRoute,
  CONNECT_TARGET_WORKSPACE,
  deriveConnectScope,
  getConnectEffectiveSourceId,
} from "./connect-target-model";

describe("connect target model", () => {
  test("builds source and workspace routes", () => {
    expect(
      buildConnectTargetRoute("/connect/import", { target: "source", sourceId: "source-123" })
    ).toBe("/connect/import?sourceId=source-123");
    expect(
      buildConnectTargetRoute("/connect/import", {
        target: "workspace",
        workspaceTargetId: "workspace:ws-a:target",
      })
    ).toBe(`/connect/import?target=${CONNECT_TARGET_WORKSPACE}`);
  });

  test("builds scope routes from workspace target keys", () => {
    expect(
      buildConnectScopeRoute("/connect/profiling", "workspace:ws-a:target", "workspace:ws-a:target")
    ).toBe(`/connect/profiling?target=${CONNECT_TARGET_WORKSPACE}`);
    expect(
      buildConnectScopeRoute("/connect/profiling", "source-xyz", "workspace:ws-a:target")
    ).toBe("/connect/profiling?sourceId=source-xyz");
  });

  test("derives shared scope and effective sources", () => {
    expect(deriveConnectScope("ws-a", "source-1")).toEqual({
      type: "source",
      workspaceId: "ws-a",
      sourceId: "source-1",
    });
    expect(deriveConnectScope("ws-a", null)).toEqual({
      type: "workspace",
      workspaceId: "ws-a",
    });
    expect(
      getConnectEffectiveSourceId(null, [{ id: "source-1" } as const, { id: "source-2" } as const])
    ).toBeNull();
    expect(getConnectEffectiveSourceId(null, [{ id: "source-1" } as const])).toBe("source-1");
  });
});
