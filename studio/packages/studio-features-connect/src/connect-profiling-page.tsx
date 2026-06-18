import {
  BarChart3Icon,
  BookOpenIcon,
  Loader2Icon,
  MessageSquareIcon,
  PlayIcon,
  RotateCcwIcon,
  SearchIcon,
  SquareIcon,
  XIcon,
} from "lucide-react";
import { memo, type ReactNode, useCallback, useMemo, useRef, useState } from "react";
import {
  ConnectEmptyState,
  ConnectErrorBanner,
  ConnectSectionCard,
  ConnectSectionHeader,
} from "./connect-page-primitives";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

function formatDuration(durationMs: number): string {
  if (durationMs < 1000) {
    return "0s";
  }
  const totalSeconds = Math.floor(durationMs / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes === 0) {
    return `${seconds}s`;
  }
  return `${minutes}m ${seconds}s`;
}

export type ConnectProfilingStatusKey =
  | "queued"
  | "discovering"
  | "profiling"
  | "enriching"
  | "extracting"
  | "indexing"
  | "completed"
  | "failed";

export type ConnectProfilingEnrichmentSource = "fresh" | "cached" | "fallback";
export type ConnectProfilingPipelineStageKey =
  | "discovering"
  | "profiling"
  | "enriching"
  | "extracting"
  | "indexing";
export type ConnectProfilingTableStageStatus = "queued" | "active" | "completed" | "failed";

export interface ConnectProfilingTableLike {
  readonly schemaName: string;
  readonly tableName: string;
  readonly tableType: string;
  readonly rowCount: number | null;
  readonly columnCount?: number | null;
}

export interface ConnectProfilingSummaryLike {
  readonly tablesDiscovered: number;
  readonly columnsProfiled: number;
  readonly enumsExtracted: number;
  readonly relationshipsFound: number;
  readonly catalogItemsIndexed: number;
  readonly durationMs: number;
  readonly enrichmentCacheHits?: number;
  readonly enrichmentFallbacks?: number;
  readonly enrichmentFresh?: number;
}

export interface ConnectProfilingTableStageLike {
  readonly tableName: string;
  readonly stages: Record<ConnectProfilingPipelineStageKey, ConnectProfilingTableStageStatus>;
  readonly columns: number;
  readonly durationMs: number;
  readonly enrichmentSource?: ConnectProfilingEnrichmentSource;
}

export interface ConnectProfilingPageProps {
  readonly tables: readonly ConnectProfilingTableLike[];
  readonly isLoading: boolean;
  readonly hasTarget: boolean;
  readonly isRunning: boolean;
  readonly isCompleted: boolean;
  readonly currentStatusKey: ConnectProfilingStatusKey;
  readonly reconnectingAttempt?: number | null;
  readonly maxReconnectAttempts?: number;
  readonly tableStages: ReadonlyMap<string, ConnectProfilingTableStageLike>;
  readonly discoveredTableNames: readonly string[];
  readonly currentTable: string | null;
  readonly totalTableCount: number;
  readonly completedTableCount: number;
  readonly elapsedMs: number;
  readonly error?: string | null;
  readonly onDismissError: () => void;
  readonly onRetryError?: () => void;
  readonly onProfileAll: () => void;
  readonly onProfileTable: (tableName: string) => void;
  readonly onCancel: () => void;
  readonly onClear: () => void;
  readonly canStartProfiling?: boolean;
  readonly summary?: ConnectProfilingSummaryLike | null;
  readonly controls?:
    | ReactNode
    | ((context: {
        readonly tableCount: number;
        readonly totalColumns: number;
        readonly isRunning: boolean;
        readonly isCompleted: boolean;
        readonly hasTarget: boolean;
      }) => ReactNode);
  readonly emptyState: {
    readonly title: string;
    readonly description: string;
    readonly action?: { readonly label: string; readonly onClick: () => void };
  };
  readonly nextSteps?: ReactNode;
  readonly subtitle?: ReactNode;
  readonly targetMeta?: ReactNode;
  readonly chatLinkLabel?: string;
  readonly formatText?: (value: string) => string;
}

const CONNECT_INGESTION_STATUS_COLORS: Record<ConnectProfilingStatusKey, string> = {
  queued: "var(--muted-foreground)",
  discovering: "var(--primary)",
  profiling: "var(--primary)",
  enriching: "var(--primary)",
  extracting: "var(--primary)",
  indexing: "var(--primary)",
  completed: "var(--dot-emerald)",
  failed: "var(--dot-red)",
};

