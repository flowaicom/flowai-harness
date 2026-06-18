/**
 * Segmented progress bar for profiling runs.
 *
 * Shows completed/total with percentage, elapsed time, ETA,
 * and a segmented status bar (queued/running/completed/failed).
 *
 * @module components/data/profiling-progress-bar
 */

import { useEffect, useState } from "react";
import { INGESTION_STATUS_COLORS } from "~/lib/domain/data";
import { useScramble } from "~/lib/scramble";
import { cn, formatDuration } from "~/lib/utils";

// =============================================================================
// Types
// =============================================================================

interface ProfilingProgressBarProps {
  totalTables: number;
  completedTables: number;
  runningTables: number;
  failedTables: number;
  currentTable: string | null;
  elapsedMs: number;
  className?: string;
}

// =============================================================================
// useElapsedTimer
// =============================================================================

export function useElapsedTimer(startedAt: number | null): number {
  const [elapsed, setElapsed] = useState(() => (startedAt ? Date.now() - startedAt : 0));

  useEffect(() => {
    if (!startedAt) {
      setElapsed(0);
      return;
    }

    setElapsed(Date.now() - startedAt);
    const interval = setInterval(() => {
      setElapsed(Date.now() - startedAt);
    }, 1000);

    return () => clearInterval(interval);
  }, [startedAt]);

  return elapsed;
}

// =============================================================================
// ProfilingProgressBar
// =============================================================================

export function ProfilingProgressBar({
  totalTables,
  completedTables,
  runningTables,
  failedTables,
  currentTable,
  elapsedMs,
  className,
}: ProfilingProgressBarProps) {
  const { s } = useScramble();
  const total = totalTables || 1;
  const pct = totalTables > 0 ? Math.round((completedTables / totalTables) * 100) : 0;

  // ETA: linear extrapolation
  const eta =
    completedTables > 0 && completedTables < totalTables
      ? Math.round((elapsedMs / completedTables) * (totalTables - completedTables))
      : null;

  return (
    <div className={cn("space-y-1.5", className)}>
      <div className="flex items-center justify-between text-xs">
        <span className="text-muted-foreground">
          {completedTables}/{totalTables} tables
        </span>
        <div className="flex items-center gap-3">
          {elapsedMs > 0 && (
            <span className="text-muted-foreground">{formatDuration(elapsedMs)}</span>
          )}
          {eta != null && eta > 0 && (
            <span className="text-muted-foreground">~{formatDuration(eta)} remaining</span>
          )}
          <span className="font-mono font-medium">{pct}%</span>
        </div>
      </div>

      {/* Segmented status bar */}
      <div
        className="h-2 bg-muted rounded-full overflow-hidden flex"
        role="progressbar"
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={`Profiling progress: ${completedTables} of ${totalTables} tables completed (${pct}%)`}
      >
        {completedTables > 0 && (
          <div
            className="h-full transition-all duration-700 ease-out"
            style={{
              width: `${(completedTables / total) * 100}%`,
              backgroundColor: INGESTION_STATUS_COLORS.completed,
              transitionDelay: "0ms",
            }}
          />
        )}
        {runningTables > 0 && (
          <div
            className="h-full transition-all duration-700 ease-out animate-pulse-soft"
            style={{
              width: `${(runningTables / total) * 100}%`,
              backgroundColor: INGESTION_STATUS_COLORS.profiling,
              boxShadow: `inset 0 0 8px ${INGESTION_STATUS_COLORS.profiling}4D`,
              transitionDelay: "100ms",
            }}
          />
        )}
        {failedTables > 0 && (
          <div
            className="h-full transition-all duration-700 ease-out"
            style={{
              width: `${(failedTables / total) * 100}%`,
              backgroundColor: INGESTION_STATUS_COLORS.failed,
              transitionDelay: "200ms",
            }}
          />
        )}
        {/* queued fills the remainder implicitly via bg-muted */}
      </div>

      {/* Legend + current table */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3 text-[10px] text-muted-foreground">
          {completedTables > 0 && (
            <span className="flex items-center gap-1">
              <span
                className="w-2 h-2 rounded-full"
                style={{ backgroundColor: INGESTION_STATUS_COLORS.completed }}
              />
              Completed
            </span>
          )}
          {runningTables > 0 && (
            <span className="flex items-center gap-1">
              <span
                className="w-2 h-2 rounded-full"
                style={{ backgroundColor: INGESTION_STATUS_COLORS.profiling }}
              />
              Running
            </span>
          )}
          {failedTables > 0 && (
            <span className="flex items-center gap-1">
              <span
                className="w-2 h-2 rounded-full"
                style={{ backgroundColor: INGESTION_STATUS_COLORS.failed }}
              />
              Failed
            </span>
          )}
          {totalTables - completedTables - runningTables - failedTables > 0 && (
            <span className="flex items-center gap-1">
              <span
                className="w-2 h-2 rounded-full"
                style={{ backgroundColor: INGESTION_STATUS_COLORS.queued }}
              />
              Queued
            </span>
          )}
        </div>

        {currentTable && (
          <div className="text-xs text-muted-foreground truncate max-w-[200px]">
            Running: {s(currentTable)}
          </div>
        )}
      </div>
    </div>
  );
}
