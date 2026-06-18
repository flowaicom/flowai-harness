/**
 * Session registry store — cross-cutting tracker for all active sessions.
 *
 * Lightweight metadata-only store: does NOT duplicate session state.
 * Domain stores register/deregister sessions as side effects of start/complete
 * actions. The Activity Center subscribes only to this store.
 *
 * Sessions are workspace-scoped. The registry filters by activeWorkspaceId
 * when providing data to the Activity Center.
 *
 * @module stores/session-registry
 */

import { useEffect, useMemo } from "react";
import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import { useShallow } from "zustand/react/shallow";
import { useWorkspace } from "./workspace-store";

// =============================================================================
// Types
// =============================================================================

export type SessionKind = "chat-stream" | "eval-run" | "profiling" | "import" | "builder";

/**
 * Join-semilattice: active ≤ completed | failed | cancelled.
 * Monotonicity law: status only advances forward (never reverts to active).
 */
export type SessionStatus = "active" | "completed" | "failed" | "cancelled";

export interface SessionEntry {
  readonly id: string;
  readonly kind: SessionKind;
  label: string;
  status: SessionStatus;
  readonly workspaceId: string;
  readonly startedAt: number;
  /** Timestamp when session transitioned to a terminal status. */
  terminatedAt?: number;
  routeTo: string;
  /** Backend job ID for cancellation via /api/jobs/{jobId}/cancel. */
  readonly jobId?: string;
}

export interface SessionRegistryState {
  sessions: Map<string, SessionEntry>;
}

export interface SessionRegistryActions {
  register: (entry: Omit<SessionEntry, "status">) => void;
  /** Advance session to a terminal status (semilattice join). */
  markTerminal: (id: string, status: "completed" | "failed" | "cancelled") => void;
  /** Immediate removal — use markTerminal + sweep for graceful lifecycle. */
  deregister: (id: string) => void;
  updateLabel: (id: string, label: string) => void;
  /**
   * Status-aware sweep.
   * - Terminal sessions (completed/failed/cancelled) older than terminalMaxAgeMs are removed.
   * - Active sessions older than orphanMaxAgeMs are removed (orphan safety net).
   */
  sweepStale: (terminalMaxAgeMs: number, orphanMaxAgeMs?: number) => void;
}

export type SessionRegistryStore = SessionRegistryState & SessionRegistryActions;

// =============================================================================
// Store
// =============================================================================

export const useSessionRegistry = create<SessionRegistryStore>()(
  immer((set) => ({
    sessions: new Map(),

    register: (entry) =>
      set((state) => {
        state.sessions.set(entry.id, { ...entry, status: "active" });
      }),

    markTerminal: (id, status) =>
      set((state) => {
        const entry = state.sessions.get(id);
        if (!entry) return;
        if (entry.status === "active") {
          // Normal transition: active → terminal
          entry.status = status;
          entry.terminatedAt = Date.now();
        } else if (entry.status !== status) {
          // Terminal → terminal correction (e.g., backend reports "failed"
          // after frontend already set "completed").  Allow the correction
          // when the new status has equal or higher severity so the UI
          // converges with backend state.
          const severity: Record<string, number> = {
            completed: 1,
            failed: 2,
            cancelled: 2,
          };
          if ((severity[status] ?? 0) >= (severity[entry.status] ?? 0)) {
            entry.status = status;
          }
        }
      }),

    deregister: (id) =>
      set((state) => {
        state.sessions.delete(id);
      }),

    updateLabel: (id, label) =>
      set((state) => {
        const entry = state.sessions.get(id);
        if (entry) {
          entry.label = label;
        }
      }),

    sweepStale: (terminalMaxAgeMs, orphanMaxAgeMs = DEFAULT_ORPHAN_MAX_AGE_MS) => {
      const now = Date.now();
      const terminalCutoff = now - terminalMaxAgeMs;
      const orphanCutoff = now - orphanMaxAgeMs;
      set((state) => {
        for (const [id, entry] of state.sessions) {
          if (entry.status !== "active") {
            // Terminal sessions: sweep after short delay
            const terminatedTime = entry.terminatedAt ?? entry.startedAt;
            if (terminatedTime < terminalCutoff) {
              state.sessions.delete(id);
            }
          } else if (entry.startedAt < orphanCutoff) {
            // Active sessions: only sweep if truly orphaned (24h)
            state.sessions.delete(id);
          }
        }
      });
    },
  }))
);

// =============================================================================
// Pure Selectors — referentially transparent (workspaceId is explicit)
// =============================================================================

/** Total active session count across all workspaces. */
export const selectAllSessionCount = (state: SessionRegistryStore): number => {
  let count = 0;
  for (const entry of state.sessions.values()) {
    if (entry.status === "active") count++;
  }
  return count;
};

/** Active sessions for a given workspace. Pure: no hidden dependencies. */
export const selectSessionsForWorkspace =
  (workspaceId: string) =>
  (state: SessionRegistryStore): SessionEntry[] => {
    const result: SessionEntry[] = [];
    for (const entry of state.sessions.values()) {
      if (entry.status === "active" && entry.workspaceId === workspaceId) {
        result.push(entry);
      }
    }
    return result;
  };

