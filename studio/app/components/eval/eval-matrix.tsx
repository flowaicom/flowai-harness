/**
 * Eval matrix grid (partition matrix).
 *
 * Rows = test cases, columns = sample attempts.
 * Each cell: 20x20 rounded square colored by pass/fail.
 * Virtualized rows via @tanstack/react-virtual for 200+ test cases.
 * Rich tooltip popover, selected cell highlight, running shimmer.
 *
 * @module components/eval/eval-matrix
 */

import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useCallback, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { SampleResult, ScorerResult, TestCaseResult, TestCaseState } from "~/lib/domain/eval";
import { EVAL_STATUS_COLORS, extractSampleScore, parseScorerDetails } from "~/lib/domain/eval";
import { useScramble } from "~/lib/scramble";
import { cn, formatNumber } from "~/lib/utils";

// =============================================================================
// Types
// =============================================================================

type SortKey = "name" | "score-asc" | "score-desc" | "duration";

interface EvalMatrixProps {
  results: readonly TestCaseResult[];
  samplesPerCase: number;
  passThreshold?: number;
  testCaseStates?: Map<string, TestCaseState>;
  onCellClick?: (testCaseId: string, sampleIndex: number) => void;
  selectedCell?: { testCaseId: string; sampleIndex: number } | null;
  onTestCaseClick?: (testCaseId: string) => void;
}

interface HoveredCell {
  row: number;
  col: number;
  rect: { top: number; left: number; width: number; height: number };
  sample: SampleResult | undefined;
  testCaseId: string;
}

// =============================================================================
// Score-Intensity Colors
// =============================================================================

/**
 * Returns a background color with intensity proportional to the sample score.
 * Passing: green scale (lighter = barely passing, darker = perfect score).
 * Failing: red scale (lighter = close to threshold, darker = zero score).
 */
function scoreIntensityColor(sample: SampleResult | undefined, passThreshold: number): string {
  if (!sample) return EVAL_STATUS_COLORS.queued;

  const score = extractSampleScore(sample);

  if (sample.passed) {
    // Green range: lerp from light green (score = threshold) to solid green (score = 1.0)
    const t = passThreshold < 1 ? Math.min(1, (score - passThreshold) / (1 - passThreshold)) : 1;
    // HSL green: hue=145, saturation=60-75%, lightness=55-35%
    const lightness = 55 - t * 20;
    const saturation = 60 + t * 15;
    return `hsl(145, ${saturation}%, ${lightness}%)`;
  }
  // Red range: lerp from light red (score close to threshold) to solid red (score = 0)
  const t = passThreshold > 0 ? Math.min(1, 1 - score / passThreshold) : 1;
  const lightness = 55 - t * 15;
  const saturation = 60 + t * 15;
  return `hsl(0, ${saturation}%, ${lightness}%)`;
}

// =============================================================================
// Score Helpers
// =============================================================================

function formatSingleScoreBreakdown(scorer: ScorerResult): string {
  const parsed = parseScorerDetails(scorer);
  switch (parsed.kind) {
    case "trajectory":
      return `trajectory: ${scorer.score.toFixed(3)}`;
    case "finalResponse":
      return `final response: ${scorer.score.toFixed(3)}`;
    default:
      return `${scorer.scorerName}: ${scorer.score.toFixed(3)}`;
  }
}

function formatScoreBreakdown(scores: readonly ScorerResult[]): string {
  if (scores.length === 0) return "N/A";
  return scores.map(formatSingleScoreBreakdown).join(" · ");
}

// =============================================================================
// MatrixCell (memoized)
// =============================================================================