const CONNECT_INGESTION_STATUS_LABELS: Record<ConnectProfilingStatusKey, string> = {
  queued: "Queued",
  discovering: "Discovering",
  profiling: "Profiling",
  enriching: "Enriching",
  extracting: "Extracting",
  indexing: "Indexing",
  completed: "Completed",
  failed: "Failed",
};

const PIPELINE_STAGES: readonly {
  readonly key: ConnectProfilingPipelineStageKey;
  readonly label: string;
}[] = [
  { key: "discovering", label: "Discover" },
  { key: "profiling", label: "Profile" },
  { key: "enriching", label: "Enrich" },
  { key: "extracting", label: "Extract" },
  { key: "indexing", label: "Index" },
];

const STAGE_STATUS_COLORS: Record<ConnectProfilingTableStageStatus, string> = {
  queued: CONNECT_INGESTION_STATUS_COLORS.queued,
  active: CONNECT_INGESTION_STATUS_COLORS.profiling,
  completed: CONNECT_INGESTION_STATUS_COLORS.completed,
  failed: CONNECT_INGESTION_STATUS_COLORS.failed,
};

const ENRICHMENT_DOT_COLOR: Record<ConnectProfilingEnrichmentSource, string> = {
  fresh: "var(--dot-blue)",
  cached: "var(--dot-emerald)",
  fallback: "var(--dot-amber)",
};

function ConnectProfilingStatusTag({ status }: { readonly status: ConnectProfilingStatusKey }) {
  return (
    <span
      className="inline-flex items-center rounded-full border px-2.5 py-1 text-xs font-medium"
      style={{
        color: CONNECT_INGESTION_STATUS_COLORS[status],
        borderColor: `${CONNECT_INGESTION_STATUS_COLORS[status]}33`,
        backgroundColor: `${CONNECT_INGESTION_STATUS_COLORS[status]}14`,
      }}
    >
      {CONNECT_INGESTION_STATUS_LABELS[status]}
    </span>
  );
}

function ConnectProfilingStatusDot({
  status,
  size = 8,
}: {
  readonly status: ConnectProfilingStatusKey;
  readonly size?: number;
}) {
  const isActive =
    status === "discovering" ||
    status === "profiling" ||
    status === "enriching" ||
    status === "extracting" ||
    status === "indexing";

  return (
    <span
      className={cx("inline-block rounded-full", isActive && "animate-pulse")}
      style={{
        width: size,
        height: size,
        backgroundColor: CONNECT_INGESTION_STATUS_COLORS[status],
      }}
    />
  );
}

function ProfilingMetricCell({
  value,
  label,
}: {
  readonly value: string | number;
  readonly label: string;
}) {
  return (
    <div>
      <p className="text-2xl font-semibold tabular-nums">
        {typeof value === "number" ? value.toLocaleString() : value}
      </p>
      <p className="text-xs text-muted-foreground">{label}</p>
    </div>
  );
}

function ConnectProfilingSummaryCards({
  summary,
}: {
  readonly summary: ConnectProfilingSummaryLike;
}) {
  const fresh = summary.enrichmentFresh ?? 0;
  const cacheHits = summary.enrichmentCacheHits ?? 0;
  const fallbacks = summary.enrichmentFallbacks ?? 0;
  const hasBreakdown = fresh > 0 || cacheHits > 0 || fallbacks > 0;

  return (
    <ConnectSectionCard>
      <ConnectSectionHeader>Summary</ConnectSectionHeader>
      <div className="grid grid-cols-3 gap-3">
        <ProfilingMetricCell value={summary.tablesDiscovered} label="Tables" />
        <ProfilingMetricCell value={summary.columnsProfiled} label="Columns" />
        <ProfilingMetricCell value={summary.enumsExtracted} label="Enums" />
        <ProfilingMetricCell value={summary.relationshipsFound} label="Relations" />
        <ProfilingMetricCell value={summary.catalogItemsIndexed} label="Catalog Items" />
        <ProfilingMetricCell value={formatDuration(summary.durationMs)} label="Duration" />
      </div>

      {hasBreakdown ? (
        <div className="mt-3 flex items-center gap-4 border-t pt-3 text-xs">
          {fresh > 0 ? (
            <span className="flex items-center gap-1.5 text-[var(--dot-blue)]">{fresh} fresh</span>
          ) : null}
          {cacheHits > 0 ? (
            <span className="flex items-center gap-1.5 text-[var(--dot-emerald)]">
              {cacheHits} cached
            </span>
          ) : null}
          {fallbacks > 0 ? (
            <span className="flex items-center gap-1.5 text-[var(--dot-amber)]">
              {fallbacks} fallback
            </span>
          ) : null}
        </div>
      ) : null}
    </ConnectSectionCard>
  );
}

