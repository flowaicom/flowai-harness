/**
 * Knowledge base store — documents, knowledge items, and metrics.
 *
 * @module stores/knowledge-base
 */

import { castDraft } from "immer";
import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import { useShallow } from "zustand/react/shallow";
import type { DocumentItem, KnowledgeItem, MetricItem } from "~/lib/domain/data";

// =============================================================================
// State
// =============================================================================

export interface KnowledgeBaseState {
  documents: DocumentItem[];
  knowledgeItems: KnowledgeItem[];
  selectedDocumentId: string | null;
  metrics: MetricItem[];
}

export interface KnowledgeBaseActions {
  setDocuments: (docs: DocumentItem[]) => void;
  addDocument: (doc: DocumentItem) => void;
  removeDocument: (id: string) => void;
  selectDocument: (id: string | null) => void;
  setKnowledgeItems: (items: KnowledgeItem[]) => void;
  addKnowledgeItem: (item: KnowledgeItem) => void;
  updateKnowledgeItem: (item: KnowledgeItem) => void;
  removeKnowledgeItem: (id: string) => void;
  setMetrics: (metrics: MetricItem[]) => void;
  addMetric: (metric: MetricItem) => void;
  updateMetric: (metric: MetricItem) => void;
  removeMetric: (id: string) => void;
  reset: () => void;
}

export type KnowledgeBaseStore = KnowledgeBaseState & KnowledgeBaseActions;

// =============================================================================
// Initial State
// =============================================================================

const EMPTY_DOCS: DocumentItem[] = [];
const EMPTY_KNOWLEDGE: KnowledgeItem[] = [];
const EMPTY_METRICS: MetricItem[] = [];

const initialState: KnowledgeBaseState = {
  documents: EMPTY_DOCS,
  knowledgeItems: EMPTY_KNOWLEDGE,
  selectedDocumentId: null,
  metrics: EMPTY_METRICS,
};

// =============================================================================
// Store
// =============================================================================

export const useKnowledgeBase = create<KnowledgeBaseStore>()(
  immer((set) => ({
    ...initialState,

    setDocuments: (docs) =>
      set((state) => {
        state.documents = castDraft(docs);
      }),

    addDocument: (doc) =>
      set((state) => {
        state.documents.unshift(castDraft(doc));
      }),

    removeDocument: (id) =>
      set((state) => {
        state.documents = state.documents.filter((d) => d.id !== id);
        if (state.selectedDocumentId === id) {
          state.selectedDocumentId = null;
        }
      }),

    selectDocument: (id) =>
      set((state) => {
        state.selectedDocumentId = id;
      }),

    setKnowledgeItems: (items) =>
      set((state) => {
        state.knowledgeItems = castDraft(items);
      }),

    addKnowledgeItem: (item) =>
      set((state) => {
        state.knowledgeItems.unshift(castDraft(item));
      }),

    updateKnowledgeItem: (item) =>
      set((state) => {
        const idx = state.knowledgeItems.findIndex((k) => k.id === item.id);
        if (idx !== -1) {
          state.knowledgeItems[idx] = castDraft(item);
        }
      }),

    removeKnowledgeItem: (id) =>
      set((state) => {
        state.knowledgeItems = state.knowledgeItems.filter((k) => k.id !== id);
      }),

    setMetrics: (metrics) =>
      set((state) => {
        state.metrics = castDraft(metrics);
      }),

    addMetric: (metric) =>
      set((state) => {
        state.metrics.unshift(castDraft(metric));
      }),

    updateMetric: (metric) =>
      set((state) => {
        const idx = state.metrics.findIndex((m) => m.id === metric.id);
        if (idx !== -1) {
          state.metrics[idx] = castDraft(metric);
        }
      }),

    removeMetric: (id) =>
      set((state) => {
        state.metrics = state.metrics.filter((m) => m.id !== id);
      }),

    reset: () => set(initialState),
  }))
);

// =============================================================================
// Selectors
// =============================================================================

export const selectDocuments = (state: KnowledgeBaseStore) => state.documents;
export const selectKnowledgeItems = (state: KnowledgeBaseStore) => state.knowledgeItems;
export const selectSelectedDocumentId = (state: KnowledgeBaseStore) => state.selectedDocumentId;
export const selectMetrics = (state: KnowledgeBaseStore) => state.metrics;
export const selectDocumentCount = (state: KnowledgeBaseStore) => state.documents.length;
export const selectKnowledgeCount = (state: KnowledgeBaseStore) => state.knowledgeItems.length;
export const selectMetricCount = (state: KnowledgeBaseStore) => state.metrics.length;

// =============================================================================
// Action Bundle
// =============================================================================

export function useKnowledgeBaseActions() {
  return useKnowledgeBase(
    useShallow((s) => ({
      setDocuments: s.setDocuments,
      addDocument: s.addDocument,
      removeDocument: s.removeDocument,
      selectDocument: s.selectDocument,
      setKnowledgeItems: s.setKnowledgeItems,
      addKnowledgeItem: s.addKnowledgeItem,
      updateKnowledgeItem: s.updateKnowledgeItem,
      removeKnowledgeItem: s.removeKnowledgeItem,
      setMetrics: s.setMetrics,
      addMetric: s.addMetric,
      updateMetric: s.updateMetric,
      removeMetric: s.removeMetric,
      reset: s.reset,
    }))
  );
}