const MatrixCell = memo(
  function MatrixCell({
    sample,
    isRunningCell,
    isSelected,
    testCaseId,
    sampleIndex,
    passThreshold,
    onClick,
    onMouseEnter,
    onMouseLeave,
  }: {
    sample: SampleResult | undefined;
    isRunningCell: boolean;
    isSelected: boolean;
    testCaseId: string;
    sampleIndex: number;
    passThreshold: number;
    onClick?: () => void;
    onMouseEnter?: (e: React.MouseEvent<HTMLButtonElement>) => void;
    onMouseLeave?: () => void;
  }) {
    const color = scoreIntensityColor(sample, passThreshold);

    return (
      <button
        type="button"
        aria-label={`${testCaseId}, sample ${sampleIndex + 1}: ${sample ? (sample.passed ? "pass" : "fail") : "pending"}`}
        onClick={onClick}
        onMouseEnter={onMouseEnter}
        onMouseLeave={onMouseLeave}
        className={cn(
          "w-5 h-5 rounded-sm transition-shadow",
          onClick && "hover:shadow-md hover:ring-2 hover:ring-ring cursor-pointer",
          !onClick && "cursor-default",
          isSelected && "ring-2 ring-primary ring-offset-1 animate-cell-select",
          isRunningCell && !sample && "matrix-cell-shimmer"
        )}
        style={{
          backgroundColor: color,
          ...(isRunningCell && !sample
            ? {
                backgroundImage: `linear-gradient(90deg, ${color} 25%, ${color}88 50%, ${color} 75%)`,
                backgroundSize: "200% 100%",
              }
            : {}),
        }}
      />
    );
  },
  (prev, next) =>
    prev.sample?.passed === next.sample?.passed &&
    prev.sample?.sampleIndex === next.sample?.sampleIndex &&
    prev.isRunningCell === next.isRunningCell &&
    prev.isSelected === next.isSelected &&
    prev.passThreshold === next.passThreshold
);

// =============================================================================
// MatrixTooltip
// =============================================================================

