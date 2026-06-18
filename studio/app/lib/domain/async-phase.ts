/**
 * Shared async lifecycle vocabulary.
 *
 * Replaces 5 identical discriminated unions (DataListState, TestListState,
 * ThreadListState, WorkspaceListState, ThreadLoadingState) with one
 * shared type.
 *
 * Uses `phase` (not `status`) to avoid collision with HTTP/domain status.
 * Uses `failed` + `reason` (not `error`) to avoid collision with Error class.
 *
 * @module domain/async-phase
 */

export type AsyncPhase =
  | { phase: "idle" }
  | { phase: "loading" }
  | { phase: "ready" }
  | { phase: "failed"; reason: string };

/** Constructor helpers (stable references). */
export const AsyncPhase = {
  idle: { phase: "idle" } as const,
  loading: { phase: "loading" } as const,
  ready: { phase: "ready" } as const,
  failed: (reason: string): AsyncPhase => ({ phase: "failed", reason }),
} as const;
