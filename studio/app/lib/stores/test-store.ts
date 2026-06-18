/**
 * Test suite store with Zustand + Immer — multi-session builder chat.
 *
 * Manages the test case collection, filters, and the test case builder chat.
 *
 * Session-indexed: multiple builder chat sessions can exist concurrently.
 * Each builder session gets its own BuilderChatSession in `builderSessions`.
 * Switching away from the builder page preserves sessions.
 *
 * @module stores/test-store
 */

import { castDraft, enableMapSet } from "immer";
import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";
import { immer } from "zustand/middleware/immer";
import { useShallow } from "zustand/react/shallow";
import type { AsyncPhase } from "~/lib/domain/async-phase";
import { AsyncPhase as AP } from "~/lib/domain/async-phase";
import type { Message, MessageAccumulator, MessageId, MessagePart } from "~/lib/domain/message";
import {
  accumulatePart,
  createAccumulator,
  createUserMessage,
  finalizeAccumulator,
} from "~/lib/domain/message";
import type { StreamPart } from "~/lib/domain/stream-part";
import type {
  AuthoredTestCase,
  TestCaseBuilderSession,
  TestCaseStatus,
  ToolCatalogEntry,
} from "~/lib/domain/test-case";
import { useSessionRegistry } from "./session-registry";
import { useWorkspace } from "./workspace-store";

// Enable Immer's MapSet plugin for MessageAccumulator's pendingTools/pendingAgents
enableMapSet();

// =============================================================================
// State Types
// =============================================================================

export type TestCaseBuilderStreamingState =
  | { phase: "idle" }
  | { phase: "streaming"; startedAt: number }
  | { phase: "complete" };

/** Per-session builder chat state. */
export interface BuilderChatSession {
  sessionId: string;
  workspaceId: string;
  messages: Message[];
  accumulator: MessageAccumulator | null;
  streamPhase: TestCaseBuilderStreamingState;
  liveParts: MessagePart[];
  error: string | null;
}

export interface TestSuiteState {
  testCases: AuthoredTestCase[];
  selectedTestCaseId: string | null;
  filterStatus: TestCaseStatus | "all";
  filterTags: string[];
  loadPhase: AsyncPhase;
  // Test case builder (API session state)
  builderSession: TestCaseBuilderSession | null;
  availableTools: ToolCatalogEntry[];
  // Multi-session builder chat
  builderSessions: Map<string, BuilderChatSession>;
  focusedBuilderSessionId: string | null;
}

export interface TestSuiteActions {
  setTestCases: (cases: AuthoredTestCase[]) => void;
  addTestCase: (tc: AuthoredTestCase) => void;
  updateTestCase: (id: string, partial: Partial<AuthoredTestCase>) => void;
  replaceTestCase: (id: string, full: AuthoredTestCase) => void;
  removeTestCase: (id: string) => void;
  selectTestCase: (id: string | null) => void;
  setFilterStatus: (status: TestCaseStatus | "all") => void;
  setFilterTags: (tags: string[]) => void;
  setLoadPhase: (phase: AsyncPhase) => void;
  // Test case builder (API session state)
  setBuilderSession: (session: TestCaseBuilderSession | null) => void;
  setAvailableTools: (tools: ToolCatalogEntry[]) => void;
  // Builder chat — session-scoped
  startBuilderSession: () => string;
  adoptBuilderSession: (sessionId: string, workspaceId: string) => void;
  focusBuilderSession: (sessionId: string) => void;
  addBuilderUserMessage: (sessionId: string, content: string, id: MessageId, now: string) => void;
  startBuilderStreaming: (sessionId: string, messageId: MessageId, now?: number) => void;
  interpretBuilderStreamEvent: (sessionId: string, part: StreamPart) => void;
  finishBuilderStream: (sessionId: string, now?: string) => void;
  abortBuilderStream: (sessionId: string, now?: string) => void;
  setBuilderError: (sessionId: string, error: string | null) => void;
  resetBuilder: (sessionId: string) => void;
  reset: () => void;
}

export type TestSuiteStore = TestSuiteState & TestSuiteActions;

// =============================================================================
// Initial State
// =============================================================================

// Stable empty references for referential stability
const EMPTY_MESSAGES: Message[] = [];
const EMPTY_PARTS: MessagePart[] = [];

const initialState: TestSuiteState = {
  testCases: [],
  selectedTestCaseId: null,
  filterStatus: "all",
  filterTags: [],
  loadPhase: AP.idle,
  builderSession: null,
  availableTools: [],
  builderSessions: new Map(),
  focusedBuilderSessionId: null,
};

