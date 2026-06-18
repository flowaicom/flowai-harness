/**
 * Profiling pipeline store — multi-session profiling job lifecycle + per-table matrix.
 *
 * Session-indexed: multiple profiling jobs can run concurrently.
 * Each job gets its own ProfilingSession in `sessions`.
 *
 * The `interpretIngestionEvent` is the interpreter (exhaustive switch
 * on IngestionEvent) — the effect interpreter pattern.
 *
 * @module stores/profiling-pipeline
 */

import { castDraft, enableMapSet } from "immer";
import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import { useShallow } from "zustand/react/shallow";
import type {
  IngestionEvent,
  IngestionStatus,
  PipelineStageKey,
  TablePipelineState,
  TableStageStatus,
} from "~/lib/domain/data";
import { useSessionRegistry } from "./session-registry";
import { useWorkspace } from "./workspace-store";

enableMapSet();

// =============================================================================
// Session Type
// =============================================================================

/** Per-job profiling session with matrix state. */
export interface ProfilingSession {
  jobId: string;
  isRunning: boolean;
  jobStatus: IngestionStatus | null;
  tableStages: Map<string, TablePipelineState>;
  discoveredTableNames: string[];
  currentTable: string | null;
  startedAt: number | null;
  totalTableCount: number;
  completedTableCount: number;
  error: string | null;
}

// =============================================================================
// State
// =============================================================================

export interface ProfilingPipelineState {
  sessions: Map<string, ProfilingSession>;
  activeJobId: string | null;
}

export interface ProfilingPipelineActions {
  startProfiling: (jobId: string) => void;
  interpretIngestionEvent: (jobId: string, event: IngestionEvent) => void;
  completeProfiling: (jobId?: string) => void;
  resetMatrix: (jobId?: string) => void;
  setError: (error: string | null, jobId?: string) => void;
}

export type ProfilingPipelineStore = ProfilingPipelineState & ProfilingPipelineActions;

// =============================================================================
// Constants
// =============================================================================

const STAGE_ORDER: PipelineStageKey[] = [
  "discovering",
  "profiling",
  "enriching",
  "extracting",
  "indexing",
];

function isTerminalTableState(entry: TablePipelineState): boolean {
  const hasFailed = STAGE_ORDER.some((key) => entry.stages[key] === "failed");
  if (hasFailed) return true;
  return STAGE_ORDER.every((key) => entry.stages[key] === "completed");
}

// =============================================================================
// Initial State
// =============================================================================

const initialState: ProfilingPipelineState = {
  sessions: new Map(),
  activeJobId: null,
};

function createProfilingSession(jobId: string, now: number): ProfilingSession {
  return {
    jobId,
    isRunning: true,
    jobStatus: null,
    tableStages: new Map(),
    discoveredTableNames: [],
    currentTable: null,
    startedAt: now,
    totalTableCount: 0,
    completedTableCount: 0,
    error: null,
  };
}

// =============================================================================
// Store
// =============================================================================

