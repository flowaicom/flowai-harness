/**
 * Evaluation store with Zustand + Immer — multi-session.
 *
 * State machine for eval interactions:
 * - Idle -> Loading runs
 * - Idle -> Running eval -> Paused -> Running -> Complete
 *
 * Session-indexed: multiple evals can run concurrently.
 * Each eval run gets its own EvalSession in `evalSessions`.
 *
 * Design Principles:
 * - Store is pure data + transitions
 * - Effects are external (useEffect hooks)
 * - Selectors for granular subscriptions
 * - interpretEvalEvent is the interpreter (exhaustive switch)
 *
 * @module stores/eval-store
 */

import { castDraft, enableMapSet } from "immer";
import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import { useShallow } from "zustand/react/shallow";
import type {
  EvalConfig,
  EvalEvent,
  EvalProgress,
  EvalRun,
  EvalRunSummary,
  RunPhase,
  TestCaseResult,
  TestCaseSet,
  TestCaseState,
} from "~/lib/domain/eval";
import { DEFAULT_EVAL_CONFIG, extractSampleScore } from "~/lib/domain/eval";
import { lifecycleBus } from "./lifecycle-bus";
import { useSessionRegistry } from "./session-registry";
import { useWorkspace } from "./workspace-store";

// Enable Immer's MapSet plugin (eval state uses Map<string, ...>)
enableMapSet();

// castDraft from immer: official API for crossing the readonly→mutable boundary.

// =============================================================================
// Session Types
// =============================================================================

/** Per-eval session state — replaces singleton runPhase/liveResults/etc. */
export interface EvalSession {
  evalId: string;
  runPhase: RunPhase;
  liveProgress: EvalProgress | null;
  liveResults: Map<string, TestCaseResult>;
  testCaseStates: Map<string, TestCaseState>;
  skippedTestCases: Map<string, string>;
  /** High-water mark: last processed event seq from SequencedBus.
   *  Enables idempotent event processing on SSE reconnection replay. */
  highWaterMark: number;
  /** Set-based deduplication: tracks processed seq numbers to handle
   *  out-of-order events from SSE reconnection. Trimmed when >1000 entries. */
  processedSeqs: Set<number>;
}

// =============================================================================
// State Types
// =============================================================================

export interface EvaluationState {
  /** Lightweight summaries for the list view (no results array). */
  runs: EvalRunSummary[];
  /** Full run with results — loaded on-demand for detail page. */
  detailRun: EvalRun | null;
  activeRunId: string | null;
  testCaseSets: TestCaseSet[];
  /** Session-indexed eval state — one entry per concurrent eval run. */
  evalSessions: Map<string, EvalSession>;
  error: string | null;
  configDraft: EvalConfig;
}

export interface EvaluationActions {
  // Data loading
  setRuns: (runs: EvalRunSummary[]) => void;
  setDetailRun: (run: EvalRun | null) => void;
  setTestCaseSets: (sets: TestCaseSet[]) => void;
  addRun: (run: EvalRunSummary) => void;
  removeRun: (id: string) => void;

  // Selection
  selectRun: (id: string | null) => void;

  // Config
  updateConfigDraft: (partial: Partial<EvalConfig>) => void;
  resetConfigDraft: () => void;

  // Streaming (interpreter pattern) — session-scoped
  startEval: (runId: string) => void;
  interpretEvalEvent: (evalId: string, event: EvalEvent) => void;
  completeEval: (evalId: string) => void;
  cancelEval: (evalId: string) => void;

  // Error
  setError: (error: string | null) => void;

  // Reset
  reset: () => void;
}

export type EvaluationStore = EvaluationState & EvaluationActions;

// =============================================================================
// Initial State
// =============================================================================

const EMPTY_RUNS: EvalRunSummary[] = [];
const EMPTY_SETS: TestCaseSet[] = [];

const initialState: EvaluationState = {
  runs: EMPTY_RUNS,
  detailRun: null,
  activeRunId: null,
  testCaseSets: EMPTY_SETS,
  evalSessions: new Map(),
  error: null,
  configDraft: { ...DEFAULT_EVAL_CONFIG },
};

// =============================================================================
// Helpers
// =============================================================================

function createEvalSession(evalId: string): EvalSession {
  return {
    evalId,
    runPhase: { phase: "running", activeEvalId: evalId },
    liveProgress: null,
    liveResults: new Map(),
    testCaseStates: new Map(),
    skippedTestCases: new Map(),
    highWaterMark: -1,
    processedSeqs: new Set(),
  };
}

