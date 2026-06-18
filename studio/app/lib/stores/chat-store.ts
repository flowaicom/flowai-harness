/**
 * Conversation store with Zustand + Immer — multi-session streaming.
 *
 * State machine for chat interactions:
 * - Idle -> Loading -> Ready | Failed (thread loading)
 * - Idle -> Streaming -> Complete (message streaming)
 *
 * Session-indexed: multiple threads can stream concurrently.
 * Each thread gets its own ChatStreamSession in `streamSessions`.
 * Switching threads preserves background streams.
 *
 * Design Principles:
 * - Store is pure data + transitions
 * - Effects are external (useEffect hooks)
 * - Selectors for granular subscriptions
 * - Action bundles for grouped consumption
 * - No side effects in reducers
 *
 * @module stores/chat-store
 */

import { castDraft, enableMapSet } from "immer";
import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import { useShallow } from "zustand/react/shallow";

// Enable Immer's MapSet plugin for MessageAccumulator's pendingTools/pendingAgents
enableMapSet();

import type {
  CostSummary,
  ToolTiming as DomainToolTiming,
  FinishReason,
  KVMetrics,
  Message,
  MessageAccumulator,
  MessageId,
  MessagePart,
  PhaseBreakdown,
  ResponseValidation,
  RetryEvent,
  StreamPart,
  TokenMetrics,
} from "~/lib/domain";
import {
  accumulatePart,
  createAccumulator,
  createUserMessage,
  finalizeAccumulator,
} from "~/lib/domain";
import type { AsyncPhase } from "~/lib/domain/async-phase";
import { AsyncPhase as AP } from "~/lib/domain/async-phase";

// ============================================================================
// State Types
// ============================================================================

/**
 * Streaming state machine.
 * Distinct from AsyncPhase because it carries timing data.
 */
export type StreamingState =
  | { phase: "idle" }
  | { phase: "streaming"; startedAt: number }
  | { phase: "complete"; finishedAt: number };

/**
 * Latency metrics for a single stream.
 */
export interface StreamLatency {
  readonly timeToFirstChunk: number | null;
  readonly timeToFirstToken: number | null;
  readonly totalDuration: number | null;
}

/**
 * Mutable store version of LatencySummary.
 *
 * Domain types are readonly (algebraic purity),
 * but store state must be mutable for Immer. This is the
 * boundary where we cross from pure domain to effectful state.
 */
export interface StoreLatencySummary {
  totalDurationMs: number;
  phases: PhaseBreakdown;
  toolTimings: DomainToolTiming[];
  /** KV store metrics (bytes read/written, operation counts) */
  kvMetrics?: KVMetrics;
  /** LLM token usage metrics */
  tokenMetrics?: TokenMetrics;
  /** Time to first token in milliseconds (streaming latency) */
  ttftMs?: number;
  /** Time to first text delta in milliseconds */
  firstTextMs?: number;
  productSetSize?: number;
  planPayloadBytes?: number;
  retryCount: number;
  /** Categorized retry events */
  retryEvents?: RetryEvent[];
  hadTimeout: boolean;
}

/**
 * Pre-computed streaming statistics.
 * Computed at mutation time to avoid selector instability.
 */
export interface StreamingStats {
  readonly textLength: number;
  readonly toolCallCount: number;
  readonly pendingToolCount: number;
  readonly completedToolCount: number;
  readonly hasReasoning: boolean;
}

// ============================================================================
// Session Types
// ============================================================================

/** Per-thread streaming session — one entry per concurrent stream. */
export interface ChatStreamSession {
  threadId: string;
  liveMessage: MessageAccumulator | null;
  streamPhase: StreamingState;
  latency: StreamLatency;
  tokenCost: CostSummary | null;
  serverMetrics: StoreLatencySummary | null;
  /** Finish reason from the stream (null until finish event received). */
  finishReason: FinishReason | null;
  /** Error message from a stream error event (null until error received). */
  streamError: string | null;
  responseValidation: ResponseValidation | null;
  liveParts: MessagePart[];
  liveStats: StreamingStats | null;
  abortController: AbortController | null;
}

// ============================================================================
// Store State
// ============================================================================

/**
 * Conversation store state.
 */