function MatrixTooltip({ hovered }: { hovered: HoveredCell }) {
  const { s } = useScramble();
  const { sample, testCaseId } = hovered;
  if (typeof document === "undefined") return null;

  const tooltipWidth = 224;
  const viewportPadding = 8;
  const gap = 6;
  const estimatedHeight = sample ? 126 : 54;
  const preferredLeft = hovered.rect.left + hovered.rect.width / 2 - tooltipWidth / 2;
  const left = Math.min(
    Math.max(viewportPadding, preferredLeft),
    window.innerWidth - tooltipWidth - viewportPadding
  );
  const showAbove =
    hovered.rect.top > estimatedHeight + gap + viewportPadding &&
    hovered.rect.top + hovered.rect.height + gap + estimatedHeight > window.innerHeight;
  const top = showAbove
    ? Math.max(viewportPadding, hovered.rect.top - estimatedHeight - gap)
    : hovered.rect.top + hovered.rect.height + gap;
  const arrowLeft = Math.min(
    tooltipWidth - 18,
    Math.max(8, hovered.rect.left + hovered.rect.width / 2 - left - 5)
  );

  return createPortal(
    <div
      className="fixed z-[100] pointer-events-none"
      style={{
        top,
        left,
      }}
    >
      <div className="relative bg-popover text-popover-foreground border rounded-md shadow-lg p-2.5 text-xs w-56">
        {/* Arrow */}
        <div
          className={cn(
            "absolute border-border bg-popover rotate-45",
            showAbove ? "-bottom-1.5 border-r border-b" : "-top-1.5 border-l border-t"
          )}
          style={{
            width: 10,
            height: 10,
            left: arrowLeft,
          }}
        />

        <div className="space-y-1.5 relative">
          {/* Status */}
          <div className="flex items-center gap-1.5">
            <span
              className="w-2 h-2 rounded-full shrink-0"
              style={{
                backgroundColor: sample
                  ? sample.passed
                    ? EVAL_STATUS_COLORS.completed
                    : EVAL_STATUS_COLORS.failed
                  : EVAL_STATUS_COLORS.queued,
              }}
            />
            <span className="font-medium">
              {sample ? (sample.passed ? "Pass" : "Fail") : "Pending"}
            </span>
            <span className="text-muted-foreground ml-auto truncate max-w-[100px]">
              {s(testCaseId)}
            </span>
          </div>

          {sample && (
            <>
              {/* Score breakdown */}
              <div className="text-muted-foreground">{formatScoreBreakdown(sample.scores)}</div>

              {/* Duration + tokens */}
              <div className="flex items-center gap-2 text-muted-foreground">
                <span>{sample.durationMs}ms</span>
                <span>
                  {formatNumber(sample.tokenUsage.inputTokens + sample.tokenUsage.outputTokens)} tok
                </span>
              </div>

              {/* Error */}
              {sample.error && (
                <div className="font-mono text-destructive text-[10px] leading-tight break-words">
                  {sample.error.length > 120 ? `${sample.error.slice(0, 120)}...` : sample.error}
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </div>,
    document.body
  );
}

// =============================================================================
// EvalMatrix (Virtualized)
// =============================================================================

export function EvalMatrix({
  results,
  samplesPerCase,
  passThreshold = 0.7,
  testCaseStates,
  onCellClick,
  selectedCell,
  onTestCaseClick,
}: EvalMatrixProps) {
  const { s } = useScramble();
  const scrollRef = useRef<HTMLDivElement>(null);
  const leaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [hoveredCell, setHoveredCell] = useState<HoveredCell | null>(null);
  const [sortKey, setSortKey] = useState<SortKey>("name");

  // Sort results
  const sortedResults = useMemo(() => {
    const arr = [...results];
    switch (sortKey) {
      case "score-asc":
        return arr.sort((a, b) => a.aggregateScore - b.aggregateScore);
      case "score-desc":
        return arr.sort((a, b) => b.aggregateScore - a.aggregateScore);
      case "duration": {
        const avgDur = (r: TestCaseResult) => {
          const done = r.samples.filter((sm) => sm.durationMs > 0);
          return done.length > 0
            ? done.reduce((sum, sm) => sum + sm.durationMs, 0) / done.length
            : 0;
        };
        return arr.sort((a, b) => avgDur(b) - avgDur(a));
      }
      default:
        return arr; // original order
    }
  }, [results, sortKey]);

  // Per-test-case mean sample duration (seconds), null when no samples completed
  const avgDurationSec = useMemo(() => {
    return sortedResults.map((r) => {
      const done = r.samples.filter((sm) => sm.durationMs > 0);
      if (done.length === 0) return null;
      return done.reduce((sum, sm) => sum + sm.durationMs, 0) / done.length / 1000;
    });
  }, [sortedResults]);

  const rowVirtualizer = useVirtualizer({
    count: sortedResults.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 24, // 20px cell + 4px gap
    overscan: 5,
  });

  const handleCellMouseEnter = useCallback(
    (
      e: React.MouseEvent<HTMLButtonElement>,
      row: number,
      col: number,
      sample: SampleResult | undefined,
      testCaseId: string
    ) => {
      if (leaveTimerRef.current) {
        clearTimeout(leaveTimerRef.current);
        leaveTimerRef.current = null;
      }
      const rect = e.currentTarget.getBoundingClientRect();
      setHoveredCell({
        row,
        col,
        rect: { top: rect.top, left: rect.left, width: rect.width, height: rect.height },
        sample,
        testCaseId,
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

  if (results.length === 0) {
    return <div className="text-sm text-muted-foreground text-center py-4">No results yet</div>;
  }

  return (
    <div className="space-y-2">
      {/* Sort controls */}
      {results.length > 1 && (
        <div className="flex items-center gap-2">
          <span className="section-label">Sort:</span>
          {(
            [
              { key: "name", label: "Name" },
              { key: "score-desc", label: "Score" },
              { key: "duration", label: "Duration" },
            ] as const
          ).map(({ key, label }) => (
            <button
              key={key}
              type="button"
              onClick={() => setSortKey(sortKey === key ? "name" : key)}
              className={cn(
                "text-[10px] px-1.5 py-0.5 rounded transition-colors",
                sortKey === key
                  ? "bg-primary/10 text-primary font-medium"
                  : "text-muted-foreground hover:text-foreground"
              )}
            >
              {label}
              {sortKey === "score-asc" && key === "score-desc" && " (asc)"}
            </button>
          ))}
        </div>
      )}

      <div ref={scrollRef} className="max-h-[480px] overflow-auto scroll-container relative">
        {/* Header row (sticky) */}
        <div
          className="sticky top-0 z-10 bg-background flex items-center gap-1 pb-1"
          style={{ minWidth: `${120 + samplesPerCase * 24 + samplesPerCase * 4 + 48}px` }}
        >
          <div className="sticky left-0 z-20 bg-background w-[120px] shrink-0 text-xs font-medium text-muted-foreground truncate">
            Test Case
          </div>
          {Array.from({ length: samplesPerCase }, (_, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: static column headers
            <div key={i} className="w-5 text-xs text-muted-foreground text-center shrink-0">
              {i + 1}
            </div>
          ))}
          <div className="w-10 text-[10px] text-muted-foreground text-right shrink-0">Avg</div>
        </div>

        {/* Virtualized rows */}
        <div className="relative" style={{ height: rowVirtualizer.getTotalSize() }}>
          {rowVirtualizer.getVirtualItems().map((virtualRow) => {
            const result = sortedResults[virtualRow.index];
            const tcState = testCaseStates?.get(result.testCaseId);
            const isRunning = tcState?.state === "running";
            const scorePct = Math.round(result.aggregateScore * 100);
            const isPassing = result.aggregateScore >= passThreshold;

            return (
              <div
                key={result.testCaseId}
                className="absolute left-0 w-full flex items-center gap-1"
                style={{
                  top: virtualRow.start,
                  height: virtualRow.size,
                }}
              >
                {/* Test case name (sticky left, clickable) */}
                {/* biome-ignore lint/a11y/noStaticElementInteractions: role is conditionally set below */}
                <div
                  className={cn(
                    "sticky left-0 z-10 bg-background w-[120px] shrink-0 text-xs truncate flex items-center gap-1 group/row",
                    isRunning && "font-medium text-foreground",
                    onTestCaseClick && "cursor-pointer hover:text-primary hover:underline"
                  )}
                  title={result.testCaseId}
                  onClick={onTestCaseClick ? () => onTestCaseClick(result.testCaseId) : undefined}
                  role={onTestCaseClick ? "button" : undefined}
                  tabIndex={onTestCaseClick ? 0 : undefined}
                  onKeyDown={
                    onTestCaseClick
                      ? (e) => {
                          if (e.key === "Enter") onTestCaseClick(result.testCaseId);
                        }
                      : undefined
                  }
                >
                  {isRunning && (
                    <span
                      className="inline-block w-2 h-2 rounded-full animate-pulse shrink-0"
                      style={{ backgroundColor: EVAL_STATUS_COLORS.running }}
                    />
                  )}
                  <span className="truncate">{s(result.testCaseId)}</span>
                  {avgDurationSec[virtualRow.index] != null && (
                    <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground/50">
                      ({(avgDurationSec[virtualRow.index] ?? 0).toFixed(1)}s)
                    </span>
                  )}
                  {onTestCaseClick && (
                    <svg
                      className="w-3 h-3 shrink-0 opacity-0 group-hover/row:opacity-100 transition-opacity text-muted-foreground"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth={2}
                      aria-hidden="true"
                    >
                      <path d="M9 18l6-6-6-6" strokeLinecap="round" strokeLinejoin="round" />
                    </svg>
                  )}
                </div>

                {/* Sample cells */}
                {Array.from({ length: samplesPerCase }, (_, sampleIdx) => {
                  const sample = result.samples.find((sm) => sm.sampleIndex === sampleIdx);
                  const isCellSelected =
                    selectedCell?.testCaseId === result.testCaseId &&
                    selectedCell?.sampleIndex === sampleIdx;

                  return (
                    <MatrixCell
                      // biome-ignore lint/suspicious/noArrayIndexKey: fixed column count
                      key={sampleIdx}
                      sample={sample}
                      isRunningCell={isRunning}
                      isSelected={isCellSelected}
                      testCaseId={result.testCaseId}
                      sampleIndex={sampleIdx}
                      passThreshold={passThreshold}
                      onClick={
                        onCellClick ? () => onCellClick(result.testCaseId, sampleIdx) : undefined
                      }
                      onMouseEnter={(e) =>
                        handleCellMouseEnter(
                          e,
                          virtualRow.index,
                          sampleIdx,
                          sample,
                          result.testCaseId
                        )
                      }
                      onMouseLeave={handleCellMouseLeave}
                    />
                  );
                })}

                {/* Aggregate score column */}
                {result.samples.length > 0 && (
                  <span
                    className={cn(
                      "w-10 text-right text-[10px] font-mono tabular-nums shrink-0",
                      isPassing ? "text-[var(--dot-emerald)]" : "text-[var(--dot-red)]"
                    )}
                  >
                    {scorePct}%
                  </span>
                )}
              </div>
            );
          })}
        </div>

        {/* Tooltip */}
        {hoveredCell && <MatrixTooltip hovered={hoveredCell} />}
      </div>
    </div>
  );
}

// =============================================================================
// MatrixSkeleton
// =============================================================================

export function MatrixSkeleton({ rows, cols }: { rows: number; cols: number }) {
  return (
    <div className="space-y-1">
      {/* Header */}
      <div className="flex items-center gap-1">
        <div className="w-[120px] h-4 bg-muted rounded animate-pulse animate-shimmer" />
        {Array.from({ length: cols }, (_, i) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: static skeleton
          <div key={i} className="w-5 h-4 bg-muted rounded animate-pulse animate-shimmer" />
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
            className="w-20 h-4 bg-muted rounded animate-pulse animate-shimmer"
            style={{ animationDelay: `${rowIdx * 75}ms` }}
          />
          {Array.from({ length: cols }, (_, colIdx) => (
            <div
              // biome-ignore lint/suspicious/noArrayIndexKey: static skeleton
              key={colIdx}
              className="w-5 h-5 rounded-sm bg-muted animate-pulse animate-shimmer"
              style={{ animationDelay: `${rowIdx * 75}ms` }}
            />
          ))}
        </div>
      ))}
    </div>
  );
}