/** Count of active sessions in a given workspace. Pure. */
export const selectSessionCountForWorkspace =
  (workspaceId: string) =>
  (state: SessionRegistryStore): number => {
    let count = 0;
    for (const entry of state.sessions.values()) {
      if (entry.status === "active" && entry.workspaceId === workspaceId) count++;
    }
    return count;
  };

/** Active sessions filtered by kind + workspace. Pure. */
export const selectSessionsByKindForWorkspace =
  (kind: SessionKind, workspaceId: string) =>
  (state: SessionRegistryStore): SessionEntry[] => {
    const result: SessionEntry[] = [];
    for (const entry of state.sessions.values()) {
      if (entry.status === "active" && entry.kind === kind && entry.workspaceId === workspaceId) {
        result.push(entry);
      }
    }
    return result;
  };

/** Count of active sessions NOT in a given workspace. Pure. */
export const selectOtherWorkspaceSessionCountFor =
  (workspaceId: string) =>
  (state: SessionRegistryStore): number => {
    let count = 0;
    for (const entry of state.sessions.values()) {
      if (entry.status === "active" && entry.workspaceId !== workspaceId) count++;
    }
    return count;
  };

// =============================================================================
// Derived Hooks — combine session registry + workspace store
// =============================================================================

/** Sessions for the active workspace. Re-renders on workspace OR session change. */
export function useActiveSessionsForWorkspace(): SessionEntry[] {
  const workspaceId = useWorkspace((s) => s.activeWorkspaceId);
  return useSessionRegistry(useShallow((state) => selectSessionsForWorkspace(workspaceId)(state)));
}

/** Count of sessions in the active workspace. */
export function useActiveSessionCount(): number {
  const workspaceId = useWorkspace((s) => s.activeWorkspaceId);
  const selector = useMemo(() => selectSessionCountForWorkspace(workspaceId), [workspaceId]);
  return useSessionRegistry(selector);
}

/** Count of sessions in OTHER workspaces. */
export function useOtherWorkspaceSessionCount(): number {
  const workspaceId = useWorkspace((s) => s.activeWorkspaceId);
  const selector = useMemo(() => selectOtherWorkspaceSessionCountFor(workspaceId), [workspaceId]);
  return useSessionRegistry(selector);
}

// Backward-compatible aliases (deprecated — prefer hooks above)
export const selectActiveSessions = (state: SessionRegistryStore): SessionEntry[] =>
  selectSessionsForWorkspace(useWorkspace.getState().activeWorkspaceId)(state);
export const selectActiveSessionCount = (state: SessionRegistryStore): number =>
  selectSessionCountForWorkspace(useWorkspace.getState().activeWorkspaceId)(state);
export const selectSessionsByKind =
  (kind: SessionKind) =>
  (state: SessionRegistryStore): SessionEntry[] =>
    selectSessionsByKindForWorkspace(kind, useWorkspace.getState().activeWorkspaceId)(state);
export const selectOtherWorkspaceSessionCount = (state: SessionRegistryStore): number =>
  selectOtherWorkspaceSessionCountFor(useWorkspace.getState().activeWorkspaceId)(state);

// =============================================================================
// Action Bundle
// =============================================================================

export function useSessionRegistryActions() {
  return useSessionRegistry(
    useShallow((s) => ({
      register: s.register,
      markTerminal: s.markTerminal,
      deregister: s.deregister,
      updateLabel: s.updateLabel,
      sweepStale: s.sweepStale,
    }))
  );
}

// =============================================================================
// Cleanup Hook — periodic orphan sweep
// =============================================================================

const DEFAULT_SWEEP_INTERVAL_MS = 60_000;
/** Terminal sessions (completed/failed/cancelled) are swept after 30 seconds. */
const DEFAULT_TERMINAL_MAX_AGE_MS = 30_000;
/** Active sessions are only swept after 24 hours (true orphan safety net). */
const DEFAULT_ORPHAN_MAX_AGE_MS = 86_400_000;

/**
 * Periodic sweep of stale sessions. Mount once in the app root.
 *
 * Status-aware: terminal sessions are removed quickly (30s), active sessions
 * are only removed after 24h (orphan safety net). This prevents the previous
 * bug where a 10-minute blanket sweep killed UI entries for still-running jobs.
 *
 * Law: an active session is never swept within orphanMaxAgeMs.
 */
export function useSessionCleanup(
  intervalMs = DEFAULT_SWEEP_INTERVAL_MS,
  terminalMaxAgeMs = DEFAULT_TERMINAL_MAX_AGE_MS,
  orphanMaxAgeMs = DEFAULT_ORPHAN_MAX_AGE_MS
) {
  useEffect(() => {
    const id = setInterval(() => {
      useSessionRegistry.getState().sweepStale(terminalMaxAgeMs, orphanMaxAgeMs);
    }, intervalMs);
    return () => clearInterval(id);
  }, [intervalMs, terminalMaxAgeMs, orphanMaxAgeMs]);
}