// =============================================================================
// Persistence Configuration
// =============================================================================

const MAX_PERSISTED_SESSIONS = 5;
const SESSION_TTL_MS = 24 * 60 * 60 * 1000; // 24 hours

/** Serialized form of a BuilderChatSession for localStorage. */
interface SerializedBuilderSession {
  sessionId: string;
  workspaceId: string;
  messages: Message[];
  persistedAt: number;
}

/** Serialize Map<string, BuilderChatSession> → array for JSON storage. */
function serializeBuilderSessions(
  sessions: Map<string, BuilderChatSession>
): SerializedBuilderSession[] {
  const entries: SerializedBuilderSession[] = [];
  const now = Date.now();
  for (const [, session] of sessions) {
    // Only persist sessions with messages (no ephemeral empty sessions)
    if (session.messages.length === 0) continue;
    entries.push({
      sessionId: session.sessionId,
      workspaceId: session.workspaceId,
      messages: session.messages,
      persistedAt: now,
    });
  }
  // Keep only the most recent sessions
  return entries.slice(-MAX_PERSISTED_SESSIONS);
}

/** Deserialize array → Map<string, BuilderChatSession>, pruning expired. */
function deserializeBuilderSessions(
  entries: SerializedBuilderSession[]
): Map<string, BuilderChatSession> {
  const now = Date.now();
  const map = new Map<string, BuilderChatSession>();
  for (const entry of entries) {
    // Skip expired sessions
    if (now - entry.persistedAt > SESSION_TTL_MS) continue;
    map.set(entry.sessionId, {
      sessionId: entry.sessionId,
      workspaceId: entry.workspaceId,
      messages: entry.messages,
      accumulator: null,
      streamPhase: { phase: "idle" },
      liveParts: EMPTY_PARTS,
      error: null,
    });
  }
  return map;
}

/**
 * Compute streaming parts from accumulator (same pattern as conversation store).
 */
function computeBuilderLiveParts(accumulator: MessageAccumulator | null): MessagePart[] {
  if (!accumulator) return EMPTY_PARTS;
  if (!accumulator.textBuffer) {
    return accumulator.parts.length > 0 ? accumulator.parts : EMPTY_PARTS;
  }
  const parts = [...accumulator.parts];
  parts.push({ type: "text", text: accumulator.textBuffer });
  return parts;
}

// =============================================================================
// Store
// =============================================================================

