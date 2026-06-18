/**
 * Thread store for managing conversation threads.
 *
 * Features:
 * - Thread list management
 * - localStorage persistence (namespaced by resourceId)
 * - Optimistic updates
 *
 * @module stores/thread-store
 */

import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";
import { immer } from "zustand/middleware/immer";
import { useShallow } from "zustand/react/shallow";
import type { ResourceId, Thread, ThreadId } from "~/lib/domain";
import {
  findThread,
  removeThreadFromList,
  sortThreadsByRecent,
  touchThread,
  updateThreadInList,
} from "~/lib/domain";
import type { AsyncPhase } from "~/lib/domain/async-phase";
import { AsyncPhase as AP } from "~/lib/domain/async-phase";

// ============================================================================
// State Types
// ============================================================================

/**
 * Thread store state.
 */
export interface ThreadsState {
  threads: Thread[];
  selectedThreadId: ThreadId | null;
  loadPhase: AsyncPhase;
  resourceId: ResourceId;
}

/**
 * Thread store actions.
 */
export interface ThreadsActions {
  // List management
  setThreads: (threads: Thread[]) => void;
  addThread: (thread: Thread) => void;
  updateThread: (id: ThreadId, update: Partial<Thread>) => void;
  removeThread: (id: ThreadId) => void;
  markThreadActive: (id: ThreadId) => void;

  // Selection
  selectThread: (id: ThreadId | null) => void;

  // State
  setLoadPhase: (phase: AsyncPhase) => void;
  setResourceId: (resourceId: ResourceId) => void;

  // Utilities
  getThread: (id: ThreadId) => Thread | undefined;
  reset: () => void;
}

export type ThreadsStore = ThreadsState & ThreadsActions;

// ============================================================================
// Initial State
// ============================================================================

const initialState: ThreadsState = {
  threads: [],
  selectedThreadId: null,
  loadPhase: AP.idle,
  resourceId: "dev-user",
};

// ============================================================================
// Store Implementation
// ============================================================================

export const useThreads = create<ThreadsStore>()(
  persist(
    immer((set, get) => ({
      ...initialState,

      // ======================================================================
      // List Management
      // ======================================================================

      setThreads: (threads) =>
        set((state) => {
          state.threads = sortThreadsByRecent(threads);
          state.loadPhase = AP.ready;
        }),

      addThread: (thread) =>
        set((state) => {
          // Add at the beginning (most recent)
          state.threads = [thread, ...state.threads];
        }),

      updateThread: (id, update) =>
        set((state) => {
          state.threads = updateThreadInList(state.threads, id, update);
        }),

      removeThread: (id) =>
        set((state) => {
          state.threads = removeThreadFromList(state.threads, id);
          // Clear selection if removed thread was selected
          if (state.selectedThreadId === id) {
            state.selectedThreadId = null;
          }
        }),

      markThreadActive: (id) =>
        set((state) => {
          const thread = findThread(state.threads, id);
          if (thread) {
            const updated = touchThread(thread);
            state.threads = sortThreadsByRecent(updateThreadInList(state.threads, id, updated));
          }
        }),

      // ======================================================================
      // Selection
      // ======================================================================

      selectThread: (id) =>
        set((state) => {
          state.selectedThreadId = id;
        }),

      // ======================================================================
      // State
      // ======================================================================

      setLoadPhase: (phase) =>
        set((state) => {
          state.loadPhase = phase;
        }),

      setResourceId: (resourceId) =>
        set((state) => {
          state.resourceId = resourceId;
          // Clear threads when resource changes (different tenant)
          state.threads = [];
          state.selectedThreadId = null;
          state.loadPhase = AP.idle;
        }),

      // ======================================================================
      // Utilities
      // ======================================================================

      getThread: (id) => findThread(get().threads, id),

      reset: () => set(initialState),
    })),
    {
      name: "studio-threads",
      storage: createJSONStorage(() => localStorage),
      partialize: (state) => ({
        // Only persist threads and resourceId
        threads: state.threads,
        resourceId: state.resourceId,
      }),
    }
  )
);

// ============================================================================
// State Selectors (Granular subscriptions)
// ============================================================================

export const selectThreads = (state: ThreadsStore) => state.threads;
export const selectSelectedThreadId = (state: ThreadsStore) => state.selectedThreadId;
export const selectThreadsLoadPhase = (state: ThreadsStore) => state.loadPhase;
export const selectThreadsLoading = (state: ThreadsStore) => state.loadPhase.phase === "loading";
export const selectThreadsReady = (state: ThreadsStore) => state.loadPhase.phase === "ready";
export const selectThreadLoadError = (state: ThreadsStore) =>
  state.loadPhase.phase === "failed" ? state.loadPhase.reason : null;
export const selectThreadResourceId = (state: ThreadsStore) => state.resourceId;