// =============================================================================
// Store
// =============================================================================

export const useEvaluation = create<EvaluationStore>()(
  immer((set) => ({
    ...initialState,

    // ========================================================================
    // Data Loading
    // ========================================================================

    setRuns: (runs) =>
      set((state) => {
        state.runs = castDraft(runs);
      }),

    setDetailRun: (run) =>
      set((state) => {
        state.detailRun = run ? castDraft(run) : null;
      }),

    setTestCaseSets: (sets) =>
      set((state) => {
        state.testCaseSets = castDraft(sets);
      }),

    addRun: (run) =>
      set((state) => {
        state.runs.unshift(castDraft(run));
      }),

    removeRun: (id) =>
      set((state) => {
        state.runs = state.runs.filter((r) => r.id !== id);
        if (state.activeRunId === id) {
          state.activeRunId = null;
        }
      }),

    // ========================================================================
    // Selection
    // ========================================================================

    selectRun: (id) =>
      set((state) => {
        state.activeRunId = id;
      }),

    // ========================================================================
    // Config Draft
    // ========================================================================

    updateConfigDraft: (partial) =>
      set((state) => {
        Object.assign(state.configDraft, partial);
      }),

    resetConfigDraft: () =>
      set((state) => {
        state.configDraft = castDraft({ ...DEFAULT_EVAL_CONFIG });
      }),

    // ========================================================================
    // Streaming — Session-Indexed Interpreter
    // ========================================================================

    startEval: (runId) => {
      const now = Date.now();
      set((state) => {
        state.evalSessions.set(runId, castDraft(createEvalSession(runId)));
        state.error = null;
      });
      // Effects after pure state transition
      const workspaceId = useWorkspace.getState().activeWorkspaceId;
      useSessionRegistry.getState().register({
        id: runId,
        kind: "eval-run",
        label: `Eval ${runId.slice(0, 8)}`,
        workspaceId,
        startedAt: now,
        routeTo: `/evals/${runId}`,
        jobId: runId,
      });
    },

    interpretEvalEvent: (evalId, event) => {
      // Unwrap childProgress synchronously — avoid queueMicrotask TOCTOU
      const effectiveEvent =
        event.type === "childProgress" && event.event.type !== "childProgress"
          ? event.event
          : event;

      // Pre-compute non-deterministic values before pure state transition
      const now = new Date().toISOString();

      // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: store event handler
      set((state) => {
        const session = state.evalSessions.get(evalId);
        if (!session) return;

        // Idempotent event processing: skip events already processed.
        // Events from SequencedBus carry a `seq` field; use set-based deduplication
        // to handle out-of-order events from SSE reconnection replay.
        const seq = (event as Record<string, unknown>).seq;
        if (typeof seq === "number") {
          if (session.processedSeqs.has(seq)) {
            return; // Already processed — idempotent skip
          }
          session.processedSeqs.add(seq);
          // Update high-water mark for backward compatibility
          if (seq > session.highWaterMark) {
            session.highWaterMark = seq;
          }
          // Trim set to prevent memory growth (keep only recent entries)
          if (session.processedSeqs.size > 1000) {
            const sorted = Array.from(session.processedSeqs).sort((a, b) => a - b);
            const toRemove = sorted.slice(0, sorted.length - 500);
            for (const s of toRemove) {
              session.processedSeqs.delete(s);
            }
          }
        }

        switch (effectiveEvent.type) {
          case "started":
            session.runPhase = castDraft({ phase: "running", activeEvalId: effectiveEvent.runId });
            break;

          case "progress":
            session.liveProgress = castDraft(effectiveEvent.progress);
            for (const entry of effectiveEvent.progress.testCaseStates) {
              session.testCaseStates.set(entry.testCaseId, castDraft(entry.state));
            }
            break;

          case "testCaseStarted":
            session.testCaseStates.set(
              effectiveEvent.testCaseId,
              castDraft({
                state: "running" as const,
                startedAtMs: 0,
                completedSamples: 0,
                totalSamples: 0,
              })
            );
            break;

          case "sampleProgress":
            session.testCaseStates.set(
              effectiveEvent.testCaseId,
              castDraft({
                state: "running" as const,
                startedAtMs: 0,
                completedSamples: effectiveEvent.completedSamples,
                totalSamples: effectiveEvent.totalSamples,
              })
            );
            break;

          case "sampleComplete": {
            const tcId = effectiveEvent.testCaseId;
            const existing = session.liveResults.get(tcId);
            if (existing) {
              const updatedSamples = [...existing.samples, effectiveEvent.sample];
              session.liveResults.set(
                tcId,
                castDraft({
                  ...existing,
                  samples: updatedSamples,
                  aggregateScore:
                    updatedSamples.reduce((sum, s) => sum + extractSampleScore(s), 0) /
                    updatedSamples.length,
                })
              );
            } else {
              session.liveResults.set(
                tcId,
                castDraft({
                  testCaseId: tcId,
                  samples: [effectiveEvent.sample],
                  passAtK: [],
                  aggregateScore: extractSampleScore(effectiveEvent.sample),
                })
              );
            }
            break;
          }

          case "testCaseComplete": {
            const result = effectiveEvent.result;
            session.liveResults.set(result.testCaseId, castDraft(result));
            break;
          }

          case "testCaseSkipped": {
            session.skippedTestCases.set(effectiveEvent.testCaseId, effectiveEvent.reason);
            break;
          }

          case "completed": {
            const idx = state.runs.findIndex((r) => r.id === evalId);
            if (idx !== -1) {
              state.runs[idx] = castDraft({
                ...state.runs[idx],
                status: { status: "completed", summary: effectiveEvent.summary },
                resultCount: session.liveResults.size,
                updatedAt: now,
              });
            }

            if (state.detailRun && state.detailRun.id === evalId) {
              state.detailRun.status = castDraft({
                status: "completed",
                summary: effectiveEvent.summary,
              });
              state.detailRun.results = castDraft(Array.from(session.liveResults.values()));
              state.detailRun.updatedAt = now;
            }

            session.runPhase = castDraft({ phase: "idle" });
            break;
          }

          case "paused": {
            if (session.runPhase.phase === "running" || session.runPhase.phase === "paused") {
              session.runPhase = castDraft({
                phase: "paused",
                activeEvalId: evalId,
              });
            }
            session.liveProgress = castDraft(effectiveEvent.progress);
            break;
          }

          case "resumed": {
            if (session.runPhase.phase === "paused" || session.runPhase.phase === "running") {
              session.runPhase = castDraft({
                phase: "running",
                activeEvalId: evalId,
              });
            }
            session.liveProgress = castDraft(effectiveEvent.progress);
            break;
          }

          case "error":
            session.runPhase = castDraft({ phase: "error", message: effectiveEvent.message });
            state.error = effectiveEvent.message;
            break;

          // childProgress with nested childProgress: drop (recursion guard handled above)
          case "childProgress":
            break;
        }
      });

      // Effects after pure state transition — no side effects inside set()
      if (effectiveEvent.type === "completed") {
        const workspaceId = useWorkspace.getState().activeWorkspaceId;
        lifecycleBus.emit({
          type: "evalCompleted",
          evalId,
          passRate:
            effectiveEvent.summary.totalTestCases > 0
              ? effectiveEvent.summary.passed / effectiveEvent.summary.totalTestCases
              : 0,
          totalCases: effectiveEvent.summary.totalTestCases ?? 0,
          workspaceId,
        });
      }
    },

    completeEval: (evalId) => {
      set((state) => {
        state.evalSessions.delete(evalId);
      });
      useSessionRegistry.getState().markTerminal(evalId, "completed");
    },

    cancelEval: (evalId) => {
      set((state) => {
        state.evalSessions.delete(evalId);
      });
      useSessionRegistry.getState().markTerminal(evalId, "cancelled");
    },

    // ========================================================================
    // Error & Reset
    // ========================================================================

    setError: (error) =>
      set((state) => {
        state.error = error;
      }),

    reset: () =>
      set(() => ({
        ...initialState,
        evalSessions: new Map(),
        configDraft: { ...DEFAULT_EVAL_CONFIG },
      })),
  }))
);