export const useTestSuite = create<TestSuiteStore>()(
  persist(
    immer((set) => ({
      ...initialState,

      setTestCases: (cases) =>
        set((state) => {
          state.testCases = castDraft(cases);
          state.loadPhase = AP.ready;
        }),

      addTestCase: (tc) =>
        set((state) => {
          state.testCases.unshift(castDraft(tc));
        }),

      updateTestCase: (id, partial) =>
        set((state) => {
          const idx = state.testCases.findIndex((tc) => tc.id === id);
          if (idx !== -1) {
            Object.assign(state.testCases[idx], partial);
          }
        }),

      replaceTestCase: (id, full) =>
        set((state) => {
          const idx = state.testCases.findIndex((tc) => tc.id === id);
          if (idx !== -1) {
            state.testCases[idx] = castDraft(full);
          }
        }),

      removeTestCase: (id) =>
        set((state) => {
          state.testCases = state.testCases.filter((tc) => tc.id !== id);
          if (state.selectedTestCaseId === id) {
            state.selectedTestCaseId = null;
          }
        }),

      selectTestCase: (id) =>
        set((state) => {
          state.selectedTestCaseId = id;
        }),

      setFilterStatus: (status) =>
        set((state) => {
          state.filterStatus = status;
        }),

      setFilterTags: (tags) =>
        set((state) => {
          state.filterTags = tags;
        }),

      setLoadPhase: (phase) =>
        set((state) => {
          state.loadPhase = phase;
        }),

      setBuilderSession: (session) =>
        set((state) => {
          state.builderSession = session ? castDraft(session) : null;
        }),

      setAvailableTools: (tools) =>
        set((state) => {
          state.availableTools = castDraft(tools);
        }),

      // ========================================================================
      // Test Case Builder Chat — Session-Indexed
      // ========================================================================

      startBuilderSession: () => {
        const sessionId = crypto.randomUUID();
        const workspaceId = useWorkspace.getState().activeWorkspaceId;
        const now = Date.now();

        set((state) => {
          const session: BuilderChatSession = {
            sessionId,
            workspaceId,
            messages: [],
            accumulator: null,
            streamPhase: { phase: "idle" },
            liveParts: EMPTY_PARTS,
            error: null,
          };
          state.builderSessions.set(sessionId, castDraft(session));
          state.focusedBuilderSessionId = sessionId;
        });

        // Effect: register with session registry (after pure state transition)
        useSessionRegistry.getState().register({
          id: `builder-${sessionId}`,
          kind: "builder",
          label: "Test Builder",
          workspaceId,
          startedAt: now,
          routeTo: "/tests/new",
        });

        return sessionId;
      },

      adoptBuilderSession: (sessionId, workspaceId) =>
        set((state) => {
          if (!state.builderSessions.has(sessionId)) {
            const session: BuilderChatSession = {
              sessionId,
              workspaceId,
              messages: [],
              accumulator: null,
              streamPhase: { phase: "idle" },
              liveParts: EMPTY_PARTS,
              error: null,
            };
            state.builderSessions.set(sessionId, castDraft(session));
          }
          state.focusedBuilderSessionId = sessionId;
        }),

      focusBuilderSession: (sessionId) =>
        set((state) => {
          if (state.builderSessions.has(sessionId)) {
            state.focusedBuilderSessionId = sessionId;
          }
        }),

      addBuilderUserMessage: (sessionId, content, id, now) => {
        const message = createUserMessage(id, content, now);
        set((state) => {
          const session = state.builderSessions.get(sessionId);
          if (!session) return;
          session.messages.push(castDraft(message));
        });
      },

      startBuilderStreaming: (sessionId, messageId, actualNow) => {
        const now = actualNow ?? Date.now();
        set((state) => {
          const session = state.builderSessions.get(sessionId);
          if (!session) return;
          session.streamPhase = { phase: "streaming", startedAt: now };
          session.accumulator = castDraft(createAccumulator(messageId));
          session.liveParts = castDraft(EMPTY_PARTS);
          session.error = null;
        });
      },

      interpretBuilderStreamEvent: (sessionId, part) =>
        set((state) => {
          const session = state.builderSessions.get(sessionId);
          if (!session?.accumulator) return;
          session.accumulator = castDraft(accumulatePart(session.accumulator, part));
          session.liveParts = castDraft(computeBuilderLiveParts(session.accumulator));
        }),

      finishBuilderStream: (sessionId, now) =>
        set((state) => {
          const session = state.builderSessions.get(sessionId);
          if (!session?.accumulator) return;
          const message = finalizeAccumulator(session.accumulator, now);
          session.messages.push(castDraft(message));
          session.accumulator = null;
          session.streamPhase = { phase: "complete" };
          session.liveParts = castDraft(EMPTY_PARTS);
        }),

      abortBuilderStream: (sessionId, now) =>
        set((state) => {
          const session = state.builderSessions.get(sessionId);
          if (!session) return;
          if (session.accumulator) {
            const message = finalizeAccumulator(session.accumulator, now, undefined, {
              pendingState: "cancelled",
            });
            session.messages.push(castDraft(message));
            session.accumulator = null;
          }
          session.streamPhase = { phase: "idle" };
          session.liveParts = castDraft(EMPTY_PARTS);
        }),

      setBuilderError: (sessionId, error) =>
        set((state) => {
          const session = state.builderSessions.get(sessionId);
          if (session) {
            session.error = error;
          }
        }),

      resetBuilder: (sessionId) => {
        set((state) => {
          state.builderSessions.delete(sessionId);
          if (state.focusedBuilderSessionId === sessionId) {
            state.focusedBuilderSessionId = null;
          }
        });
        // Effect: mark terminal in session registry (after pure state transition)
        useSessionRegistry.getState().markTerminal(`builder-${sessionId}`, "completed");
      },

      reset: () => set(() => ({ ...initialState })),
    })),
    {
      name: "studio-test-builder",
      version: 1,
      storage: createJSONStorage(() => localStorage, {
        replacer: (_key: string, value: unknown) => {
          if (value instanceof Map) {
            // Convert Map<string, BuilderChatSession> → serialized array
            return {
              __type: "BuilderSessionMap",
              entries: serializeBuilderSessions(value as Map<string, BuilderChatSession>),
            };
          }
          return value;
        },
        reviver: (_key: string, value: unknown) => {
          if (
            value &&
            typeof value === "object" &&
            (value as Record<string, unknown>).__type === "BuilderSessionMap"
          ) {
            return deserializeBuilderSessions(
              (value as { entries: SerializedBuilderSession[] }).entries
            );
          }
          return value;
        },
      }),
      partialize: (state) => ({
        builderSessions: state.builderSessions,
        focusedBuilderSessionId: state.focusedBuilderSessionId,
      }),
    }
  )
);