export interface ConversationState {
  // Thread context (focused thread)
  threadId: string | null;
  resourceId: string;

  // Messages (focused thread's history)
  history: Message[];

  // State machines
  loadPhase: AsyncPhase;

  // Session-indexed streaming state
  streamSessions: Map<string, ChatStreamSession>;

  // History cache (LRU, keyed by threadId)
  historyCache: Map<string, Message[]>;

  /**
   * File change counters (thread-scoped).
   *
   * Key: threadId
   * Value: Monotonic counter incremented when files are registered
   */
  fileChangeCounters: Record<string, number>;
}

/**
 * Conversation store actions.
 */
export interface ConversationActions {
  // Thread management — focus model
  setThreadId: (threadId: string | null) => void;
  setResourceId: (resourceId: string) => void;

  // Message management
  setMessages: (messages: Message[]) => void;
  addUserMessage: (content: string, id: MessageId, now: string) => void;
  clearHistory: () => void;

  // Thread loading state
  setLoadPhase: (state: AsyncPhase) => void;

  // Streaming — session-scoped
  startStreaming: (
    threadId: string,
    abortController: AbortController,
    messageId: MessageId,
    now?: number
  ) => void;
  interpretStreamEvent: (threadId: string, part: StreamPart) => void;
  finishStream: (threadId: string, now?: string, runId?: string) => void;
  abortStream: (threadId: string, now?: string) => void;

  // Latency tracking (client-side only - browser perspective)
  recordFirstChunk: (threadId: string) => void;
  recordFirstToken: (threadId: string) => void;

  // Cost & Latency
  setTokenCost: (threadId: string, summary: CostSummary) => void;
  setServerMetrics: (threadId: string, summary: StoreLatencySummary) => void;

  // File invalidation
  triggerFileChange: (threadId: string) => void;

  // Reset
  reset: () => void;
}

export type ConversationStore = ConversationState & ConversationActions;

// ============================================================================
// Initial State
// ============================================================================

// Stable references for SSR hydration (referential stability)
const EMPTY_MESSAGES: Message[] = [];
const EMPTY_PARTS: MessagePart[] = [];
const INITIAL_LATENCY: StreamLatency = {
  timeToFirstChunk: null,
  timeToFirstToken: null,
  totalDuration: null,
};
const INITIAL_STREAMING_STATE: StreamingState = { phase: "idle" };
const INITIAL_FILE_COUNTERS: Record<string, number> = {};
const MAX_HISTORY_CACHE = 5;

const initialState: ConversationState = {
  threadId: null,
  resourceId: "dev-user",
  history: EMPTY_MESSAGES,
  loadPhase: AP.idle,
  streamSessions: new Map(),
  historyCache: new Map(),
  fileChangeCounters: INITIAL_FILE_COUNTERS,
};

// ============================================================================
// Derived State Computation (compute at mutation boundaries)
// ============================================================================

/**
 * Compute streaming parts from accumulator.
 * Called at mutation time to maintain referential stability.
 */
function computeLiveParts(accumulator: MessageAccumulator | null): MessagePart[] {
  if (!accumulator) return EMPTY_PARTS;

  const hasPendingTools = accumulator.pendingTools.size > 0;
  const hasTextBuffer = !!accumulator.textBuffer;

  // If nothing extra to append, return parts directly (stable reference)
  if (!hasTextBuffer && !hasPendingTools) {
    return accumulator.parts.length > 0 ? accumulator.parts : EMPTY_PARTS;
  }

  const parts = [...accumulator.parts];

  // Include pending tool calls (shown as running/spinning)
  for (const tool of accumulator.pendingTools.values()) {
    parts.push(tool);
  }

  if (hasTextBuffer) {
    parts.push({ type: "text", text: accumulator.textBuffer });
  }

  return parts.length > 0 ? parts : EMPTY_PARTS;
}

/**
 * Compute streaming statistics from accumulator.
 * Called at mutation time to maintain referential stability.
 */