// =============================================================================
// State Selectors
// =============================================================================

export const selectEvalRuns = (state: EvaluationStore) => state.runs;
export const selectEvalDetailRun = (state: EvaluationStore) => state.detailRun;
export const selectEvalActiveRunId = (state: EvaluationStore) => state.activeRunId;
export const selectTestCaseSets = (state: EvaluationStore) => state.testCaseSets;
export const selectEvalError = (state: EvaluationStore) => state.error;
export const selectEvalConfigDraft = (state: EvaluationStore) => state.configDraft;
export const selectEvalSessions = (state: EvaluationStore) => state.evalSessions;

// =============================================================================
// Session-Scoped Selectors
// =============================================================================

/** Get the full session for a specific eval run. */
export const selectEvalSession =
  (evalId: string) =>
  (state: EvaluationStore): EvalSession | undefined =>
    state.evalSessions.get(evalId);

/** Check if a specific eval is actively running. */
export const selectEvalSessionRunning =
  (evalId: string) =>
  (state: EvaluationStore): boolean =>
    state.evalSessions.get(evalId)?.runPhase.phase === "running";

/** Check if a specific eval is paused. */
export const selectEvalSessionPaused =
  (evalId: string) =>
  (state: EvaluationStore): boolean =>
    state.evalSessions.get(evalId)?.runPhase.phase === "paused";