// =============================================================================
// State Selectors
// =============================================================================

export const selectTestCases = (state: TestSuiteStore) => state.testCases;
export const selectSelectedTestCaseId = (state: TestSuiteStore) => state.selectedTestCaseId;
export const selectTestFilterStatus = (state: TestSuiteStore) => state.filterStatus;
export const selectTestFilterTags = (state: TestSuiteStore) => state.filterTags;
export const selectTestLoadPhase = (state: TestSuiteStore) => state.loadPhase;
export const selectTestsLoading = (state: TestSuiteStore) => state.loadPhase.phase === "loading";
export const selectTestLoadError = (state: TestSuiteStore) =>
  state.loadPhase.phase === "failed" ? state.loadPhase.reason : null;
export const selectBuilderSession = (state: TestSuiteStore) => state.builderSession;
export const selectAvailableTools = (state: TestSuiteStore) => state.availableTools;
export const selectHasTestCases = (state: TestSuiteStore) => state.testCases.length > 0;
export const selectTestCaseCount = (state: TestSuiteStore) => state.testCases.length;

// Filtered test cases (derived)
export const selectFilteredTestCases = (state: TestSuiteStore) => {
  let cases = state.testCases;
  if (state.filterStatus !== "all") {
    cases = cases.filter((tc) => tc.status === state.filterStatus);
  }
  if (state.filterTags.length > 0) {
    cases = cases.filter((tc) => tc.tags.some((t) => state.filterTags.includes(t)));
  }
  return cases;
};

// =============================================================================
// Builder Chat Selectors — Backward-Compatible (derive from focused session)
// =============================================================================

export const selectBuilderSessionId = (state: TestSuiteStore): string | null =>
  state.focusedBuilderSessionId;

export const selectBuilderMessages = (state: TestSuiteStore): Message[] =>
  state.builderSessions.get(state.focusedBuilderSessionId ?? "")?.messages ?? EMPTY_MESSAGES;

export const selectBuilderLiveParts = (state: TestSuiteStore): MessagePart[] =>
  state.builderSessions.get(state.focusedBuilderSessionId ?? "")?.liveParts ?? EMPTY_PARTS;

export const selectBuilderIsStreaming = (state: TestSuiteStore): boolean => {
  const session = state.builderSessions.get(state.focusedBuilderSessionId ?? "");
  return session?.streamPhase.phase === "streaming" || false;
};

export const selectBuilderStreamPhase = (state: TestSuiteStore): TestCaseBuilderStreamingState =>
  state.builderSessions.get(state.focusedBuilderSessionId ?? "")?.streamPhase ?? { phase: "idle" };

export const selectBuilderError = (state: TestSuiteStore): string | null =>
  state.builderSessions.get(state.focusedBuilderSessionId ?? "")?.error ?? null;

export const selectBuilderHasMessages = (state: TestSuiteStore): boolean => {
  const session = state.builderSessions.get(state.focusedBuilderSessionId ?? "");
  return (session?.messages.length ?? 0) > 0;
};

// Multi-session selectors
export const selectBuilderSessions = (state: TestSuiteStore) => state.builderSessions;
export const selectBuilderSessionCount = (state: TestSuiteStore) => state.builderSessions.size;

// =============================================================================
// Action Bundles
// =============================================================================

export function useTestSuiteActions() {
  return useTestSuite(
    useShallow((s) => ({
      setTestCases: s.setTestCases,
      addTestCase: s.addTestCase,
      updateTestCase: s.updateTestCase,
      replaceTestCase: s.replaceTestCase,
      removeTestCase: s.removeTestCase,
      selectTestCase: s.selectTestCase,
      setFilterStatus: s.setFilterStatus,
      setFilterTags: s.setFilterTags,
      setLoadPhase: s.setLoadPhase,
      setBuilderSession: s.setBuilderSession,
      setAvailableTools: s.setAvailableTools,
      reset: s.reset,
    }))
  );
}

export function useBuilderChatActions() {
  return useTestSuite(
    useShallow((s) => ({
      startSession: s.startBuilderSession,
      adoptSession: s.adoptBuilderSession,
      focusSession: s.focusBuilderSession,
      sendMessage: s.addBuilderUserMessage,
      startStream: s.startBuilderStreaming,
      interpretEvent: s.interpretBuilderStreamEvent,
      finishStream: s.finishBuilderStream,
      abortStream: s.abortBuilderStream,
      setError: s.setBuilderError,
      resetBuilder: s.resetBuilder,
    }))
  );
}