function computeLiveStats(accumulator: MessageAccumulator | null): StreamingStats | null {
  if (!accumulator) return null;

  const parts = accumulator.parts;
  let textLength = accumulator.textBuffer.length;
  let toolCallCount = 0;
  const pendingToolCount = accumulator.pendingTools.size;
  let completedToolCount = 0;
  let hasReasoning = false;

  for (const part of parts) {
    switch (part.type) {
      case "text":
        textLength += part.text.length;
        break;
      case "reasoning":
        hasReasoning = true;
        break;
      case "tool-invocation":
        toolCallCount++;
        if (part.state === "result") completedToolCount++;
        break;
    }
  }

  return {
    textLength,
    toolCallCount,
    pendingToolCount,
    completedToolCount,
    hasReasoning,
  };
}

// ============================================================================
// Helpers
// ============================================================================

function createStreamSession(threadId: string): ChatStreamSession {
  return {
    threadId,
    liveMessage: null,
    streamPhase: { phase: "idle" },
    latency: { ...INITIAL_LATENCY },
    tokenCost: null,
    serverMetrics: null,
    finishReason: null,
    streamError: null,
    responseValidation: null,
    liveParts: EMPTY_PARTS,
    liveStats: null,
    abortController: null,
  };
}

/** LRU eviction for history cache. */
function evictHistoryCache(cache: Map<string, Message[]>) {
  if (cache.size <= MAX_HISTORY_CACHE) return;
  // Delete oldest entry (first in iteration order)
  const firstKey = cache.keys().next().value;
  if (firstKey !== undefined) cache.delete(firstKey);
}

// ============================================================================
// Store Implementation
// ============================================================================

