/**
 * Import pipeline store — multi-session data import lifecycle.
 *
 * Session-indexed: multiple import jobs can run concurrently.
 * Each job gets its own ImportSession in `sessions`.
 *
 * Domain-agnostic: no hardcoded table names, no dim/fact vocabulary.
 * The domain reducer interprets ImportEvent values. This store only applies
 * that pure state transition and performs the required browser/app effects.
 *
 * @module stores/import-pipeline
 */

import { castDraft, enableMapSet } from "immer";
import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import { useShallow } from "zustand/react/shallow";
import type { ImportEvent } from "~/lib/domain/data";
import {
  createImportSessionModel,
  type ImportLifecycleEffect,
  type ImportSessionModel,
  reduceImportSessionEvent,
} from "~/lib/domain/import-pipeline";
import { lifecycleBus } from "./lifecycle-bus";
import { useSessionRegistry } from "./session-registry";
import { useSourceCatalog } from "./source-catalog";
import { useWorkspace } from "./workspace-store";

enableMapSet();

// =============================================================================
// Session Type
// =============================================================================

/** Per-job import session state. */
export type ImportSession = ImportSessionModel;

// =============================================================================
// State
// =============================================================================

export interface ImportPipelineState {
  sessions: Map<string, ImportSession>;
  activeJobId: string | null;
}

export interface ImportPipelineActions {
  startImport: (jobId: string) => void;
  interpretImportEvent: (jobId: string, event: ImportEvent) => void;
  completeImport: (jobId?: string) => void;
  resetImport: (jobId?: string) => void;
}

export type ImportPipelineStore = ImportPipelineState & ImportPipelineActions;

// =============================================================================
// Initial State
// =============================================================================

const initialState: ImportPipelineState = {
  sessions: new Map(),
  activeJobId: null,
};

// =============================================================================
// Store
// =============================================================================

export const useImportPipeline = create<ImportPipelineStore>()(
  immer((set) => ({
    ...initialState,

    startImport: (jobId) => {
      const now = Date.now();
      set((state) => {
        const session = createImportSessionModel(jobId, now);
        state.sessions.set(jobId, castDraft(session));
        state.activeJobId = jobId;
      });
      // Effect: register with session registry (after pure state transition)
      const workspaceId = useWorkspace.getState().activeWorkspaceId;
      useSessionRegistry.getState().register({
        id: `import-${jobId}`,
        kind: "import",
        label: `Import ${jobId.slice(0, 8)}`,
        workspaceId,
        startedAt: now,
        routeTo: "/connect/import",
        jobId,
      });
    },

    interpretImportEvent: (jobId, event) => {
      const now = Date.now();

      const out: { autoCreatedJobId: string | null } = { autoCreatedJobId: null };
      let effects: readonly ImportLifecycleEffect[] = [];

      set((state) => {
        let session = state.sessions.get(jobId);

        // Auto-create session if started event arrives first
        if (!session && event.type === "started") {
          const newSession = createImportSessionModel(event.jobId, now);
          state.sessions.set(event.jobId, castDraft(newSession));
          state.activeJobId = event.jobId;
          session = state.sessions.get(event.jobId);
          out.autoCreatedJobId = event.jobId;
        }

        if (!session) return;

        const reduction = reduceImportSessionEvent(session, event);
        state.sessions.set(session.jobId, castDraft(reduction.session));
        effects = reduction.effects;
      });

      // Effect: register auto-created session with session registry
      if (out.autoCreatedJobId) {
        const workspaceId = useWorkspace.getState().activeWorkspaceId;
        useSessionRegistry.getState().register({
          id: `import-${out.autoCreatedJobId}`,
          kind: "import",
          label: `Import ${out.autoCreatedJobId.slice(0, 8)}`,
          workspaceId,
          startedAt: now,
          routeTo: "/connect/import",
          jobId: out.autoCreatedJobId,
        });
      }

      // Effects: lifecycle bus emissions (after pure state transition)
      if (effects.length > 0) {
        const sourceId = useSourceCatalog.getState().selectedSourceId ?? "";
        const workspaceId = useWorkspace.getState().activeWorkspaceId;
        for (const effect of effects) {
          if (effect.type === "profilingComplete") {
            lifecycleBus.emit({
              type: "profilingComplete",
              sourceId,
              tableCount: effect.tableCount,
              workspaceId,
            });
          } else {
            lifecycleBus.emit({
              type: "importComplete",
              sourceId,
              tableCount: effect.tableCount,
              totalRowCount: effect.totalRowCount,
              workspaceId,
            });
          }
        }
      }
    },

    completeImport: (jobId?) => {
      const resolved: { id: string | null } = { id: null };
      set((state) => {
        const id = jobId ?? state.activeJobId;
        if (!id) return;
        resolved.id = id;
        const session = state.sessions.get(id);
        if (session) {
          state.sessions.set(id, castDraft({ ...session, isRunning: false }));
        }
      });
      // Effect: mark terminal in session registry (after pure state transition)
      if (resolved.id) {
        useSessionRegistry.getState().markTerminal(`import-${resolved.id}`, "completed");
      }
    },

    resetImport: (jobId?) => {
      const resolved: { id: string | null } = { id: null };
      set((state) => {
        const id = jobId ?? state.activeJobId;
        resolved.id = id;
        if (id) {
          state.sessions.delete(id);
        }
        if (state.activeJobId === id) {
          state.activeJobId = null;
        }
      });
      // Effect: deregister from session registry (after pure state transition)
      if (resolved.id) {
        useSessionRegistry.getState().deregister(`import-${resolved.id}`);
      }
    },
  }))
);

