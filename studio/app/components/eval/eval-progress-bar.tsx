/**
 * Segmented progress bar for eval runs.
 *
 * Shows completed/total with percentage, elapsed time, ETA,
 * and a segmented status bar (queued/running/completed/failed).
 * Smooth easing, running glow, staggered transitions, legend.
 *
 * @module components/eval/eval-progress-bar
 */

import type { EvalProgress } from "~/lib/domain/eval";
import { EVAL_STATUS_COLORS } from "~/lib/domain/eval";
import { cn } from "~/lib/utils";

interface EvalProgressBarProps {
  progress: EvalProgress;
  className?: string;
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const secs = Math.round(ms / 1000);
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  const remainingSecs = secs % 60;
  return `${mins}m ${remainingSecs}s`;
}

export function EvalProgressBar({ progress, className }: EvalProgressBarProps) {
  const pct =
    progress.totalSamples > 0
      ? Math.round((progress.completedSamples / progress.totalSamples) * 100)
      : 0;

  // Count test case states for segmented bar
  let queued = 0;
  let running = 0;
  let completed = 0;
  let failed = 0;
  for (const entry of progress.testCaseStates) {
    switch (entry.state.state) {
      case "queued":
        queued++;
        break;
      case "running":
        running++;
        break;
      case "completed":
        completed++;
        break;
      case "failed":
        failed++;
        break;
    }
  }
  const total = progress.totalTestCases || 1;

  return (
    <div className={cn("space-y-1.5", className)}>
      <div className="flex items-center justify-between text-xs">
        <span className="text-muted-foreground">
          {progress.completedTestCases}/{progress.totalTestCases} test cases
        </span>
        <div className="flex items-center gap-3">
          {progress.elapsedMs > 0 && (
            <span className="text-muted-foreground">{formatDuration(progress.elapsedMs)}</span>
          )}
          {progress.estimatedRemainingMs != null && progress.estimatedRemainingMs > 0 ? (
            <span className="text-muted-foreground">
              ~{formatDuration(progress.estimatedRemainingMs)} remaining
            </span>
          ) : progress.completedTestCases < progress.totalTestCases &&
            progress.completedTestCases === 0 ? (
            <span className="text-muted-foreground/50">Estimating...</span>
          ) : null}
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
        aria-label={`Eval progress: ${progress.completedTestCases} of ${progress.totalTestCases} test cases completed (${pct}%)`}
      >
        {completed > 0 && (
          <div
            className="h-full transition-all duration-700 ease-out"
            style={{
              width: `${(completed / total) * 100}%`,
              backgroundColor: EVAL_STATUS_COLORS.completed,
              transitionDelay: "0ms",
            }}
          />
        )}
        {running > 0 && (
          <div
            className="h-full transition-all duration-700 ease-out animate-pulse-soft"
            style={{
              width: `${(running / total) * 100}%`,
              backgroundColor: EVAL_STATUS_COLORS.running,
              boxShadow: `inset 0 0 8px ${EVAL_STATUS_COLORS.running}4D`,
              transitionDelay: "100ms",
            }}
          />
        )}
        {failed > 0 && (
          <div
            className="h-full transition-all duration-700 ease-out"
            style={{
              width: `${(failed / total) * 100}%`,
              backgroundColor: EVAL_STATUS_COLORS.failed,
              transitionDelay: "200ms",
            }}
          />
        )}
        {/* queued fills the remainder implicitly via bg-muted */}
      </div>

      {/* Legend + current test case */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3 text-[10px] text-muted-foreground">
          {completed > 0 && (
            <span className="flex items-center gap-1">
              <span
                className="status-dot"
                style={{ backgroundColor: EVAL_STATUS_COLORS.completed }}
              />
              Completed
            </span>
          )}
          {running > 0 && (
            <span className="flex items-center gap-1">
              <span
                className="status-dot"
                style={{ backgroundColor: EVAL_STATUS_COLORS.running }}
              />
              Running
            </span>
          )}
          {failed > 0 && (
            <span className="flex items-center gap-1">
              <span className="status-dot" style={{ backgroundColor: EVAL_STATUS_COLORS.failed }} />
              Failed
            </span>
          )}
          {queued > 0 && (
            <span className="flex items-center gap-1">
              <span className="status-dot" style={{ backgroundColor: EVAL_STATUS_COLORS.queued }} />
              Queued
            </span>
          )}
        </div>

        {progress.currentTestCaseId && (
          <div className="text-xs text-muted-foreground truncate max-w-[200px]">
            Running: {progress.currentTestCaseId}
          </div>
        )}
      </div>
    </div>
  );
}