export const selectSelectedThread = (state: ThreadsStore): Thread | undefined =>
  state.selectedThreadId ? findThread(state.threads, state.selectedThreadId) : undefined;

/**
 * Memoized selector for recent threads (referential stability).
 * Caches result per (threads, limit) to avoid creating new arrays.
 */
const recentThreadsCache = new WeakMap<Thread[], Map<number, Thread[]>>();

export const selectRecentThreads =
  (limit: number) =>
  (state: ThreadsStore): Thread[] => {
    const threads = state.threads;

    // Fast path: if limit >= length, return the array directly
    if (limit >= threads.length) {
      return threads;
    }

    // Check cache
    let limitCache = recentThreadsCache.get(threads);
    if (limitCache) {
      const cached = limitCache.get(limit);
      if (cached) return cached;
    } else {
      limitCache = new Map();
      recentThreadsCache.set(threads, limitCache);
    }

    // Compute and cache
    const result = threads.slice(0, limit);
    limitCache.set(limit, result);
    return result;
  };

export const selectThreadCount = (state: ThreadsStore) => state.threads.length;
export const selectHasThreads = (state: ThreadsStore) => state.threads.length > 0;

// ============================================================================
// Action Bundles
// ============================================================================

/**
 * Thread list management actions.
 */
export function useThreadActions() {
  return useThreads(
    useShallow((s) => ({
      setThreads: s.setThreads,
      addThread: s.addThread,
      updateThread: s.updateThread,
      removeThread: s.removeThread,
      markThreadActive: s.markThreadActive,
      selectThread: s.selectThread,
      setLoadPhase: s.setLoadPhase,
      setResourceId: s.setResourceId,
      getThread: s.getThread,
      reset: s.reset,
    }))
  );
}

// ============================================================================
// Backward-Compatible Aliases (deprecated — use new names above)
// ============================================================================

// Plan names (ThreadCatalog vocabulary)
/** @deprecated Use ThreadsStore */
export type ThreadCatalog = ThreadsStore;
/** @deprecated Use ThreadsState */
export type ThreadCatalogState = ThreadsState;
/** @deprecated Use ThreadsActions */
export type ThreadCatalogActions = ThreadsActions;
/** @deprecated Use useThreads */
export const useThreadCatalog = useThreads;
/** @deprecated Use useThreadActions */
export const useThreadCatalogActions = useThreadActions;
/** @deprecated Use selectThreadsLoadPhase */
export const selectThreadLoadPhase = selectThreadsLoadPhase;

// Legacy names
/** @deprecated Use ThreadsActions */
export type ThreadActions = ThreadsActions;
/** @deprecated Use AsyncPhase from ~/lib/domain/async-phase */
export type ThreadListState = AsyncPhase;
/** @deprecated Use ThreadsState */
export type ThreadState = ThreadsState;
/** @deprecated Use ThreadsStore */
export type ThreadStore = ThreadsStore;
/** @deprecated Use useThreads */
export const useThreadStore = useThreads;

// Legacy selector aliases
/** @deprecated Use selectSelectedThreadId */
export const selectActiveThreadId = selectSelectedThreadId;
/** @deprecated Use selectThreadsLoadPhase */
export const selectListState = selectThreadsLoadPhase;
/** @deprecated Use selectSelectedThread */
export const selectActiveThread = selectSelectedThread;
/** @deprecated Use selectThreadLoadError */
export const selectError = selectThreadLoadError;
/** @deprecated Use selectThreadsLoading */
export const selectIsLoading = selectThreadsLoading;
/** @deprecated Use selectThreadsReady */
export const selectIsReady = selectThreadsReady;
/** @deprecated Use selectThreadResourceId */
export const selectResourceId = selectThreadResourceId;

// Legacy action selectors
/** @deprecated Use useThreadActions() */
export const selectSetThreads = (state: ThreadsStore) => state.setThreads;
/** @deprecated Use useThreadActions() */
export const selectAddThread = (state: ThreadsStore) => state.addThread;
/** @deprecated Use useThreadActions() */
export const selectUpdateThread = (state: ThreadsStore) => state.updateThread;
/** @deprecated Use useThreadActions() */
export const selectRemoveThread = (state: ThreadsStore) => state.removeThread;
/** @deprecated Use useThreadActions() */
export const selectTouchThread = (state: ThreadsStore) => state.markThreadActive;
/** @deprecated Use useThreadActions() */
export const selectSetActiveThread = (state: ThreadsStore) => state.selectThread;
/** @deprecated Use useThreadActions() */
export const selectSetListState = (state: ThreadsStore) => state.setLoadPhase;
/** @deprecated Use useThreadActions() */
export const selectSetResourceId = (state: ThreadsStore) => state.setResourceId;
/** @deprecated Use useThreadActions() */
export const selectGetThread = (state: ThreadsStore) => state.getThread;
/** @deprecated Use useThreadActions() */
export const selectResetThreads = (state: ThreadsStore) => state.reset;
