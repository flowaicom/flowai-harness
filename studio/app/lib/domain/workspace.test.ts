import { describe, expect, test } from "bun:test";

import type { Workspace, WorkspaceDatabase, WorkspaceRole } from "./workspace";
import {
  missingBundleRoleList,
  missingBundleRolesLabel,
  workspaceBundleComplete,
  workspaceBundleLabel,
  workspaceContextFromIds,
} from "./workspace";

function db(role: WorkspaceRole): WorkspaceDatabase {
  return {
    id: `${role}-db`,
    workspaceId: "customer-a",
    role,
    displayName: role,
    databaseUrl: `sqlite:${role}.db`,
    createdAt: "2026-04-10T00:00:00Z",
  };
}

function workspace(databases: WorkspaceDatabase[]): Workspace {
  return {
    id: "customer-a",
    displayName: "Customer A",
    createdAt: "2026-04-10T00:00:00Z",
    databases,
  };
}

describe("workspace bundle selectors", () => {
  test("derives completeness from database roles when bundle is absent", () => {
    const complete = workspace([db("target"), db("catalog"), db("embeddings"), db("workspace")]);

    expect(workspaceBundleComplete(complete)).toBe(true);
    expect(workspaceBundleLabel(complete)).toBe("full");
    expect(missingBundleRoleList(complete)).toEqual([]);
  });

  test("reports missing roles without mutating workspace input", () => {
    const partial = workspace([db("target"), db("catalog")]);

    expect(workspaceBundleComplete(partial)).toBe(false);
    expect(workspaceBundleLabel(partial)).toBe("2 missing");
    expect(missingBundleRoleList(partial)).toEqual(["embeddings", "workspace"]);
    expect(missingBundleRolesLabel(partial)).toBe("Embeddings, Workspace");
    expect(partial.databases.map((database) => database.role)).toEqual(["target", "catalog"]);
  });

  test("trusts backend bundle status when provided", () => {
    const degraded: Workspace = {
      ...workspace([db("target"), db("catalog"), db("embeddings"), db("workspace")]),
      bundle: {
        requiredRoles: ["target", "catalog", "embeddings", "workspace"],
        configuredRoles: ["target", "catalog"],
        missingRoles: ["embeddings", "workspace"],
        status: "degraded",
        complete: false,
      },
    };

    expect(workspaceBundleComplete(degraded)).toBe(false);
    expect(workspaceBundleLabel(degraded)).toBe("2 missing");
    expect(missingBundleRoleList(degraded)).toEqual(["embeddings", "workspace"]);
  });
});

describe("workspace context", () => {
  test("normalizes blank workspace ids to default without tenant suffix", () => {
    const context = workspaceContextFromIds("tenant-a", "   ");

    expect(context).toEqual({
      baseTenantId: "tenant-a",
      workspaceId: "default",
      workspaceTenantId: "tenant-a",
      isDefaultWorkspace: true,
      headers: { "X-Workspace-Id": "default" },
    });
    expect(Object.isFrozen(context)).toBe(true);
    expect(Object.isFrozen(context.headers)).toBe(true);
  });

  test("derives non-default workspace tenant ids and preserves profile metadata", () => {
    const context = workspaceContextFromIds("tenant-a", " customer-a ", {
      profileId: "profile-1",
      bundleId: "bundle-1",
    });

    expect(context).toEqual({
      baseTenantId: "tenant-a",
      workspaceId: "customer-a",
      workspaceTenantId: "tenant-a::workspace:customer-a",
      isDefaultWorkspace: false,
      headers: { "X-Workspace-Id": "customer-a" },
      profileId: "profile-1",
      bundleId: "bundle-1",
    });
  });
});
