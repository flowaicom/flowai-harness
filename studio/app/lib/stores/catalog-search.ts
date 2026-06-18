/**
 * Catalog search store — search state for the data tab.
 *
 * @module stores/catalog-search
 */

import { castDraft } from "immer";
import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import { useShallow } from "zustand/react/shallow";
import type { DataSearchResult } from "~/lib/domain/data";

// =============================================================================
// State
// =============================================================================

export interface CatalogSearchState {
  query: string;
  results: DataSearchResult | null;
}

export interface CatalogSearchActions {
  setQuery: (query: string) => void;
  setResults: (results: DataSearchResult | null) => void;
  reset: () => void;
}

export type CatalogSearchStore = CatalogSearchState & CatalogSearchActions;

// =============================================================================
// Initial State
// =============================================================================

const initialState: CatalogSearchState = {
  query: "",
  results: null,
};

// =============================================================================
// Store
// =============================================================================

export const useCatalogSearch = create<CatalogSearchStore>()(
  immer((set) => ({
    ...initialState,

    setQuery: (query) =>
      set((state) => {
        state.query = query;
      }),

    setResults: (results) =>
      set((state) => {
        state.results = results ? castDraft(results) : null;
      }),

    reset: () => set(initialState),
  }))
);

// =============================================================================
// Selectors
// =============================================================================

export const selectSearchQuery = (state: CatalogSearchStore) => state.query;
export const selectSearchResults = (state: CatalogSearchStore) => state.results;

// =============================================================================
// Action Bundle
// =============================================================================

export function useCatalogSearchActions() {
  return useCatalogSearch(
    useShallow((s) => ({
      setQuery: s.setQuery,
      setResults: s.setResults,
      reset: s.reset,
    }))
  );
}