export const useConversation = create<ConversationStore>()(
  immer((set, _get) => ({
    ...initialState,

    // ========================================================================
    // Thread Management — Focus Model
    // ========================================================================

    setThreadId: (threadId) =>
      set((state) => {
        // 1. Cache current history (if non-empty)
        if (state.history.length > 0 && state.threadId) {
          state.historyCache.set(state.threadId, [...state.history]);
          evictHistoryCache(state.historyCache);
        }

        // 2. Update focus
        state.threadId = threadId;

        // 3. Restore from cache or mark for DB load
        if (threadId) {
          const cached = state.historyCache.get(threadId);
          if (cached) {
            state.history = [...cached]; // Defensive copy — isolate from cache mutations
            state.loadPhase = AP.ready;
          } else {
            state.history = [];
            state.loadPhase = AP.loading;
          }
        } else {
          state.history = [];
          state.loadPhase = AP.idle;
        }

        // IMPORTANT: DO NOT touch streamSessions — streams survive focus change
      }),

    setResourceId: (resourceId) =>
      set((state) => {
        state.resourceId = resourceId;
      }),

    // ========================================================================
    // Message Management
    // ========================================================================

    setMessages: (messages) =>
      set((state) => {
        state.history = castDraft(messages);
        state.loadPhase = AP.ready;
      }),

    addUserMessage: (content, id, now) => {
      const message = createUserMessage(id, content, now);
      set((state) => {
        state.history.push(castDraft(message));
      });
    },

    clearHistory: () =>
      set((state) => {
        state.history = [];
      }),

    // ========================================================================
    // Thread Loading State
    // ========================================================================

    setLoadPhase: (loadPhase) =>
      set((state) => {
        state.loadPhase = loadPhase;
      }),

    // ========================================================================
    // Streaming — Session-Indexed
    // ========================================================================

    startStreaming: (threadId, abortController, messageId, actualNow) => {
      const now = actualNow ?? Date.now();
      set((state) => {
        const session = createStreamSession(threadId);
        session.streamPhase = { phase: "streaming", startedAt: now };
        session.liveMessage = castDraft(createAccumulator(messageId));
        session.abortController = abortController;
        state.streamSessions.set(threadId, castDraft(session));
      });
    },

    interpretStreamEvent: (threadId, part) =>
      set((state) => {
        const session = state.streamSessions.get(threadId);
        if (!session?.liveMessage) return;

        // Update accumulator with new part
        session.liveMessage = castDraft(accumulatePart(session.liveMessage, part));

        // Handle cost summary
        if (part.type === "data-cost-summary") {
          session.tokenCost = part.data;
        }

        // Handle backend latency summary (backend owns metrics)
        if (part.type === "data-latency-summary") {
          session.serverMetrics = {
            totalDurationMs: part.data.totalDurationMs,
            phases: { ...part.data.phases },
            toolTimings: [...part.data.toolTimings],
            kvMetrics: part.data.kvMetrics ? { ...part.data.kvMetrics } : undefined,
            tokenMetrics: part.data.tokenMetrics ? { ...part.data.tokenMetrics } : undefined,
            ttftMs: part.data.ttftMs,
            firstTextMs: part.data.firstTextMs,
            productSetSize: part.data.productSetSize,
            planPayloadBytes: part.data.planPayloadBytes,
            retryCount: part.data.retryCount,
            retryEvents: part.data.retryEvents ? [...part.data.retryEvents] : undefined,
            hadTimeout: part.data.hadTimeout,
          };
        }

        // Handle finish reason (store for non-normal termination display)
        if (part.type === "finish") {
          session.finishReason = part.finishReason;
        }

        // Handle error events — track for session-level error state
        if (part.type === "error") {
          session.streamError = part.error.message;
        }

        if (
          part.type === "custom" &&
          part.name === "response-validation" &&
          part.data &&
          typeof part.data === "object"
        ) {
          session.responseValidation = castDraft(part.data as ResponseValidation);
        }

        // Handle file registration (push-based invalidation)
        if (part.type === "data-file-registered") {
          const fileThreadId = part.data.threadId;
          const current = state.fileChangeCounters[fileThreadId] ?? 0;
          state.fileChangeCounters[fileThreadId] = current + 1;
        }

        // Compute derived state at mutation boundary
        session.liveParts = castDraft(computeLiveParts(session.liveMessage));
        session.liveStats = computeLiveStats(session.liveMessage);
      }),

    finishStream: (threadId, now, runId) => {
      // Pre-compute timestamp before pure state transition
      const finishedAt = Date.now();

      set((state) => {
        const session = state.streamSessions.get(threadId);
        if (!session?.liveMessage) return;

        // Finalize the message
        const message = finalizeAccumulator(session.liveMessage, now, runId);
        const completedMessage = session.responseValidation
          ? { ...message, responseValidation: session.responseValidation }
          : message;

        if (threadId === state.threadId) {
          // Focused thread: push to visible history
          state.history.push(castDraft(completedMessage));
        } else {
          // Backgrounded: update cache if exists
          const cached = state.historyCache.get(threadId);
          if (cached) {
            cached.push(castDraft(completedMessage));
          }
          // Either way, DB is authoritative — will reload on focus
        }

        // Update streaming state
        const startedAt =
          session.streamPhase.phase === "streaming" ? session.streamPhase.startedAt : finishedAt;
        session.streamPhase = { phase: "complete", finishedAt };
        session.latency = {
          ...session.latency,
          totalDuration: finishedAt - startedAt,
        };
        session.liveMessage = null;
        session.abortController = null;
        session.responseValidation = null;
        session.liveParts = castDraft(EMPTY_PARTS);
        session.liveStats = null;

        // Remove session after completion (data is in history now)
        state.streamSessions.delete(threadId);
      });
    },

    abortStream: (threadId, now) => {
      // Mutable box: captures controller reference from inside set() for post-set() effect
      const box: { controller: AbortController | null } = { controller: null };

      set((state) => {
        const session = state.streamSessions.get(threadId);
        if (!session) return;

        // Capture the real (non-draft) controller for post-set() abort
        box.controller = session.abortController;

        // Finalize any partial message
        if (session.liveMessage) {
          const message = finalizeAccumulator(session.liveMessage, now, undefined, {
            pendingState: "cancelled",
          });
          if (threadId === state.threadId) {
            state.history.push(castDraft(message));
          } else {
            const cached = state.historyCache.get(threadId);
            if (cached) cached.push(castDraft(message));
          }
        }

        state.streamSessions.delete(threadId);
      });

      // Effects after pure state transition
      box.controller?.abort();
    },

    // ========================================================================
    // Latency Tracking
    // ========================================================================

    recordFirstChunk: (threadId) => {
      const now = Date.now();
      set((state) => {
        const session = state.streamSessions.get(threadId);
        if (
          session?.streamPhase.phase === "streaming" &&
          session.latency.timeToFirstChunk === null
        ) {
          session.latency = {
            ...session.latency,
            timeToFirstChunk: now - session.streamPhase.startedAt,
          };
        }
      });
    },

    recordFirstToken: (threadId) => {
      const now = Date.now();
      set((state) => {
        const session = state.streamSessions.get(threadId);
        if (
          session?.streamPhase.phase === "streaming" &&
          session.latency.timeToFirstToken === null
        ) {
          session.latency = {
            ...session.latency,
            timeToFirstToken: now - session.streamPhase.startedAt,
          };
        }
      });
    },

    // ========================================================================
    // Cost & Backend Latency
    // ========================================================================

    setTokenCost: (threadId, summary) =>
      set((state) => {
        const session = state.streamSessions.get(threadId);
        if (session) {
          session.tokenCost = summary;
        }
      }),

    setServerMetrics: (threadId, summary) =>
      set((state) => {
        const session = state.streamSessions.get(threadId);
        if (session) {
          session.serverMetrics = {
            ...summary,
            toolTimings: [...summary.toolTimings],
          };
        }
      }),

    // ========================================================================
    // File Invalidation
    // ========================================================================

    triggerFileChange: (threadId) =>
      set((state) => {
        const current = state.fileChangeCounters[threadId] ?? 0;
        state.fileChangeCounters[threadId] = current + 1;
      }),

    // ========================================================================
    // Reset
    // ========================================================================

    reset: () => set(initialState),
  }))
);

