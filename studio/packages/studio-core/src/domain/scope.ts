export type WorkspaceScope = {
  readonly type: "workspace";
  /** Canonical Harness Studio path scope key. */
  readonly workspaceKey?: string;
  /** Existing Studio alias retained while the UI cuts over to workspaceKey. */
  readonly workspaceId: string;
};

export type SourceScope = {
  readonly type: "source";
  /** Canonical Harness Studio path scope key. */
  readonly workspaceKey?: string;
  /** Existing Studio alias retained while the UI cuts over to workspaceKey. */
  readonly workspaceId: string;
  readonly sourceId: string;
};

export type AppScope = WorkspaceScope | SourceScope;

export function hasSourceScope(scope: AppScope): scope is SourceScope {
  return scope.type === "source";
}

export function getWorkspaceKey(scope: AppScope): string {
  return scope.workspaceKey ?? scope.workspaceId;
}

export function createWorkspaceScope(workspaceKey: string): WorkspaceScope {
  return { type: "workspace", workspaceKey, workspaceId: workspaceKey };
}

export function createSourceScope(workspaceKey: string, sourceId: string): SourceScope {
  return { type: "source", workspaceKey, workspaceId: workspaceKey, sourceId };
}
