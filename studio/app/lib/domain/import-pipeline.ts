import type { ImportEvent, ImportStage, ImportSummary } from "./data";

export interface ImportSessionModel {
  readonly jobId: string;
  readonly isRunning: boolean;
  readonly importStage: ImportStage | null;
  readonly tableCounts: ReadonlyMap<string, number>;
  readonly batchRowsLoaded: number;
  readonly batchTotalRows: number;
  readonly importSummary: ImportSummary | null;
  readonly profilingTotal: number;
  readonly profilingCompleted: number;
  readonly profilingTerminalTables: readonly string[];
  readonly startedAt: number | null;
  readonly error: string | null;
}

export type ImportLifecycleEffect =
  | { readonly type: "profilingComplete"; readonly tableCount: number }
  | {
      readonly type: "importComplete";
      readonly tableCount: number;
      readonly totalRowCount: number;
    };

export interface ImportEventReduction {
  readonly session: ImportSessionModel;
  readonly effects: readonly ImportLifecycleEffect[];
}

export function createImportSessionModel(jobId: string, now: number): ImportSessionModel {
  return {
    jobId,
    isRunning: true,
    importStage: null,
    tableCounts: new Map(),
    batchRowsLoaded: 0,
    batchTotalRows: 0,
    importSummary: null,
    profilingTotal: 0,
    profilingCompleted: 0,
    profilingTerminalTables: [],
    startedAt: now,
    error: null,
  };
}

export function reduceImportSessionEvent(
  session: ImportSessionModel,
  event: ImportEvent
): ImportEventReduction {
  let next: ImportSessionModel = {
    ...session,
    tableCounts: new Map(session.tableCounts),
    profilingTerminalTables: [...session.profilingTerminalTables],
  };
  const effects: ImportLifecycleEffect[] = [];

  switch (event.type) {
    case "started":
      if (!next.isRunning) {
        next = { ...next, isRunning: true };
      }
      break;
    case "stageProgress": {
      const batchPatch =
        event.stage.stage === "loadingTables" && event.stage.currentBatchRows !== undefined
          ? {
              batchRowsLoaded: event.stage.currentBatchRows,
              batchTotalRows: event.stage.totalBatchRows ?? 0,
            }
          : {};
      next = { ...next, ...batchPatch, importStage: event.stage };
      break;
    }
    case "tableLoaded": {
      const tableCounts = new Map(next.tableCounts);
      tableCounts.set(event.tableName, event.rowCount);
      next = { ...next, tableCounts };
      break;
    }
    case "batchProgress":
      next = {
        ...next,
        batchRowsLoaded: event.rowsLoaded,
        batchTotalRows: event.totalRows,
      };
      break;
    case "schemaCreated":
    case "validationPassed":
    case "knowledgeExtractionStarted":
    case "documentExtracted":
      break;
    case "profilingEvent": {
      const inner = event.inner;
      if (inner.type === "progress") {
        const status = inner.status;
        if (status.status === "discovering") {
          next = { ...next, profilingTotal: status.tablesFound };
        }
      } else if (inner.type === "tableCompleted" || inner.type === "tableFailed") {
        if (!next.profilingTerminalTables.includes(inner.tableName)) {
          const terminalTables = [...next.profilingTerminalTables, inner.tableName];
          next = {
            ...next,
            profilingTerminalTables: terminalTables,
            profilingCompleted: terminalTables.length,
          };
        }
      } else if (inner.type === "completed" && next.importSummary) {
        next = {
          ...next,
          importStage: { stage: "completed", summary: next.importSummary },
        };
        effects.push({
          type: "profilingComplete",
          tableCount: next.profilingCompleted,
        });
      }
      break;
    }
    case "completed": {
      const tableCounts = Object.values(event.summary.tableRowCounts);
      next = {
        ...next,
        importSummary: event.summary,
        importStage: { stage: "completed", summary: event.summary },
      };
      effects.push({
        type: "importComplete",
        tableCount: Object.keys(event.summary.tableRowCounts).length,
        totalRowCount: tableCounts.reduce((a, b) => a + b, 0),
      });
      break;
    }
    case "error":
      next = {
        ...next,
        isRunning: false,
        error: event.message,
        importStage: { stage: "failed", error: event.message },
      };
      break;
    default:
      assertNever(event);
  }

  return { session: next, effects };
}

function assertNever(value: never): never {
  throw new Error(`Unhandled import event: ${JSON.stringify(value)}`);
}
