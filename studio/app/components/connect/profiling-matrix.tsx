/**
 * Profiling pipeline matrix (partition matrix).
 *
 * Rows = tables, Columns = 5 fixed pipeline stages.
 * Each cell: 20x20 rounded square colored by stage status.
 * Tooltip on hover, selected cell highlight, running shimmer.
 *
 * @module components/data/profiling-matrix
 */

import { memo, useCallback, useRef, useState } from "react";
import {
  type EnrichmentSource,
  INGESTION_STATUS_COLORS,
  PIPELINE_STAGES,
  type PipelineStageKey,
  type TablePipelineState,
  type TableStageStatus,
} from "~/lib/domain/data";
import { useScramble } from "~/lib/scramble";
import { cn } from "~/lib/utils";

// =============================================================================
// Types
// =============================================================================

interface ProfilingMatrixProps {
  tableNames: string[];
  tableStages: Map<string, TablePipelineState>;
  currentTable: string | null;
  onCellClick?: (tableName: string, stage: PipelineStageKey) => void;
  selectedCell?: { tableName: string; stage: PipelineStageKey } | null;
}

interface HoveredCell {
  tableName: string;
  stage: PipelineStageKey;
  rect: { top: number; left: number; width: number; height: number };
  entry: TablePipelineState | undefined;
}

// =============================================================================
// Status → Color Mapping
// =============================================================================

const STAGE_STATUS_COLORS: Record<TableStageStatus, string> = {
  queued: INGESTION_STATUS_COLORS.queued,
  active: INGESTION_STATUS_COLORS.profiling,
  completed: INGESTION_STATUS_COLORS.completed,
  failed: INGESTION_STATUS_COLORS.failed,
};

// =============================================================================
// MatrixCell (memoized)
// =============================================================================

/** Inner dot color for enrichment source provenance. */
const ENRICHMENT_DOT_COLOR: Record<EnrichmentSource, string> = {
  fresh: "var(--dot-blue)",
  cached: "var(--dot-emerald)",
  fallback: "var(--dot-amber)",
};

const MatrixCell = memo(
  function MatrixCell({
    stageStatus,
    isSelected,
    tableName,
    stage,
    enrichmentSource,
    onClick,
    onMouseEnter,
    onMouseLeave,
  }: {
    stageStatus: TableStageStatus;
    isSelected: boolean;
    tableName: string;
    stage: PipelineStageKey;
    enrichmentSource?: EnrichmentSource;
    onClick?: () => void;
    onMouseEnter?: (e: React.MouseEvent<HTMLButtonElement>) => void;
    onMouseLeave?: () => void;
  }) {
    const color = STAGE_STATUS_COLORS[stageStatus];
    const isActive = stageStatus === "active";
    // Show inner dot on the enriching cell when completed with known source
    const showDot =
      stage === "enriching" && stageStatus === "completed" && enrichmentSource != null;

    return (
      <button
        type="button"
        aria-label={`${tableName}, ${stage}: ${stageStatus}`}
        onClick={onClick}
        onMouseEnter={onMouseEnter}
        onMouseLeave={onMouseLeave}
        className={cn(
          "w-5 h-5 rounded-sm transition-shadow relative",
          onClick && "hover:shadow-md hover:ring-2 hover:ring-ring cursor-pointer",
          !onClick && "cursor-default",
          isSelected && "ring-2 ring-primary ring-offset-1 animate-cell-select",
          isActive && "matrix-cell-shimmer"
        )}
        style={{
          backgroundColor: color,
          ...(isActive
            ? {
                backgroundImage: `linear-gradient(90deg, ${color} 25%, ${color}88 50%, ${color} 75%)`,
                backgroundSize: "200% 100%",
              }
            : {}),
        }}
      >
        {showDot && (
          <span className="absolute inset-0 flex items-center justify-center" aria-hidden="true">
            <span
              className="w-1.5 h-1.5 rounded-full"
              style={{ backgroundColor: ENRICHMENT_DOT_COLOR[enrichmentSource] }}
            />
          </span>
        )}
      </button>
    );
  },
  (prev, next) =>
    prev.stageStatus === next.stageStatus &&
    prev.isSelected === next.isSelected &&
    prev.enrichmentSource === next.enrichmentSource
);

// =============================================================================
// MatrixTooltip
// =============================================================================

