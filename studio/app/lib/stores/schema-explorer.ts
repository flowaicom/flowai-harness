/**
 * Schema explorer store — table discovery + column detail.
 *
 * @module stores/schema-explorer
 */

import { castDraft } from "immer";
import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import { useShallow } from "zustand/react/shallow";
import type { PhysicalTable, TableInfo } from "~/lib/domain/data";

// =============================================================================
// State
// =============================================================================

export interface SchemaExplorerState {
  tables: TableInfo[];
  selectedTableName: string | null;
  tableDetail: PhysicalTable | null;
}

export interface SchemaExplorerActions {
  setTables: (tables: TableInfo[]) => void;
  selectTable: (name: string | null) => void;
  setTableDetail: (detail: PhysicalTable | null) => void;
  reset: () => void;
}

export type SchemaExplorerStore = SchemaExplorerState & SchemaExplorerActions;

// =============================================================================
// Initial State
// =============================================================================

const EMPTY_TABLES: TableInfo[] = [];

const initialState: SchemaExplorerState = {
  tables: EMPTY_TABLES,
  selectedTableName: null,
  tableDetail: null,
};

// =============================================================================
// Store
// =============================================================================

export const useSchemaExplorer = create<SchemaExplorerStore>()(
  immer((set) => ({
    ...initialState,

    setTables: (tables) =>
      set((state) => {
        state.tables = castDraft(tables);
      }),

    selectTable: (name) =>
      set((state) => {
        state.selectedTableName = name;
        if (name === null) {
          state.tableDetail = null;
        }
      }),

    setTableDetail: (detail) =>
      set((state) => {
        state.tableDetail = detail ? castDraft(detail) : null;
      }),

    reset: () => set(initialState),
  }))
);

// =============================================================================
// Selectors
// =============================================================================

export const selectTables = (state: SchemaExplorerStore) => state.tables;
export const selectSelectedTableName = (state: SchemaExplorerStore) => state.selectedTableName;
export const selectTableDetail = (state: SchemaExplorerStore) => state.tableDetail;
export const selectTableCount = (state: SchemaExplorerStore) => state.tables.length;

// =============================================================================
// Action Bundle
// =============================================================================

export function useSchemaExplorerActions() {
  return useSchemaExplorer(
    useShallow((s) => ({
      setTables: s.setTables,
      selectTable: s.selectTable,
      setTableDetail: s.setTableDetail,
      reset: s.reset,
    }))
  );
}
