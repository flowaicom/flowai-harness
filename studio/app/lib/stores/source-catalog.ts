/**
 * Source catalog store — data source CRUD + connection health.
 *
 * @module stores/source-catalog
 */

import { castDraft, enableMapSet } from "immer";
import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import { useShallow } from "zustand/react/shallow";
import type { AsyncPhase } from "~/lib/domain/async-phase";
import { AsyncPhase as AP } from "~/lib/domain/async-phase";
import type { DataSource, DataSourceStatus } from "~/lib/domain/data";

enableMapSet();

// =============================================================================
// State
// =============================================================================

export interface SourceCatalogState {
  sources: DataSource[];
  selectedSourceId: string | null;
  connectionStatuses: Map<string, DataSourceStatus>;
  loadPhase: AsyncPhase;
}

export interface SourceCatalogActions {
  setSources: (sources: DataSource[]) => void;
  addSource: (source: DataSource) => void;
  updateSource: (source: DataSource) => void;
  removeSource: (id: string) => void;
  selectSource: (id: string | null) => void;
  setConnectionStatus: (id: string, status: DataSourceStatus) => void;
  setLoadPhase: (phase: AsyncPhase) => void;
  reset: () => void;
}

export type SourceCatalogStore = SourceCatalogState & SourceCatalogActions;

// =============================================================================
// Initial State
// =============================================================================

const EMPTY_SOURCES: DataSource[] = [];

const createInitialState = (): SourceCatalogState => ({
  sources: EMPTY_SOURCES,
  selectedSourceId: null,
  connectionStatuses: new Map(),
  loadPhase: AP.idle,
});

// =============================================================================
// Store
// =============================================================================

export const useSourceCatalog = create<SourceCatalogStore>()(
  immer((set) => ({
    ...createInitialState(),

    setSources: (sources) =>
      set((state) => {
        state.sources = castDraft(Array.isArray(sources) ? sources : []);
        state.loadPhase = AP.ready;
      }),

    addSource: (source) =>
      set((state) => {
        state.sources.unshift(castDraft(source));
      }),

    updateSource: (source) =>
      set((state) => {
        const idx = state.sources.findIndex((s) => s.id === source.id);
        if (idx !== -1) {
          state.sources[idx] = castDraft(source);
        }
      }),

    removeSource: (id) =>
      set((state) => {
        state.sources = state.sources.filter((s) => s.id !== id);
        if (state.selectedSourceId === id) {
          state.selectedSourceId = null;
        }
      }),

    selectSource: (id) =>
      set((state) => {
        state.selectedSourceId = id;
      }),

    setConnectionStatus: (id, status) =>
      set((state) => {
        state.connectionStatuses.set(id, castDraft(status));
      }),

    setLoadPhase: (phase) =>
      set((state) => {
        state.loadPhase = phase;
      }),

    reset: () => set(createInitialState()),
  }))
);

// =============================================================================
// Selectors
// =============================================================================

export const selectSources = (state: SourceCatalogStore) => state.sources;
export const selectSelectedSourceId = (state: SourceCatalogStore) => state.selectedSourceId;
export const selectConnectionStatuses = (state: SourceCatalogStore) => state.connectionStatuses;
export const selectSourcesLoadPhase = (state: SourceCatalogStore) => state.loadPhase;
export const selectSourcesLoading = (state: SourceCatalogStore) =>
  state.loadPhase.phase === "loading";

export const selectSelectedSource = (state: SourceCatalogStore) =>
  state.selectedSourceId
    ? (state.sources.find((s) => s.id === state.selectedSourceId) ?? null)
    : null;

export const selectSourceCount = (state: SourceCatalogStore) => state.sources.length;
export const selectHasSources = (state: SourceCatalogStore) => state.sources.length > 0;

// =============================================================================
// Action Bundle
// =============================================================================

export function useSourceCatalogActions() {
  return useSourceCatalog(
    useShallow((s) => ({
      setSources: s.setSources,
      addSource: s.addSource,
      updateSource: s.updateSource,
      removeSource: s.removeSource,
      selectSource: s.selectSource,
      setConnectionStatus: s.setConnectionStatus,
      setLoadPhase: s.setLoadPhase,
      reset: s.reset,
    }))
  );
}