function MatrixTooltip({
  hovered,
  containerRect,
  s,
}: {
  hovered: HoveredCell;
  containerRect: DOMRect;
  s: (text: string) => string;
}) {
  const { tableName, stage, entry } = hovered;
  const cellTop = hovered.rect.top - containerRect.top;
  const cellLeft = hovered.rect.left - containerRect.left;
  const stageLabel = PIPELINE_STAGES.find((s) => s.key === stage)?.label ?? stage;
  const stageStatus = entry?.stages[stage] ?? "queued";

  return (
    <div
      className="absolute z-30 pointer-events-none"
      style={{
        top: cellTop + hovered.rect.height + 6,
        left: Math.max(4, cellLeft - 80),
      }}
    >
      <div className="bg-popover text-popover-foreground border rounded-md shadow-lg p-2.5 text-xs w-52">
        {/* Arrow */}
        <div
          className="absolute -top-1.5 border-l border-t border-border bg-popover rotate-45"
          style={{
            width: 10,
            height: 10,
            left: Math.min(80, cellLeft) + hovered.rect.width / 2 - 5,
          }}
        />

        <div className="space-y-1.5 relative">
          {/* Status */}
          <div className="flex items-center gap-1.5">
            <span
              className="w-2 h-2 rounded-full shrink-0"
              style={{ backgroundColor: STAGE_STATUS_COLORS[stageStatus] }}
            />
            <span className="font-medium capitalize">{stageStatus}</span>
            <span className="text-muted-foreground ml-auto">{stageLabel}</span>
          </div>

          {/* Table info */}
          <div className="font-mono text-muted-foreground truncate">{s(tableName)}</div>

          {entry && entry.durationMs != null && (
            <div className="flex items-center gap-2 text-muted-foreground">
              <span>{entry.columns ?? 0} cols</span>
              <span>{(entry.durationMs / 1000).toFixed(1)}s</span>
            </div>
          )}

          {/* Enrichment source in tooltip for enriching stage */}
          {stage === "enriching" && entry?.enrichmentSource && (
            <div className="flex items-center gap-1.5 text-muted-foreground">
              <span
                className="w-2 h-2 rounded-full shrink-0"
                style={{ backgroundColor: ENRICHMENT_DOT_COLOR[entry.enrichmentSource] }}
              />
              <span className="capitalize">{entry.enrichmentSource}</span>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

// =============================================================================
// ProfilingMatrix
// =============================================================================

export function ProfilingMatrix({
  tableNames,
  tableStages,
  currentTable,
  onCellClick,
  selectedCell,
}: ProfilingMatrixProps) {
  const { s } = useScramble();
  const scrollRef = useRef<HTMLDivElement>(null);
  const leaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [hoveredCell, setHoveredCell] = useState<HoveredCell | null>(null);

  const handleCellMouseEnter = useCallback(
    (
      e: React.MouseEvent<HTMLButtonElement>,
      tableName: string,
      stage: PipelineStageKey,
      entry: TablePipelineState | undefined
    ) => {
      if (leaveTimerRef.current) {
        clearTimeout(leaveTimerRef.current);
        leaveTimerRef.current = null;
      }
      const rect = e.currentTarget.getBoundingClientRect();
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
      <div className="text-sm text-muted-foreground text-center py-4">No tables profiled yet</div>
    );
  }

  const containerRect = scrollRef.current?.getBoundingClientRect();

  return (
    <div ref={scrollRef} className="max-h-[480px] overflow-auto relative">
      {/* Header row (sticky) */}
      <div className="sticky top-0 z-10 bg-background flex items-center gap-1 pb-1">
        <div className="sticky left-0 z-20 bg-background w-[140px] shrink-0 text-xs font-medium text-muted-foreground truncate">
          Table
        </div>
        {PIPELINE_STAGES.map((stage) => (
          <div
            key={stage.key}
            className="w-5 text-xs text-muted-foreground text-center shrink-0"
            title={stage.label}
          >
            {stage.label.slice(0, 3)}
          </div>
        ))}
      </div>

      {/* Rows */}
      <div className="space-y-0.5">
        {tableNames.map((tableName) => {
          const entry = tableStages.get(tableName);
          const isCurrentRow = currentTable === tableName;

          return (
            <div key={tableName} className="flex items-center gap-1">
              {/* Table name (sticky left) */}
              <div
                className={cn(
                  "sticky left-0 z-10 bg-background w-[140px] shrink-0 text-xs truncate flex items-center gap-1",
                  isCurrentRow && "font-medium text-foreground"
                )}
                title={s(tableName)}
              >
                {isCurrentRow && (
                  <span
                    className="inline-block w-2 h-2 rounded-full animate-pulse shrink-0"
                    style={{ backgroundColor: INGESTION_STATUS_COLORS.profiling }}
                  />
                )}
                {s(tableName)}
              </div>

              {/* Stage cells */}
              {PIPELINE_STAGES.map((stage) => {
                const stageStatus: TableStageStatus = entry?.stages[stage.key] ?? "queued";
                const isCellSelected =
                  selectedCell?.tableName === tableName && selectedCell?.stage === stage.key;

                return (
                  <MatrixCell
                    key={stage.key}
                    stageStatus={stageStatus}
                    isSelected={isCellSelected}
                    tableName={tableName}
                    stage={stage.key}
                    enrichmentSource={entry?.enrichmentSource}
                    onClick={onCellClick ? () => onCellClick(tableName, stage.key) : undefined}
                    onMouseEnter={(e) => handleCellMouseEnter(e, tableName, stage.key, entry)}
                    onMouseLeave={handleCellMouseLeave}
                  />
                );
              })}
            </div>
          );
        })}
      </div>

      {/* Tooltip */}
      {hoveredCell && containerRect && (
        <MatrixTooltip hovered={hoveredCell} containerRect={containerRect} s={s} />
      )}
    </div>
  );
}

// =============================================================================
// ProfilingMatrixSkeleton
// =============================================================================

export function ProfilingMatrixSkeleton({ rows }: { rows: number }) {
  return (
    <div className="space-y-1">
      {/* Header */}
      <div className="flex items-center gap-1">
        <div className="w-[140px] h-4 bg-muted rounded animate-shimmer" />
        {PIPELINE_STAGES.map((stage) => (
          <div key={stage.key} className="w-5 h-4 bg-muted rounded animate-shimmer" />
        ))}
      </div>
      {/* Rows */}
      {Array.from({ length: rows }, (_, rowIdx) => (
        <div
          // biome-ignore lint/suspicious/noArrayIndexKey: static skeleton
          key={rowIdx}
          className="flex items-center gap-1"
          style={{ animationDelay: `${rowIdx * 75}ms` }}
        >
          <div
            className="w-24 h-4 bg-muted rounded animate-shimmer"
            style={{ animationDelay: `${rowIdx * 75}ms` }}
          />
          {PIPELINE_STAGES.map((stage) => (
            <div
              key={stage.key}
              className="w-5 h-5 rounded-sm bg-muted animate-shimmer"
              style={{ animationDelay: `${rowIdx * 75}ms` }}
            />
          ))}
        </div>
      ))}
    </div>
  );
}