function ConnectProfilingProgressBar({
  totalTables,
  completedTables,
  runningTables,
  failedTables,
  currentTable,
  elapsedMs,
  formatText,
}: {
  readonly totalTables: number;
  readonly completedTables: number;
  readonly runningTables: number;
  readonly failedTables: number;
  readonly currentTable: string | null;
  readonly elapsedMs: number;
  readonly formatText: (value: string) => string;
}) {
  const total = totalTables || 1;
  const pct = totalTables > 0 ? Math.round((completedTables / totalTables) * 100) : 0;
  const eta =
    completedTables > 0 && completedTables < totalTables
      ? Math.round((elapsedMs / completedTables) * (totalTables - completedTables))
      : null;

  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between text-xs">
        <span className="text-muted-foreground">
          {completedTables}/{totalTables} tables
        </span>
        <div className="flex items-center gap-3">
          {elapsedMs > 0 ? (
            <span className="text-muted-foreground">{formatDuration(elapsedMs)}</span>
          ) : null}
          {eta != null && eta > 0 ? (
            <span className="text-muted-foreground">~{formatDuration(eta)} remaining</span>
          ) : null}
          <span className="font-mono font-medium">{pct}%</span>
        </div>
      </div>

      <div
        className="flex h-2 overflow-hidden rounded-full bg-muted"
        role="progressbar"
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={`Profiling progress: ${completedTables} of ${totalTables} tables completed (${pct}%)`}
      >
        {completedTables > 0 ? (
          <div
            className="h-full transition-all duration-700 ease-out"
            style={{
              width: `${(completedTables / total) * 100}%`,
              backgroundColor: CONNECT_INGESTION_STATUS_COLORS.completed,
            }}
          />
        ) : null}
        {runningTables > 0 ? (
          <div
            className="h-full animate-pulse transition-all duration-700 ease-out"
            style={{
              width: `${(runningTables / total) * 100}%`,
              backgroundColor: CONNECT_INGESTION_STATUS_COLORS.profiling,
            }}
          />
        ) : null}
        {failedTables > 0 ? (
          <div
            className="h-full transition-all duration-700 ease-out"
            style={{
              width: `${(failedTables / total) * 100}%`,
              backgroundColor: CONNECT_INGESTION_STATUS_COLORS.failed,
            }}
          />
        ) : null}
      </div>

      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3 text-[10px] text-muted-foreground">
          {completedTables > 0 ? (
            <span className="flex items-center gap-1">
              <span
                className="h-2 w-2 rounded-full"
                style={{ backgroundColor: CONNECT_INGESTION_STATUS_COLORS.completed }}
              />
              Completed
            </span>
          ) : null}
          {runningTables > 0 ? (
            <span className="flex items-center gap-1">
              <span
                className="h-2 w-2 rounded-full"
                style={{ backgroundColor: CONNECT_INGESTION_STATUS_COLORS.profiling }}
              />
              Running
            </span>
          ) : null}
          {failedTables > 0 ? (
            <span className="flex items-center gap-1">
              <span
                className="h-2 w-2 rounded-full"
                style={{ backgroundColor: CONNECT_INGESTION_STATUS_COLORS.failed }}
              />
              Failed
            </span>
          ) : null}
          {totalTables - completedTables - runningTables - failedTables > 0 ? (
            <span className="flex items-center gap-1">
              <span
                className="h-2 w-2 rounded-full"
                style={{ backgroundColor: CONNECT_INGESTION_STATUS_COLORS.queued }}
              />
              Queued
            </span>
          ) : null}
        </div>

        {currentTable ? (
          <div className="max-w-[200px] truncate text-xs text-muted-foreground">
            Running: {formatText(currentTable)}
          </div>
        ) : null}
      </div>
    </div>
  );
}