// ============================================================================
// State Selectors (granular subscriptions)
// ============================================================================

export const selectConversationThreadId = (state: ConversationStore) => state.threadId;
export const selectHistory = (state: ConversationStore) => state.history;
export const selectStreamSessions = (state: ConversationStore) => state.streamSessions;

// Thread loading state selectors
export const selectLoadPhase = (state: ConversationStore) => state.loadPhase;
export const selectConversationLoading = (state: ConversationStore) =>
  state.loadPhase.phase === "loading";
export const selectConversationReady = (state: ConversationStore) =>
  state.loadPhase.phase === "ready";
export const selectConversationError = (state: ConversationStore) =>
  state.loadPhase.phase === "failed" ? state.loadPhase.reason : null;

export const selectConversationResourceId = (state: ConversationStore) => state.resourceId;
export const selectFileChangeCounters = (state: ConversationStore) => state.fileChangeCounters;

/**
 * Select the change counter for a specific thread.
 */
export const selectFileChangeCounter = (threadId: string) => (state: ConversationStore) =>
  state.fileChangeCounters[threadId] ?? 0;

// ============================================================================
// Session-Scoped Selectors
// ============================================================================

/** Get the stream session for a specific thread. */
export const selectStreamSession =
  (threadId: string) =>
  (state: ConversationStore): ChatStreamSession | undefined =>
    state.streamSessions.get(threadId);

// ============================================================================
// Backward-Compatible Selectors (derive from focused thread's session)
// ============================================================================

/** Streaming state — checks focused thread's session. */
export const selectConversationIsStreaming = (state: ConversationStore): boolean => {
  if (!state.threadId) return false;
  return state.streamSessions.get(state.threadId)?.streamPhase.phase === "streaming";
};

/** Stream phase for focused thread. */
export const selectStreamPhase = (state: ConversationStore): StreamingState =>
  state.streamSessions.get(state.threadId ?? "")?.streamPhase ?? INITIAL_STREAMING_STATE;

/** Client latency for focused thread. */
export const selectClientLatency = (state: ConversationStore): StreamLatency =>
  state.streamSessions.get(state.threadId ?? "")?.latency ?? INITIAL_LATENCY;

/** Token cost for focused thread. */
export const selectTokenCost = (state: ConversationStore): CostSummary | null =>
  state.streamSessions.get(state.threadId ?? "")?.tokenCost ?? null;

/** Server metrics for focused thread. */
export const selectServerMetrics = (state: ConversationStore): StoreLatencySummary | null =>
  state.streamSessions.get(state.threadId ?? "")?.serverMetrics ?? null;

/** Finish reason for focused thread (null = no finish event yet or normal "stop"). */
export const selectFinishReason = (state: ConversationStore): FinishReason | null =>
  state.streamSessions.get(state.threadId ?? "")?.finishReason ?? null;

/** Abort controller for focused thread. */
export const selectConversationAbortController = (
  state: ConversationStore
): AbortController | null =>
  state.streamSessions.get(state.threadId ?? "")?.abortController ?? null;