export const useProfilingPipeline = create<ProfilingPipelineStore>()(
  immer((set) => ({
    ...initialState,

    startProfiling: (jobId) => {
      const now = Date.now();
      set((state) => {
        const session = createProfilingSession(jobId, now);
        state.sessions.set(jobId, castDraft(session));
        state.activeJobId = jobId;
      });
      // Effect: register with session registry (after pure state transition)
      const workspaceId = useWorkspace.getState().activeWorkspaceId;
      useSessionRegistry.getState().register({
        id: `profiling-${jobId}`,
        kind: "profiling",
        label: `Profiling ${jobId.slice(0, 8)}`,
        workspaceId,
        startedAt: now,
        routeTo: "/connect/profiling",
        jobId,
      });
    },

    interpretIngestionEvent: (jobId, event) => {
      const now = Date.now();
      const out: { autoCreatedJobId: string | null } = { autoCreatedJobId: null };

      // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: store event handler
      set((state) => {
        let session = state.sessions.get(jobId);

        // Auto-create session if started event arrives first
        if (!session && event.type === "started") {
          const newSession = createProfilingSession(event.jobId, now);
          state.sessions.set(event.jobId, castDraft(newSession));
          state.activeJobId = event.jobId;
          session = state.sessions.get(event.jobId);
          out.autoCreatedJobId = event.jobId;
        }

        if (!session) return;

        switch (event.type) {
          case "started":
            if (!session.isRunning) {
              session.isRunning = true;
              session.startedAt = now;
            }
            break;

          case "progress": {
            session.jobStatus = castDraft(event.status);
            const sk = event.status.status;
            if (sk === "discovering" && "tablesFound" in event.status) {
              const n = (event.status as { tablesFound: number }).tablesFound;
              if (n > session.totalTableCount) session.totalTableCount = n;
            }
            break;
          }

          case "tableProfiled": {
            const existing = session.tableStages.get(event.tableName);
            if (existing && isTerminalTableState(existing)) {
              existing.columns = event.columns;
              existing.durationMs = event.durationMs;
              break;
            }

            session.currentTable = event.tableName;
            if (!existing) {
              session.discoveredTableNames.push(event.tableName);
            }
            const stages: Record<PipelineStageKey, TableStageStatus> = {
              discovering: "completed",
              profiling: "completed",
              enriching: "queued",
              extracting: "queued",
              indexing: "queued",
            };
            session.tableStages.set(
              event.tableName,
              castDraft({
                tableName: event.tableName,
                stages,
                columns: event.columns,
                durationMs: event.durationMs,
              })
            );
            break;
          }

          case "tableEnriched": {
            const entry = session.tableStages.get(event.tableName);
            if (entry) {
              entry.stages.enriching = "completed";
              (entry as { enrichmentSource?: string }).enrichmentSource = event.source;
            }
            break;
          }

          case "tableCompleted": {
            let entry = session.tableStages.get(event.tableName);
            const alreadyTerminal = entry ? isTerminalTableState(entry) : false;
            if (!entry) {
              session.discoveredTableNames.push(event.tableName);
              entry = castDraft({
                tableName: event.tableName,
                stages: {
                  discovering: "completed",
                  profiling: "completed",
                  enriching: "completed",
                  extracting: "completed",
                  indexing: "completed",
                },
                columns: 0,
                durationMs: 0,
              });
              session.tableStages.set(event.tableName, entry);
            } else {
              for (const key of STAGE_ORDER) {
                entry.stages[key] = "completed";
              }
            }
            if (!alreadyTerminal) {
              session.completedTableCount++;
            }
            if (session.currentTable === event.tableName) {
              session.currentTable = null;
            }
            break;
          }

          case "tableFailed": {
            let entry = session.tableStages.get(event.tableName);
            const alreadyTerminal = entry ? isTerminalTableState(entry) : false;
            if (!entry) {
              const stages: Record<PipelineStageKey, TableStageStatus> = {
                discovering: "failed",
                profiling: "failed",
                enriching: "failed",
                extracting: "failed",
                indexing: "failed",
              };
              session.discoveredTableNames.push(event.tableName);
              session.tableStages.set(
                event.tableName,
                castDraft({ tableName: event.tableName, stages, columns: 0, durationMs: 0 })
              );
              entry = session.tableStages.get(event.tableName)!;
            } else {
              for (const key of STAGE_ORDER) {
                if (entry.stages[key] !== "completed") {
                  entry.stages[key] = "failed";
                }
              }
            }
            if (!alreadyTerminal) {
              session.completedTableCount++;
            }
            if (session.currentTable === event.tableName) {
              session.currentTable = null;
            }
            break;
          }

          case "completed":
            // Sweep: mark tables without explicit completion as completed.
            for (const entry of session.tableStages.values()) {
              const hasFailed = STAGE_ORDER.some((k) => entry.stages[k] === "failed");
              const allDone = STAGE_ORDER.every((k) => entry.stages[k] === "completed");
              if (!hasFailed && !allDone) {
                for (const key of STAGE_ORDER) {
                  entry.stages[key] = "completed";
                }
                session.completedTableCount++;
              }
            }
            session.currentTable = null;
            session.jobStatus = castDraft({
              status: "completed" as const,
              summary: event.summary,
            });
            break;

          case "error":
            session.isRunning = false;
            session.error = event.message;
            session.currentTable = null;
            break;
        }
      });

      // Effect: register auto-created session with session registry
      if (out.autoCreatedJobId) {
        const workspaceId = useWorkspace.getState().activeWorkspaceId;
        useSessionRegistry.getState().register({
          id: `profiling-${out.autoCreatedJobId}`,
          kind: "profiling",
          label: `Profiling ${out.autoCreatedJobId.slice(0, 8)}`,
          workspaceId,
          startedAt: now,
          routeTo: "/connect/profiling",
          jobId: out.autoCreatedJobId,
        });
      }
    },

    completeProfiling: (jobId?) => {
      const resolved: { id: string | null } = { id: null };
      set((state) => {
        const id = jobId ?? state.activeJobId;
        if (!id) return;
        resolved.id = id;
        const session = state.sessions.get(id);
        if (session) {
          session.isRunning = false;
        }
      });
      // Effect: mark terminal in session registry (after pure state transition)
      if (resolved.id) {
        useSessionRegistry.getState().markTerminal(`profiling-${resolved.id}`, "completed");
      }
    },

    resetMatrix: (jobId?) => {
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
        useSessionRegistry.getState().deregister(`profiling-${resolved.id}`);
      }
    },

    setError: (error, jobId?) =>
      set((state) => {
        const id = jobId ?? state.activeJobId;
        if (!id) return;
        const session = state.sessions.get(id);
        if (session) {
          session.error = error;
        }
      }),
  }))
);