// =============================================================================
// Backward-Compatible Selectors (derive from active session)
// =============================================================================

const getActiveSession = (state: ImportPipelineStore): ImportSession | undefined =>
  state.activeJobId ? state.sessions.get(state.activeJobId) : undefined;

const EMPTY_TABLE_COUNTS = new Map<string, number>();

export const selectImportJobId = (state: ImportPipelineStore) => state.activeJobId;
export const selectImportStage = (state: ImportPipelineStore) =>
  getActiveSession(state)?.importStage ?? null;
export const selectTableCounts = (state: ImportPipelineStore) =>
  getActiveSession(state)?.tableCounts ?? EMPTY_TABLE_COUNTS;
export const selectBatchRowsLoaded = (state: ImportPipelineStore) =>
  getActiveSession(state)?.batchRowsLoaded ?? 0;
export const selectBatchTotalRows = (state: ImportPipelineStore) =>
  getActiveSession(state)?.batchTotalRows ?? 0;

export const selectImportIsRunning = (state: ImportPipelineStore): boolean => {
  for (const session of state.sessions.values()) {
    if (session.isRunning) return true;
  }
  return false;
};

export const selectImportSummary = (state: ImportPipelineStore) =>
  getActiveSession(state)?.importSummary ?? null;
export const selectImportProfilingTotal = (state: ImportPipelineStore) =>
  getActiveSession(state)?.profilingTotal ?? 0;
export const selectImportProfilingCompleted = (state: ImportPipelineStore) =>
  getActiveSession(state)?.profilingCompleted ?? 0;
export const selectImportStartedAt = (state: ImportPipelineStore) =>
  getActiveSession(state)?.startedAt ?? null;
export const selectImportError = (state: ImportPipelineStore) =>
  getActiveSession(state)?.error ?? null;

// Multi-session selectors
export const selectImportSessions = (state: ImportPipelineStore) => state.sessions;
export const selectRunningImportCount = (state: ImportPipelineStore): number => {
  let count = 0;
  for (const session of state.sessions.values()) {
    if (session.isRunning) count++;
  }
  return count;
};

// =============================================================================
// Action Bundle
// =============================================================================

export function useImportPipelineActions() {
  return useImportPipeline(
    useShallow((s) => ({
      startImport: s.startImport,
      interpretImportEvent: s.interpretImportEvent,
      completeImport: s.completeImport,
      resetImport: s.resetImport,
    }))
  );
}