/** Pre-computed live parts for focused thread. */
export const selectLiveParts = (state: ConversationStore): MessagePart[] =>
  state.streamSessions.get(state.threadId ?? "")?.liveParts ?? EMPTY_PARTS;

/** Pre-computed live stats for focused thread. */
export const selectLiveStats = (state: ConversationStore): StreamingStats | null =>
  state.streamSessions.get(state.threadId ?? "")?.liveStats ?? null;

/** Live message accumulator for focused thread. */
export const selectLiveMessage = (state: ConversationStore): MessageAccumulator | null =>
  state.streamSessions.get(state.threadId ?? "")?.liveMessage ?? null;

// ============================================================================
// Multi-Session Selectors
// ============================================================================

/** Any thread currently streaming? */
export const selectAnyThreadStreaming = (state: ConversationStore): boolean => {
  for (const session of state.streamSessions.values()) {
    if (session.streamPhase.phase === "streaming") return true;
  }
  return false;
};

/** Count of actively streaming threads. */
export const selectStreamingThreadCount = (state: ConversationStore): number => {
  let count = 0;
  for (const session of state.streamSessions.values()) {
    if (session.streamPhase.phase === "streaming") count++;
  }
  return count;
};

/** IDs of currently streaming threads. */
export const selectStreamingThreadIds = (state: ConversationStore): string[] => {
  const ids: string[] = [];
  for (const session of state.streamSessions.values()) {
    if (session.streamPhase.phase === "streaming") ids.push(session.threadId);
  }
  return ids;
};

// ============================================================================
// Computed Selectors
// ============================================================================

export const selectHistoryCount = (state: ConversationStore) => state.history.length;
export const selectHasHistory = (state: ConversationStore) => state.history.length > 0;
export const selectLastHistoryMessage = (state: ConversationStore) =>
  state.history.length > 0 ? state.history[state.history.length - 1] : null;
export const selectHasTokenCost = (state: ConversationStore) => selectTokenCost(state) !== null;
export const selectHasServerMetrics = (state: ConversationStore) =>
  selectServerMetrics(state) !== null;

// ============================================================================
// Action Bundles (replace individual action selectors)
// ============================================================================

/**
 * All conversation lifecycle actions.
 * Use in components that drive the send->stream->finish flow.
 */
export function useConversationActions() {
  return useConversation(
    useShallow((s) => ({
      sendMessage: s.addUserMessage,
      startStream: s.startStreaming,
      interpretEvent: s.interpretStreamEvent,
      finishStream: s.finishStream,
      abortStream: s.abortStream,
      loadThread: s.setThreadId,
      replaceHistory: s.setMessages,
      setLoadPhase: s.setLoadPhase,
      clearHistory: s.clearHistory,
      setResourceId: s.setResourceId,
      triggerFileChange: s.triggerFileChange,
      reset: s.reset,
    }))
  );
}

/**
 * Streaming metrics actions.
 * Use in components that track latency and cost.
 */
export function useStreamMetrics() {
  return useConversation(
    useShallow((s) => ({
      recordFirstChunk: s.recordFirstChunk,
      recordFirstToken: s.recordFirstToken,
      setTokenCost: s.setTokenCost,
      setServerMetrics: s.setServerMetrics,
    }))
  );
}

// ============================================================================
// Backward-Compatible Aliases (deprecated — use new names above)
// ============================================================================

/** @deprecated Use ConversationState */
export type ChatState = ConversationState;
/** @deprecated Use ConversationActions */
export type ChatActions = ConversationActions;
/** @deprecated Use ConversationStore */
export type ChatStore = ConversationStore;
/** @deprecated Use AsyncPhase from ~/lib/domain/async-phase */
export type ThreadLoadingState = AsyncPhase;

/** @deprecated Use useConversation */
export const useChatStore = useConversation;

