/**
 * Workspace domain types for studio multi-tenancy.
 */

export type WorkspaceId = string;

export type WorkspaceDatabaseType = "sqlite" | "neondb" | "external";

export interface DatabaseConfig {
  readonly type: "default" | "external";
  readonly url?: string;
  readonly schema?: string;
}

export interface WorkspaceDatabase {
  readonly id: string;
  readonly workspaceId: string;
  readonly role: WorkspaceRole;
  readonly displayName: string;
  readonly databaseUrl?: string;
  readonly createdAt: string;
}

export type WorkspaceBundleStatus = "complete" | "degraded" | "empty";

export interface WorkspaceBundle {
  readonly requiredRoles: WorkspaceRole[];
  readonly configuredRoles: WorkspaceRole[];
  readonly missingRoles: WorkspaceRole[];
  readonly status: WorkspaceBundleStatus;
  readonly complete: boolean;
}

export interface Workspace {
  readonly id: WorkspaceId;
  readonly displayName: string;
  readonly createdAt: string;
  readonly updatedAt?: string;
  readonly databaseType?: WorkspaceDatabaseType;
  readonly databases: WorkspaceDatabase[];
  readonly bundle?: WorkspaceBundle;
}

export interface ProvisionWorkspaceRequest {
  readonly databaseUrl: string;
  readonly displayNamePrefix?: string;
}

export interface CreateWorkspaceRequest {
  readonly id?: string;
  readonly displayName: string;
  readonly databaseUrl?: string;
  readonly catalogDatabaseUrl?: string;
  readonly embeddingsDatabaseUrl?: string;
  readonly workspaceDatabaseUrl?: string;
  readonly databaseType?: WorkspaceDatabaseType;
  readonly neondbApiKey?: string;
  readonly neondbProjectId?: string;
}

export interface UpdateWorkspaceRequest {
  readonly displayName?: string;
}

export function sortWorkspacesByRecent(workspaces: readonly Workspace[]): Workspace[] {
  return [...workspaces].sort(
    (a, b) => new Date(b.createdAt || 0).getTime() - new Date(a.createdAt || 0).getTime()
  );
}

export function findWorkspace(
  workspaces: readonly Workspace[],
  id: WorkspaceId
): Workspace | undefined {
  return workspaces.find((w) => w.id === id);
}

export function isDefaultWorkspace(ws: Workspace): boolean {
  return ws.id === "default";
}

export interface WorkspaceContext {
  readonly baseTenantId: string;
  readonly workspaceId: WorkspaceId;
  readonly workspaceTenantId: string;
  readonly isDefaultWorkspace: boolean;
  readonly headers: Readonly<Record<"X-Workspace-Id", string>>;
  readonly profileId?: string;
  readonly bundleId?: string;
}

export function normalizeWorkspaceId(workspaceId?: string | null): WorkspaceId {
  const trimmed = (workspaceId ?? "").trim();
  return trimmed.length === 0 ? "default" : trimmed;
}

export function workspaceTenantId(baseTenantId: string, workspaceId?: string | null): string {
  const normalized = normalizeWorkspaceId(workspaceId);
  return normalized === "default" ? baseTenantId : `${baseTenantId}::workspace:${normalized}`;
}

export function workspaceContextFromIds(
  baseTenantId: string,
  workspaceId?: string | null,
  options?: {
    readonly profileId?: string;
    readonly bundleId?: string;
  }
): WorkspaceContext {
  const normalized = normalizeWorkspaceId(workspaceId);
  return Object.freeze({
    baseTenantId,
    workspaceId: normalized,
    workspaceTenantId: workspaceTenantId(baseTenantId, normalized),
    isDefaultWorkspace: normalized === "default",
    headers: Object.freeze({ "X-Workspace-Id": normalized }),
    ...(options?.profileId === undefined ? {} : { profileId: options.profileId }),
    ...(options?.bundleId === undefined ? {} : { bundleId: options.bundleId }),
  });
}

// ============================================================================
// URL Derivation (client-side preview)
// ============================================================================

export const WORKSPACE_ROLES = ["target", "catalog", "embeddings", "workspace"] as const;
export type WorkspaceRole = (typeof WORKSPACE_ROLES)[number];

export const ROLE_LABELS: Record<WorkspaceRole, string> = {
  target: "Target",
  catalog: "Catalog",
  embeddings: "Embeddings",
  workspace: "Workspace",
};

export function missingBundleRoleList(workspace: Workspace): WorkspaceRole[] {
  if (workspace.bundle) return [...workspace.bundle.missingRoles];
  const configuredRoles = new Set(workspace.databases.map((db) => db.role));
  return WORKSPACE_ROLES.filter((role) => !configuredRoles.has(role));
}

export function workspaceBundleComplete(workspace: Workspace): boolean {
  if (workspace.bundle) return workspace.bundle.complete;
  return missingBundleRoleList(workspace).length === 0;
}

export function workspaceBundleLabel(workspace: Workspace): string {
  if (workspaceBundleComplete(workspace)) return "full";
  const missing = missingBundleRoleList(workspace).length;
  return missing > 0 ? `${missing} missing` : "degraded";
}

export function missingBundleRolesLabel(workspace: Workspace): string {
  return missingBundleRoleList(workspace)
    .map((role) => ROLE_LABELS[role])
    .join(", ");
}

export interface DerivedWorkspaceUrl {
  readonly role: WorkspaceRole;
  readonly displayName: string;
  readonly databaseUrl: string;
}

/**
 * Client-side preview of the 4 derived database URLs from a base PostgreSQL URL.
 * Returns null if the URL is invalid or not a PostgreSQL URL.
 */
export function deriveWorkspaceUrls(baseUrl: string): DerivedWorkspaceUrl[] | null {
  const trimmed = baseUrl.trim();
  if (!trimmed.startsWith("postgresql://") && !trimmed.startsWith("postgres://")) {
    return null;
  }

  try {
    const url = new URL(trimmed);
    const originalDb = url.pathname.replace(/^\//, "");
    if (!originalDb) return null;

    return WORKSPACE_ROLES.map((role) => {
      const derived = new URL(trimmed);
      derived.pathname = role === "target" ? `/${originalDb}` : `/${role}`;
      return {
        role,
        displayName: `${originalDb} — ${ROLE_LABELS[role]}`,
        databaseUrl: derived.toString(),
      };
    });
  } catch {
    return null;
  }
}
