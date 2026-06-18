/**
 * Workspace store for multi-tenancy workspace management.
 *
 * Manages the active workspace selection and workspace list.
 * Persists active workspace ID to localStorage.
 *
 * @module stores/workspace-store
 */

import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";
import { immer } from "zustand/middleware/immer";
import { setWorkspaceHeader as setWorkspaceHeaderApi } from "~/lib/api/client";
import type { Workspace, WorkspaceDatabase, WorkspaceId } from "~/lib/domain/workspace";
import { findWorkspace, sortWorkspacesByRecent } from "~/lib/domain/workspace";
import { applyWorkspaceRuntimeScope as applyWorkspaceRuntimeScopeDescription } from "~/lib/domain/workspace-runtime-scope";
import { setStorageNamespace as setStorageNamespaceStorage } from "~/lib/storage";

// ============================================================================
// State Types
// ============================================================================

/**
 * Loading state for workspace list.
 * @deprecated Use AsyncPhase from ~/lib/domain
 */
export type WorkspaceListState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready" }
  | { status: "error"; error: string };

/**
 * Workspace store state.
 */
export interface WorkspaceState {
  workspaces: Workspace[];
  activeWorkspaceId: WorkspaceId;
  listState: WorkspaceListState;
}

/**
 * Workspace store actions.
 */
export interface WorkspaceActions {
  setWorkspaces: (workspaces: Workspace[]) => void;
  addWorkspace: (workspace: Workspace) => void;
  updateWorkspace: (id: WorkspaceId, update: Partial<Workspace>) => void;
  removeWorkspace: (id: WorkspaceId) => void;
  setActiveWorkspace: (id: WorkspaceId) => void;
  setListState: (state: WorkspaceListState) => void;
  getWorkspace: (id: WorkspaceId) => Workspace | undefined;
}

export type WorkspaceStore = WorkspaceState & WorkspaceActions;

// ============================================================================
// Initial State
// ============================================================================

const initialState: WorkspaceState = {
  workspaces: [],
  activeWorkspaceId: "default",
  listState: { status: "idle" },
};

const fallbackWorkspaceId = (workspaces: readonly Workspace[]): WorkspaceId =>
  workspaces[0]?.id ?? "default";

const workspaceRuntimeScopeInterpreter = {
  setStorageNamespace: setStorageNamespaceStorage,
  setWorkspaceHeader: setWorkspaceHeaderApi,
};

function applyWorkspaceRuntimeScope(workspaceId: WorkspaceId | null | undefined): void {
  applyWorkspaceRuntimeScopeDescription(workspaceRuntimeScopeInterpreter, workspaceId);
}

applyWorkspaceRuntimeScope(initialState.activeWorkspaceId);

// ============================================================================
// Store Implementation
// ============================================================================

export const useWorkspace = create<WorkspaceStore>()(
  persist(
    immer((set, get) => ({
      ...initialState,

      setWorkspaces: (workspaces) => {
        let nextActiveWorkspaceId: WorkspaceId | null = null;
        set((state) => {
          const sortedWorkspaces = sortWorkspacesByRecent(workspaces);
          state.workspaces = sortedWorkspaces;
          state.listState = { status: "ready" };
          // Validate active workspace still exists — fall back to most recent.
          if (!sortedWorkspaces.some((ws) => ws.id === state.activeWorkspaceId)) {
            nextActiveWorkspaceId = fallbackWorkspaceId(sortedWorkspaces);
            state.activeWorkspaceId = nextActiveWorkspaceId;
          }
        });
        if (nextActiveWorkspaceId) {
          applyWorkspaceRuntimeScope(nextActiveWorkspaceId);
        }
      },

      addWorkspace: (workspace) =>
        set((state) => {
          state.workspaces = [workspace, ...state.workspaces];
        }),

      updateWorkspace: (id, update) =>
        set((state) => {
          state.workspaces = state.workspaces.map((ws) =>
            ws.id === id ? { ...ws, ...update } : ws
          );
        }),

      removeWorkspace: (id) => {
        let nextActiveWorkspaceId: WorkspaceId | null = null;
        set((state) => {
          state.workspaces = state.workspaces.filter((ws) => ws.id !== id);
          if (state.activeWorkspaceId === id) {
            nextActiveWorkspaceId = fallbackWorkspaceId(state.workspaces);
            state.activeWorkspaceId = nextActiveWorkspaceId;
          }
        });
        if (nextActiveWorkspaceId) {
          applyWorkspaceRuntimeScope(nextActiveWorkspaceId);
        }
      },

      setActiveWorkspace: (id) => {
        applyWorkspaceRuntimeScope(id);
        set((state) => {
          state.activeWorkspaceId = id;
        });
      },

      setListState: (listState) =>
        set((state) => {
          state.listState = listState;
        }),

      getWorkspace: (id) => findWorkspace(get().workspaces, id),
    })),
    {
      name: "studio-workspace",
      storage: createJSONStorage(() => localStorage),
      partialize: (state) => ({
        activeWorkspaceId: state.activeWorkspaceId,
      }),
      // Initialize runtime scope immediately after localStorage rehydration:
      // API headers and namespaced settings storage must move together.
      onRehydrateStorage: () => (state) => {
        if (state?.activeWorkspaceId) {
          applyWorkspaceRuntimeScope(state.activeWorkspaceId);
        }
      },
    }
  )
);

/** @deprecated Use useWorkspace */
export const useWorkspaceStore = useWorkspace;

// ============================================================================
// State Selectors (Granular subscriptions)
// ============================================================================

export const selectWorkspaces = (state: WorkspaceStore) => state.workspaces;
export const selectActiveWorkspaceId = (state: WorkspaceStore) => state.activeWorkspaceId;
export const selectWorkspaceListState = (state: WorkspaceStore) => state.listState;
export const selectWorkspaceIsLoading = (state: WorkspaceStore) =>
  state.listState.status === "loading";
export const selectWorkspaceIsReady = (state: WorkspaceStore) => state.listState.status === "ready";
export const selectWorkspaceError = (state: WorkspaceStore) =>
  state.listState.status === "error" ? state.listState.error : null;
export const selectWorkspaceCount = (state: WorkspaceStore) => state.workspaces.length;
export const selectHasWorkspaces = (state: WorkspaceStore) => state.workspaces.length > 0;

/**
 * Select the currently active workspace.
 */
export const selectActiveWorkspace = (state: WorkspaceStore): Workspace | undefined =>
  findWorkspace(state.workspaces, state.activeWorkspaceId);

/**
 * Select databases of the currently active workspace.
 */
export const selectActiveWorkspaceDatabases = (state: WorkspaceStore): WorkspaceDatabase[] =>
  findWorkspace(state.workspaces, state.activeWorkspaceId)?.databases ?? EMPTY_DATABASES;

const EMPTY_DATABASES: WorkspaceDatabase[] = [];

// ============================================================================
// Action Selectors (Stable references for callbacks)
// ============================================================================

export const selectSetWorkspaces = (state: WorkspaceStore) => state.setWorkspaces;
export const selectAddWorkspace = (state: WorkspaceStore) => state.addWorkspace;
export const selectUpdateWorkspace = (state: WorkspaceStore) => state.updateWorkspace;
export const selectRemoveWorkspace = (state: WorkspaceStore) => state.removeWorkspace;
export const selectSetActiveWorkspace = (state: WorkspaceStore) => state.setActiveWorkspace;
export const selectSetWorkspaceListState = (state: WorkspaceStore) => state.setListState;
export const selectGetWorkspace = (state: WorkspaceStore) => state.getWorkspace;