type HoveredCell = {
  readonly tableName: string;
  readonly stage: ConnectProfilingPipelineStageKey;
  readonly rect: {
    readonly top: number;
    readonly left: number;
    readonly width: number;
    readonly height: number;
  };
  readonly entry: ConnectProfilingTableStageLike | undefined;
};

const MatrixCell = memo(
  function MatrixCell({
    stageStatus,
    tableName,
    stage,
    enrichmentSource,
    onMouseEnter,
    onMouseLeave,
  }: {
    readonly stageStatus: ConnectProfilingTableStageStatus;
    readonly tableName: string;
    readonly stage: ConnectProfilingPipelineStageKey;
    readonly enrichmentSource?: ConnectProfilingEnrichmentSource;
    readonly onMouseEnter?: (event: React.MouseEvent<HTMLButtonElement>) => void;
    readonly onMouseLeave?: () => void;
  }) {
    const showDot =
      stage === "enriching" && stageStatus === "completed" && enrichmentSource != null;

    return (
      <button
        type="button"
        aria-label={`${tableName}, ${stage}: ${stageStatus}`}
        onMouseEnter={onMouseEnter}
        onMouseLeave={onMouseLeave}
        className={cx(
          "relative h-5 w-5 cursor-default rounded-sm transition-shadow",
          stageStatus === "active" && "animate-pulse"
        )}
        style={{ backgroundColor: STAGE_STATUS_COLORS[stageStatus] }}
      >
        {showDot ? (
          <span className="absolute inset-0 flex items-center justify-center" aria-hidden="true">
            <span
              className="h-1.5 w-1.5 rounded-full"
              style={{ backgroundColor: ENRICHMENT_DOT_COLOR[enrichmentSource] }}
            />
          </span>
        ) : null}
      </button>
    );
  },
  (previous, next) =>
    previous.stageStatus === next.stageStatus && previous.enrichmentSource === next.enrichmentSource
);