/** Get live progress for a specific eval. */
export const selectEvalSessionProgress =
  (evalId: string) =>
  (state: EvaluationStore): EvalProgress | null =>
    state.evalSessions.get(evalId)?.liveProgress ?? null;

/** Get live results for a specific eval. */
export const selectEvalSessionResults =
  (evalId: string) =>
  (state: EvaluationStore): Map<string, TestCaseResult> =>
    state.evalSessions.get(evalId)?.liveResults ?? EMPTY_RESULTS;

/** Get test case states for a specific eval. */
export const selectEvalSessionTestCaseStates =
  (evalId: string) =>
  (state: EvaluationStore): Map<string, TestCaseState> =>
    state.evalSessions.get(evalId)?.testCaseStates ?? EMPTY_TC_STATES;

/** Get skipped test cases for a specific eval. */
export const selectEvalSessionSkipped =
  (evalId: string) =>
  (state: EvaluationStore): Map<string, string> =>
    state.evalSessions.get(evalId)?.skippedTestCases ?? EMPTY_SKIPPED;

// Stable empty references
const EMPTY_RESULTS: Map<string, TestCaseResult> = new Map();
const EMPTY_TC_STATES: Map<string, TestCaseState> = new Map();
const EMPTY_SKIPPED: Map<string, string> = new Map();

// =============================================================================
// Multi-Session Computed Selectors
// =============================================================================

/** Count of currently running/paused eval sessions. */
export const selectRunningEvalCount = (state: EvaluationStore): number => {
  let count = 0;
  for (const session of state.evalSessions.values()) {
    if (session.runPhase.phase === "running" || session.runPhase.phase === "paused") count++;
  }
  return count;
};

/** IDs of currently running eval sessions. */
export const selectRunningEvalIds = (state: EvaluationStore): string[] => {
  const ids: string[] = [];
  for (const session of state.evalSessions.values()) {
    if (session.runPhase.phase === "running" || session.runPhase.phase === "paused") {
      ids.push(session.evalId);
    }
  }
  return ids;
};

/** Check if a specific eval has an active session (running, paused, or has data). */
export const selectHasEvalSession =
  (evalId: string) =>
  (state: EvaluationStore): boolean =>
    state.evalSessions.has(evalId);

export const selectActiveEvalRun = (state: EvaluationStore) => {
  if (!state.activeRunId) return null;
  if (state.detailRun?.id === state.activeRunId) return state.detailRun;
  return state.runs.find((r) => r.id === state.activeRunId) ?? null;
};

export const selectEvalRunCount = (state: EvaluationStore) => state.runs.length;
export const selectHasEvalRuns = (state: EvaluationStore) => state.runs.length > 0;

// =============================================================================
// Action Bundles
// =============================================================================

export function useEvaluationActions() {
  return useEvaluation(
    useShallow((s) => ({
      setRuns: s.setRuns,
      setDetailRun: s.setDetailRun,
      setTestCaseSets: s.setTestCaseSets,
      addRun: s.addRun,
      removeRun: s.removeRun,
      selectRun: s.selectRun,
      updateConfigDraft: s.updateConfigDraft,
      resetConfigDraft: s.resetConfigDraft,
      startEval: s.startEval,
      interpretEvalEvent: s.interpretEvalEvent,
      completeEval: s.completeEval,
      cancelEval: s.cancelEval,
      setError: s.setError,
      reset: s.reset,
    }))
  );
}

export function useEvaluationData() {
  return useEvaluation(
    useShallow((s) => ({
      runs: s.runs,
      detailRun: s.detailRun,
      activeRunId: s.activeRunId,
      testCaseSets: s.testCaseSets,
      evalSessions: s.evalSessions,
      error: s.error,
      configDraft: s.configDraft,
    }))
  );
}