// =============================================================================
// Backward-Compatible Selectors (derive from active session)
// =============================================================================

const getActiveSession = (state: ProfilingPipelineStore): ProfilingSession | undefined =>
  state.activeJobId ? state.sessions.get(state.activeJobId) : undefined;

const EMPTY_TABLE_STAGES = new Map<string, TablePipelineState>();
const EMPTY_TABLE_NAMES: string[] = [];
const EMPTY_PROFILING_JOBS = new Map<string, IngestionStatus>();

export const selectProfilingJobs = (
  state: ProfilingPipelineStore
): Map<string, IngestionStatus> => {
  // Build a map from all sessions' job statuses
  const jobs = new Map<string, IngestionStatus>();
  for (const session of state.sessions.values()) {
    if (session.jobStatus) {
      jobs.set(session.jobId, session.jobStatus);
    }
  }
  return jobs.size > 0 ? jobs : EMPTY_PROFILING_JOBS;
};

export const selectProfilingActiveJobId = (state: ProfilingPipelineStore) => state.activeJobId;

export const selectProfilingIsRunning = (state: ProfilingPipelineStore): boolean => {
  for (const session of state.sessions.values()) {
    if (session.isRunning) return true;
  }
  return false;
};

export const selectTableStages = (state: ProfilingPipelineStore) =>
  getActiveSession(state)?.tableStages ?? EMPTY_TABLE_STAGES;

export const selectDiscoveredTableNames = (state: ProfilingPipelineStore) =>
  getActiveSession(state)?.discoveredTableNames ?? EMPTY_TABLE_NAMES;

export const selectCurrentProfilingTable = (state: ProfilingPipelineStore) =>
  getActiveSession(state)?.currentTable ?? null;

export const selectProfilingStartedAt = (state: ProfilingPipelineStore) =>
  getActiveSession(state)?.startedAt ?? null;

export const selectTotalTableCount = (state: ProfilingPipelineStore) =>
  getActiveSession(state)?.totalTableCount ?? 0;

export const selectCompletedTableCount = (state: ProfilingPipelineStore) =>
  getActiveSession(state)?.completedTableCount ?? 0;

export const selectProfilingError = (state: ProfilingPipelineStore) =>
  getActiveSession(state)?.error ?? null;

// Multi-session selectors
export const selectProfilingSessions = (state: ProfilingPipelineStore) => state.sessions;
export const selectRunningProfilingCount = (state: ProfilingPipelineStore): number => {
  let count = 0;
  for (const session of state.sessions.values()) {
    if (session.isRunning) count++;
  }
  return count;
};

// =============================================================================
// Action Bundle
// =============================================================================

export function useProfilingPipelineActions() {
  return useProfilingPipeline(
    useShallow((s) => ({
      startProfiling: s.startProfiling,
      interpretIngestionEvent: s.interpretIngestionEvent,
      completeProfiling: s.completeProfiling,
      resetMatrix: s.resetMatrix,
      setError: s.setError,
    }))
  );
}