function MatrixTooltip({
  hovered,
  containerRect,
  formatText,
}: {
  readonly hovered: HoveredCell;
  readonly containerRect: DOMRect;
  readonly formatText: (value: string) => string;
}) {
  const cellTop = hovered.rect.top - containerRect.top;
  const cellLeft = hovered.rect.left - containerRect.left;
  const stageLabel =
    PIPELINE_STAGES.find((stage) => stage.key === hovered.stage)?.label ?? hovered.stage;
  const stageStatus = hovered.entry?.stages[hovered.stage] ?? "queued";

  return (
    <div
      className="absolute z-30 pointer-events-none"
      style={{
        top: cellTop + hovered.rect.height + 6,
        left: Math.max(4, cellLeft - 80),
      }}
    >
      <div className="w-52 rounded-md border bg-popover p-2.5 text-xs text-popover-foreground shadow-lg">
        <div
          className="absolute -top-1.5 rotate-45 border-l border-t border-border bg-popover"
          style={{
            width: 10,
            height: 10,
            left: Math.min(80, cellLeft) + hovered.rect.width / 2 - 5,
          }}
        />
        <div className="relative space-y-1.5">
          <div className="flex items-center gap-1.5">
            <span
              className="h-2 w-2 rounded-full shrink-0"
              style={{ backgroundColor: STAGE_STATUS_COLORS[stageStatus] }}
            />
            <span className="font-medium capitalize">{stageStatus}</span>
            <span className="ml-auto text-muted-foreground">{stageLabel}</span>
          </div>

          <div className="truncate font-mono text-muted-foreground">
            {formatText(hovered.tableName)}
          </div>

          {hovered.entry && hovered.entry.durationMs != null ? (
            <div className="flex items-center gap-2 text-muted-foreground">
              <span>{hovered.entry.columns ?? 0} cols</span>
              <span>{(hovered.entry.durationMs / 1000).toFixed(1)}s</span>
            </div>
          ) : null}

          {hovered.stage === "enriching" && hovered.entry?.enrichmentSource ? (
            <div className="flex items-center gap-1.5 text-muted-foreground">
              <span
                className="h-2 w-2 rounded-full shrink-0"
                style={{ backgroundColor: ENRICHMENT_DOT_COLOR[hovered.entry.enrichmentSource] }}
              />
              <span className="capitalize">{hovered.entry.enrichmentSource}</span>
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function ConnectProfilingMatrix({
  tableNames,
  tableStages,
  currentTable,
  formatText,
}: {
  readonly tableNames: readonly string[];
  readonly tableStages: ReadonlyMap<string, ConnectProfilingTableStageLike>;
  readonly currentTable: string | null;
  readonly formatText: (value: string) => string;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const leaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [hoveredCell, setHoveredCell] = useState<HoveredCell | null>(null);

  const handleCellMouseEnter = useCallback(
    (
      event: React.MouseEvent<HTMLButtonElement>,
      tableName: string,
      stage: ConnectProfilingPipelineStageKey,
      entry: ConnectProfilingTableStageLike | undefined
    ) => {
      if (leaveTimerRef.current) {
        clearTimeout(leaveTimerRef.current);
        leaveTimerRef.current = null;
      }
      const rect = event.currentTarget.getBoundingClientRect();
      setHoveredCell({
        tableName,
        stage,
        rect: { top: rect.top, left: rect.left, width: rect.width, height: rect.height },
        entry,
      });
    },
    []
  );

  const handleCellMouseLeave = useCallback(() => {
    leaveTimerRef.current = setTimeout(() => {
      setHoveredCell(null);
      leaveTimerRef.current = null;
    }, 100);
  }, []);

  if (tableNames.length === 0) {
    return (
      <div className="py-4 text-center text-sm text-muted-foreground">No tables profiled yet</div>
    );
  }

  const containerRect = scrollRef.current?.getBoundingClientRect();

  return (
    <div ref={scrollRef} className="relative max-h-[480px] overflow-auto">
      <div className="sticky top-0 z-10 flex items-center gap-1 bg-background pb-1">
        <div className="sticky left-0 z-20 w-[140px] shrink-0 truncate bg-background text-xs font-medium text-muted-foreground">
          Table
        </div>
        {PIPELINE_STAGES.map((stage) => (
          <div
            key={stage.key}
            className="w-5 shrink-0 text-center text-xs text-muted-foreground"
            title={stage.label}
          >
            {stage.label.slice(0, 3)}
          </div>
        ))}
      </div>

      <div className="space-y-0.5">
        {tableNames.map((tableName) => {
          const entry = tableStages.get(tableName);
          const isCurrentRow = currentTable === tableName;

          return (
            <div key={tableName} className="flex items-center gap-1">
              <div
                className={cx(
                  "sticky left-0 z-10 flex w-[140px] shrink-0 items-center gap-1 truncate bg-background text-xs",
                  isCurrentRow && "font-medium text-foreground"
                )}
                title={formatText(tableName)}
              >
                {isCurrentRow ? (
                  <span
                    className="inline-block h-2 w-2 shrink-0 animate-pulse rounded-full"
                    style={{ backgroundColor: CONNECT_INGESTION_STATUS_COLORS.profiling }}
                  />
                ) : null}
                {formatText(tableName)}
              </div>

              {PIPELINE_STAGES.map((stage) => (
                <MatrixCell
                  key={stage.key}
                  stageStatus={entry?.stages[stage.key] ?? "queued"}
                  tableName={tableName}
                  stage={stage.key}
                  enrichmentSource={entry?.enrichmentSource}
                  onMouseEnter={(event) => handleCellMouseEnter(event, tableName, stage.key, entry)}
                  onMouseLeave={handleCellMouseLeave}
                />
              ))}
            </div>
          );
        })}
      </div>

      {hoveredCell && containerRect ? (
        <MatrixTooltip
          hovered={hoveredCell}
          containerRect={containerRect}
          formatText={formatText}
        />
      ) : null}
    </div>
  );
}

function ConnectProfilingMatrixSkeleton({ rows }: { readonly rows: number }) {
  return (
    <div className="space-y-1">
      <div className="flex items-center gap-1">
        <div className="h-4 w-[140px] animate-pulse rounded bg-muted" />
        {PIPELINE_STAGES.map((stage) => (
          <div key={stage.key} className="h-4 w-5 animate-pulse rounded bg-muted" />
        ))}
      </div>
      {Array.from({ length: rows }, (_, rowIndex) => (
        <div key={`profiling-skeleton-${rowIndex}`} className="flex items-center gap-1">
          <div className="h-4 w-24 animate-pulse rounded bg-muted" />
          {PIPELINE_STAGES.map((stage) => (
            <div
              key={`${stage.key}-${rowIndex}`}
              className="h-5 w-5 animate-pulse rounded-sm bg-muted"
            />
          ))}
        </div>
      ))}
    </div>
  );
}

function renderControls(
  controls: ConnectProfilingPageProps["controls"],
  context: {
    readonly tableCount: number;
    readonly totalColumns: number;
    readonly isRunning: boolean;
    readonly isCompleted: boolean;
    readonly hasTarget: boolean;
  }
) {
  return typeof controls === "function" ? controls(context) : controls;
}

export function ConnectProfilingPage({
  tables,
  isLoading,
  hasTarget,
  isRunning,
  isCompleted,
  currentStatusKey,
  reconnectingAttempt = null,
  maxReconnectAttempts = 5,
  tableStages,
  discoveredTableNames,
  currentTable,
  totalTableCount,
  completedTableCount,
  elapsedMs,
  error = null,
  onDismissError,
  onRetryError,
  onProfileAll,
  onProfileTable,
  onCancel,
  onClear,
  canStartProfiling = true,
  summary = null,
  controls,
  emptyState,
  nextSteps,
  subtitle = "Profile tables to discover column statistics, semantic types, and relationships",
  targetMeta,
  formatText = (value) => value,
}: ConnectProfilingPageProps) {
  const hasMatrixData = discoveredTableNames.length > 0;
  const runningTables = isRunning && currentTable ? 1 : 0;
  const failedTables = useMemo(() => {
    let count = 0;
    for (const entry of tableStages.values()) {
      if (Object.values(entry.stages).some((status) => status === "failed")) {
        count += 1;
      }
    }
    return count;
  }, [tableStages]);
  const totalColumns = useMemo(
    () => tables.reduce((sum, table) => sum + (table.columnCount ?? 12), 0),
    [tables]
  );

  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      <div className="flex items-center justify-between border-b px-6 py-4">
        <div>
          <h1 className="text-lg font-semibold">Profiling</h1>
          <p className="text-sm text-muted-foreground">{subtitle}</p>
          {targetMeta ? <p className="mt-1 text-xs text-muted-foreground">{targetMeta}</p> : null}
        </div>
        <div className="flex items-center gap-2">
          {!isRunning && !isCompleted ? (
            <button
              type="button"
              onClick={onProfileAll}
              disabled={!canStartProfiling}
              className="flex items-center gap-2 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-50"
            >
              {isLoading ? (
                <Loader2Icon className="size-4 animate-spin" />
              ) : (
                <PlayIcon className="size-4" />
              )}
              Profile All
            </button>
          ) : null}

          {isCompleted ? (
            <>
              <button
                type="button"
                onClick={onProfileAll}
                className="flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-sm transition-colors hover:bg-muted"
              >
                <RotateCcwIcon className="size-3.5" />
                Re-profile
              </button>
              <button
                type="button"
                onClick={onClear}
                className="flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-sm transition-colors hover:bg-muted"
              >
                <XIcon className="size-3.5" />
                Clear
              </button>
            </>
          ) : null}

          {isRunning ? (
            <button
              type="button"
              onClick={onCancel}
              className="flex items-center gap-1.5 rounded-md border border-destructive/50 px-3 py-1.5 text-sm text-destructive transition-colors hover:bg-destructive/10"
            >
              <SquareIcon className="size-3.5" />
              Stop
            </button>
          ) : null}

          <ConnectProfilingStatusTag status={currentStatusKey} />
        </div>
      </div>

      {reconnectingAttempt !== null ? (
        <div className="flex items-center gap-2 border-b bg-amber-500/10 px-6 py-2 text-xs text-amber-700 dark:text-amber-400">
          <Loader2Icon className="size-3.5 animate-spin" />
          Reconnecting to profiling stream (attempt {reconnectingAttempt}/{maxReconnectAttempts})
          ...
        </div>
      ) : null}

      {!isRunning && !isCompleted && tables.length > 0 && controls ? (
        <div className="border-b px-6 py-3">
          {renderControls(controls, {
            tableCount: tables.length,
            totalColumns,
            isRunning,
            isCompleted,
            hasTarget,
          })}
        </div>
      ) : null}

      {(isRunning || hasMatrixData) && totalTableCount > 0 ? (
        <div className="sticky top-0 z-10 border-b bg-background px-6 py-3">
          <ConnectProfilingProgressBar
            totalTables={totalTableCount}
            completedTables={completedTableCount}
            runningTables={runningTables}
            failedTables={failedTables}
            currentTable={currentTable}
            elapsedMs={elapsedMs}
            formatText={formatText}
          />
        </div>
      ) : null}

      <div className="flex-1 overflow-y-auto scroll-container">
        <div className="mx-auto max-w-5xl space-y-6 p-6">
          {error ? (
            <ConnectErrorBanner message={error} onDismiss={onDismissError} onRetry={onRetryError} />
          ) : null}

          {hasMatrixData ? (
            <ConnectSectionCard>
              <ConnectSectionHeader>Pipeline</ConnectSectionHeader>
              <ConnectProfilingMatrix
                tableNames={discoveredTableNames}
                tableStages={tableStages}
                currentTable={currentTable}
                formatText={formatText}
              />
            </ConnectSectionCard>
          ) : null}

          {isCompleted && summary ? (
            <>
              <ConnectProfilingSummaryCards summary={summary} />
              {nextSteps ? (
                <ConnectSectionCard>
                  <ConnectSectionHeader>Next Steps</ConnectSectionHeader>
                  {nextSteps}
                </ConnectSectionCard>
              ) : (
                <ConnectSectionCard>
                  <ConnectSectionHeader>Next Steps</ConnectSectionHeader>
                  <div className="flex flex-wrap gap-2">
                    <button
                      type="button"
                      className="inline-flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-xs font-medium transition-colors hover:bg-muted"
                    >
                      <BookOpenIcon className="size-3.5" />
                      Browse Knowledge
                    </button>
                    <button
                      type="button"
                      className="inline-flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-xs font-medium transition-colors hover:bg-muted"
                    >
                      <SearchIcon className="size-3.5" />
                      Search Data
                    </button>
                    <button
                      type="button"
                      className="inline-flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary/90"
                    >
                      <MessageSquareIcon className="size-3.5" />
                      Open Chat
                    </button>
                  </div>
                </ConnectSectionCard>
              )}
            </>
          ) : null}

          {isLoading ? (
            <ConnectSectionCard>
              <ConnectSectionHeader>Tables</ConnectSectionHeader>
              <ConnectProfilingMatrixSkeleton rows={6} />
            </ConnectSectionCard>
          ) : tables.length === 0 ? (
            <ConnectEmptyState
              icon={BarChart3Icon}
              title={emptyState.title}
              description={emptyState.description}
              action={emptyState.action}
            />
          ) : (
            <ConnectSectionCard>
              <ConnectSectionHeader>Tables ({tables.length})</ConnectSectionHeader>
              <div className="divide-y">
                {tables.map((table) => {
                  const stageEntry = tableStages.get(table.tableName);
                  const hasFailedStage = stageEntry
                    ? Object.values(stageEntry.stages).some((status) => status === "failed")
                    : false;
                  const allCompleted = stageEntry
                    ? Object.values(stageEntry.stages).every((status) => status === "completed")
                    : false;
                  const dotStatus = hasFailedStage
                    ? "failed"
                    : allCompleted
                      ? "completed"
                      : "profiling";

                  return (
                    <div
                      key={`${table.schemaName}.${table.tableName}`}
                      className="flex items-center gap-3 py-2.5 first:pt-0 last:pb-0"
                    >
                      <div className="flex w-5 shrink-0 justify-center">
                        {stageEntry ? (
                          <ConnectProfilingStatusDot status={dotStatus} size={8} />
                        ) : null}
                      </div>

                      <div className="min-w-0 flex-1">
                        <span className="font-mono text-xs">{formatText(table.tableName)}</span>
                        <span className="ml-2 text-xs text-muted-foreground">
                          {table.tableType}
                        </span>
                      </div>

                      <span className="shrink-0 font-mono text-xs tabular-nums text-muted-foreground">
                        {table.rowCount?.toLocaleString() ?? "-"} rows
                      </span>

                      <button
                        type="button"
                        onClick={() => onProfileTable(table.tableName)}
                        disabled={isRunning}
                        className={cx(
                          "shrink-0 rounded px-2 py-1 text-xs transition-colors",
                          "text-muted-foreground hover:bg-muted hover:text-foreground",
                          "disabled:cursor-not-allowed disabled:opacity-40"
                        )}
                        aria-label={`Profile ${table.tableName}`}
                      >
                        Profile
                      </button>
                    </div>
                  );
                })}
              </div>
            </ConnectSectionCard>
          )}
        </div>
      </div>
    </div>
  );
}
