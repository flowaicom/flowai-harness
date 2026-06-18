import type { WorkspaceId } from "./workspace";

export interface WorkspaceRuntimeScopeInterpreter {
  readonly setStorageNamespace: (workspaceId: WorkspaceId) => void;
  /**
   * Legacy API compatibility only. Harness Studio routes workspace scope
   * explicitly through `/workspaces/:workspaceKey/...`; this header must not be
   * required for correct harness routing.
   */
  readonly setWorkspaceHeader?: (workspaceId: WorkspaceId) => void;
}

export function normalizeWorkspaceRuntimeScopeId(
  workspaceId: WorkspaceId | null | undefined
): WorkspaceId {
  return workspaceId || "default";
}

export function applyWorkspaceRuntimeScope(
  interpreter: WorkspaceRuntimeScopeInterpreter,
  workspaceId: WorkspaceId | null | undefined
): WorkspaceId {
  const normalized = normalizeWorkspaceRuntimeScopeId(workspaceId);
  interpreter.setStorageNamespace(normalized);
  interpreter.setWorkspaceHeader?.(normalized);
  return normalized;
}