// State selectors (old name -> new name)
/** @deprecated Use selectConversationThreadId */
export const selectThreadId = selectConversationThreadId;
/** @deprecated Use selectHistory */
export const selectMessages = selectHistory;
/** @deprecated Use selectStreamPhase */
export const selectStreamingState = selectStreamPhase;
/** @deprecated Use selectClientLatency */
export const selectLatency = selectClientLatency;
/** @deprecated Use selectClientLatency */
export const selectConversationLatency = selectClientLatency;
/** @deprecated Use selectTokenCost */
export const selectCostSummary = selectTokenCost;
/** @deprecated Use selectServerMetrics */
export const selectBackendLatency = selectServerMetrics;
/** @deprecated Use selectLoadPhase */
export const selectThreadLoading = selectLoadPhase;
/** @deprecated Use selectConversationLoading */
export const selectChatIsLoading = selectConversationLoading;
/** @deprecated Use selectConversationReady */
export const selectChatIsReady = selectConversationReady;
/** @deprecated Use selectConversationError */
export const selectChatError = selectConversationError;
/** @deprecated Use selectLiveParts */
export const selectStreamingParts = selectLiveParts;
/** @deprecated Use selectLiveStats */
export const selectStreamingStats = selectLiveStats;
/** @deprecated Use selectLiveMessage */
export const selectAccumulator = selectLiveMessage;
/** @deprecated Use selectFileChangeCounters */
export const selectFileInvalidationSignals = selectFileChangeCounters;
/** @deprecated Use selectFileChangeCounter */
export const selectFileInvalidationSignal = selectFileChangeCounter;
/** @deprecated Use selectHasTokenCost */
export const selectHasCostSummary = selectHasTokenCost;
/** @deprecated Use selectHasServerMetrics */
export const selectHasBackendLatency = selectHasServerMetrics;
/** @deprecated Use selectConversationResourceId */
export const selectResourceId = selectConversationResourceId;
/** @deprecated Use selectConversationIsStreaming */
export const selectIsStreaming = selectConversationIsStreaming;
/** @deprecated Use selectConversationAbortController */
export const selectAbortController = selectConversationAbortController;
/** @deprecated Use selectHistoryCount */
export const selectMessageCount = selectHistoryCount;
/** @deprecated Use selectHasHistory */
export const selectHasMessages = selectHasHistory;
/** @deprecated Use selectLastHistoryMessage */
export const selectLastMessage = selectLastHistoryMessage;

// Action selectors (old pattern — use action bundles instead)
/** @deprecated Use useConversationActions() */
export const selectSetThreadId = (state: ConversationStore) => state.setThreadId;
/** @deprecated Use useConversationActions() */
export const selectSetResourceId = (state: ConversationStore) => state.setResourceId;
/** @deprecated Use useConversationActions() */
export const selectSetMessages = (state: ConversationStore) => state.setMessages;
/** @deprecated Use useConversationActions() */
export const selectAddUserMessage = (state: ConversationStore) => state.addUserMessage;
/** @deprecated Use useConversationActions() */
export const selectClearMessages = (state: ConversationStore) => state.clearHistory;
/** @deprecated Use useConversationActions() */
export const selectSetThreadLoading = (state: ConversationStore) => state.setLoadPhase;
/** @deprecated Use useConversationActions() */
export const selectStartStreaming = (state: ConversationStore) => state.startStreaming;
/** @deprecated Use useConversationActions() */
export const selectHandleStreamPart = (state: ConversationStore) => state.interpretStreamEvent;
/** @deprecated Use useConversationActions() */
export const selectCompleteStreaming = (state: ConversationStore) => state.finishStream;
/** @deprecated Use useConversationActions() */
export const selectCancelStreaming = (state: ConversationStore) => state.abortStream;
/** @deprecated Use useStreamMetrics() */
export const selectRecordFirstChunk = (state: ConversationStore) => state.recordFirstChunk;
/** @deprecated Use useStreamMetrics() */
export const selectRecordFirstToken = (state: ConversationStore) => state.recordFirstToken;
/** @deprecated Use useStreamMetrics() */
export const selectSetCostSummary = (state: ConversationStore) => state.setTokenCost;
/** @deprecated Use useStreamMetrics() */
export const selectSetBackendLatency = (state: ConversationStore) => state.setServerMetrics;
/** @deprecated Use useConversationActions() */
export const selectTriggerFileInvalidation = (state: ConversationStore) => state.triggerFileChange;
/** @deprecated Use useConversationActions() */
export const selectReset = (state: ConversationStore) => state.reset;
