import type { AppScope } from "@studio/core/domain/scope";

export const CONNECT_TARGET_WORKSPACE = "workspace";

export type ConnectTargetRouteOptions =
  | { readonly target: "source"; readonly sourceId: string }
  | { readonly target: "workspace"; readonly workspaceTargetId: string }
  | { readonly target?: undefined };

export type ConnectTargetSelection =
  | { readonly kind: "source"; readonly sourceId: string }
  | { readonly kind: "workspace-target" }
  | { readonly kind: "default" };

export function connectTargetOptions(
  sourceId: string | null | undefined,
  workspaceTargetId: string | null | undefined
): ConnectTargetRouteOptions {
  if (sourceId != null) return { target: "source", sourceId };
  if (workspaceTargetId != null) return { target: "workspace", workspaceTargetId };
  return {};
}

export function buildConnectTargetRoute(
  basePath: string,
  options: ConnectTargetRouteOptions = {}
): string {
  const search = new URLSearchParams();
  if (options.target === "source") {
    search.set("sourceId", options.sourceId);
  } else if (options.target === "workspace") {
    search.set("target", CONNECT_TARGET_WORKSPACE);
  }
  const query = search.toString();
  return query ? `${basePath}?${query}` : basePath;
}

export function buildConnectScopeRoute(
  basePath: string,
  targetKey: string,
  workspaceTargetId?: string | null
): string {
  if (targetKey === "" || targetKey === "default") {
    return basePath;
  }
  if (workspaceTargetId != null && targetKey === workspaceTargetId) {
    return buildConnectTargetRoute(basePath, { target: "workspace", workspaceTargetId });
  }
  return buildConnectTargetRoute(basePath, { target: "source", sourceId: targetKey });
}

export function deriveConnectTargetSelection(
  sourceIdFromUrl: string | null,
  prefersWorkspaceTarget: boolean
): ConnectTargetSelection {
  if (sourceIdFromUrl !== null) return { kind: "source", sourceId: sourceIdFromUrl };
  if (prefersWorkspaceTarget) return { kind: "workspace-target" };
  return { kind: "default" };
}

export function deriveConnectScope(
  workspaceId: string | null | undefined,
  sourceId: string | null | undefined
): AppScope | null {
  if (!workspaceId) return null;
  if (sourceId) {
    return { type: "source", workspaceId, sourceId };
  }
  return { type: "workspace", workspaceId };
}

export function connectTargetOptionsFromScope(
  scope: AppScope | null | undefined,
  workspaceTargetId: string | null | undefined
): ConnectTargetRouteOptions {
  if (!scope) return {};
  if (scope.type === "source") {
    return { target: "source", sourceId: scope.sourceId };
  }
  if (workspaceTargetId != null) {
    return { target: "workspace", workspaceTargetId };
  }
  return {};
}

export function getConnectEffectiveSourceId<TSource extends { readonly id: string }>(
  selectedSourceId: string | null | undefined,
  sources: readonly TSource[]
): string | null {
  if (selectedSourceId != null) return selectedSourceId;
  if (sources.length === 1) return sources[0]?.id ?? null;
  return null;
}
