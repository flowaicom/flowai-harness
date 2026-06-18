import { type ReactNode, useEffect, useMemo, useState } from "react";
import { Link } from "react-router";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

type ResultFilter = "all" | "passed" | "failed";

export interface SharedEvalDetailResultLike {
  readonly testCaseId: string;
  readonly aggregateScore: number;
  readonly samples: readonly unknown[];
}

export interface SharedEvalDetailSummaryLike {
  readonly passed: number;
  readonly failed: number;
  readonly aggregateScore: number;
  readonly totalDurationMs: number;
}

export interface SharedEvalDetailRunMetaLike {
  readonly id: string;
  readonly parentRunId?: string | null;
  readonly config: {
    readonly mode: string;
    readonly model?: string | null;
    readonly provider?: string | null;
    readonly samplesPerCase: number;
    readonly concurrency: number;
    readonly passThreshold: number;
    readonly retryPolicy?: {
      readonly maxRetries: number;
    } | null;
  };
  readonly status: {
    readonly status: string;
  };
}

export interface SharedEvalDetailPageProps<
  TResult extends SharedEvalDetailResultLike = SharedEvalDetailResultLike,
  TSummary extends SharedEvalDetailSummaryLike = SharedEvalDetailSummaryLike,
  TRunMeta extends SharedEvalDetailRunMetaLike = SharedEvalDetailRunMetaLike,
> {
  readonly loaded: boolean;
  readonly loadingError?: string | null;
  readonly containerClassName?: string;
  readonly runMeta: TRunMeta | null;
  readonly results: readonly TResult[];
  readonly summary?: TSummary | null;
  readonly isActiveRun: boolean;
  readonly isPaused: boolean;
  readonly activatingCount?: number | null;
  readonly exporting?: boolean;
  readonly error?: string | null;
  readonly skippedTestCases?: ReadonlyMap<string, string>;
  readonly formatText?: (value: string) => string;
  readonly copyIdControl?: ReactNode;
  readonly reconnectingBanner?: ReactNode;
  readonly progressBar?: ReactNode;
  readonly dataReadinessCard?: ReactNode;
  readonly runActivityPanel?: ReactNode;
  readonly enableResultFilter?: boolean;
  readonly renderStatusTag?: (status: string) => ReactNode;
  readonly renderErrorBanner?: (message: string, onDismiss: () => void) => ReactNode;
  readonly renderResultsMatrix: (args: {
    readonly results: readonly TResult[];
    readonly runMeta: TRunMeta;
  }) => ReactNode;
  readonly renderMatrixSkeleton?: (args: { readonly samplesPerCase: number }) => ReactNode;
  readonly renderSummaryCards?: (summary: TSummary) => ReactNode;
  readonly onDismissError?: () => void;
  readonly onNavigateParentRun?: (parentRunId: string) => void;
  readonly onPauseResume?: () => void;
  readonly onOpenExpand?: () => void;
  readonly onOpenRerun?: () => void;
  readonly onExportFailures?: () => void;
  readonly onOpenCompare?: () => void;
  readonly onActivatePassing?: () => void;
  readonly onCancelRun?: () => void;
  readonly cancelling?: boolean;
}

export interface SharedEvalCompareRunLike {
  readonly id: string;
  readonly config: {
    readonly mode: string;
  };
  readonly status: {
    readonly status: string;
  };
}

export interface SharedEvalCompareCaseLike {
  readonly testCaseId: string;
  readonly leftScore: number | null;
  readonly rightScore: number | null;
  readonly scoreDelta: number;
  readonly regression: boolean;
  readonly improvement: boolean;
}

export interface SharedEvalRunComparisonLike {
  readonly scoreDelta: number;
  readonly passRateDelta: number;
  readonly testCaseComparisons: readonly SharedEvalCompareCaseLike[];
}

export interface SharedEvalCompareOverlayProps<
  TComparison extends SharedEvalRunComparisonLike = SharedEvalRunComparisonLike,
  TRun extends SharedEvalCompareRunLike = SharedEvalCompareRunLike,
> {
  readonly currentRunId: string;
  readonly runs: readonly TRun[];
  readonly comparison: TComparison | null;
  readonly loading: boolean;
  readonly onSelectRun: (otherId: string) => void;
  readonly onClose: () => void;
  readonly buildTestCaseHref?: (testCaseId: string) => string;
}

function renderLoadingState(error?: string | null) {
  return (
    <div className="max-w-4xl mx-auto p-8 space-y-6">
      {error ? (
        <div className="rounded-md accent-bar-red bg-[var(--accent-red)] px-4 py-3 text-[var(--dot-red)] text-sm">
          {error}
        </div>
      ) : (
        <>
          <div className="flex justify-between">
            <div className="space-y-2">
              <div className="h-6 w-32 bg-muted rounded animate-shimmer" />
              <div className="h-4 w-48 bg-muted rounded animate-shimmer" />
            </div>
            <div className="h-6 w-20 bg-muted rounded animate-shimmer" />
          </div>
          <div className="grid grid-cols-4 gap-4">
            {Array.from({ length: 4 }, (_, index) => (
              <div
                key={`eval-detail-skeleton-chip-${index}`}
                className="h-10 bg-muted rounded animate-shimmer"
              />
            ))}
          </div>
          <div className="rounded-lg keyline-card p-4">
            <div className="h-4 w-28 bg-muted rounded animate-shimmer mb-3" />
            <div className="space-y-2">
              {Array.from({ length: 8 }, (_, row) => (
                <div key={`eval-detail-skeleton-row-${row}`} className="grid grid-cols-5 gap-2">
                  {Array.from({ length: 5 }, (_, col) => (
                    <div
                      key={`eval-detail-skeleton-cell-${row}-${col}`}
                      className="h-8 rounded bg-muted animate-shimmer"
                    />
                  ))}
                </div>
              ))}
            </div>
          </div>
        </>
      )}
    </div>
  );
}

function ConfigChip({
  label,
  value,
  title,
}: {
  readonly label: string;
  readonly value: string;
  readonly title?: string;
}) {
  return (
    <span
      className="inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded-md bg-muted text-muted-foreground"
      title={title}
    >
      <span className="text-muted-foreground/60">{label}:</span>
      <span className="font-medium text-foreground">{value}</span>
    </span>
  );
}

function SkippedBanner({
  skippedTestCases,
  formatText,
}: {
  readonly skippedTestCases: ReadonlyMap<string, string>;
  readonly formatText: (value: string) => string;
}) {
  const [expanded, setExpanded] = useState(false);
  const entries = Array.from(skippedTestCases.entries());

  return (
    <div className="rounded-md accent-bar-amber bg-[var(--accent-amber)] px-3 py-1.5 text-xs text-[var(--dot-amber)]">
      <button
        type="button"
        onClick={() => setExpanded((value) => !value)}
        aria-expanded={expanded}
        aria-label={`${entries.length} test case${entries.length > 1 ? "s" : ""} skipped — click to ${expanded ? "collapse" : "expand"}`}
        className="flex items-center gap-1.5 w-full text-left font-medium"
      >
        <span>
          {entries.length} test case{entries.length > 1 ? "s" : ""} skipped
        </span>
        <svg
          className={cx("w-3 h-3 transition-transform", expanded && "rotate-180")}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth={2}
          aria-hidden="true"
        >
          <path d="M6 9l6 6 6-6" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </button>
      {expanded ? (
        <div className="mt-1 space-y-0.5">
          {entries.map(([testCaseId, reason]) => (
            <div key={testCaseId} className="text-[10px] text-[var(--dot-amber)]/80">
              <span className="font-mono">{formatText(testCaseId).slice(0, 8)}</span>: {reason}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function StatCard({
  value,
  label,
  color,
}: {
  readonly value: string;
  readonly label: string;
  readonly color?: string;
}) {
  return (
    <div className="rounded-lg keyline-card p-3 text-center">
      <div className={cx("text-xl font-bold tabular-nums", color)}>{value}</div>
      <div className="text-[10px] text-muted-foreground">{label}</div>
    </div>
  );
}

function LiveStatsBar<
  TResult extends SharedEvalDetailResultLike,
  TSummary extends SharedEvalDetailSummaryLike,
>({
  results,
  passThreshold,
  summary,
}: {
  readonly results: readonly TResult[];
  readonly passThreshold: number;
  readonly summary: TSummary | null;
}) {
  if (results.length === 0 && !summary) {
    return null;
  }

  const passed =
    summary?.passed ?? results.filter((result) => result.aggregateScore >= passThreshold).length;
  const failed =
    summary?.failed ??
    results.filter((result) => result.aggregateScore < passThreshold && result.samples.length > 0)
      .length;
  const score =
    summary?.aggregateScore ??
    (results.length > 0
      ? results.reduce((total, result) => total + result.aggregateScore, 0) / results.length
      : 0);
  const duration = summary?.totalDurationMs ?? null;

  return (
    <div className="grid grid-cols-4 gap-3">
      <StatCard value={String(passed)} label="Passed" color="text-[var(--dot-emerald)]" />
      <StatCard
        value={String(failed)}
        label="Failed"
        color={failed > 0 ? "text-[var(--dot-red)]" : undefined}
      />
      <StatCard value={`${Math.round(score * 100)}%`} label="Score" />
      <StatCard
        value={duration != null ? `${(duration / 1000).toFixed(1)}s` : "--"}
        label="Duration"
      />
    </div>
  );
}

export function SharedEvalDetailPage<
  TResult extends SharedEvalDetailResultLike = SharedEvalDetailResultLike,
  TSummary extends SharedEvalDetailSummaryLike = SharedEvalDetailSummaryLike,
  TRunMeta extends SharedEvalDetailRunMetaLike = SharedEvalDetailRunMetaLike,
>({
  loaded,
  loadingError,
  containerClassName,
  runMeta,
  results,
  summary = null,
  isActiveRun,
  isPaused,
  activatingCount = null,
  exporting = false,
  error,
  skippedTestCases,
  formatText = (value) => value,
  copyIdControl,
  reconnectingBanner,
  progressBar,
  dataReadinessCard,
  runActivityPanel,
  enableResultFilter = false,
  renderStatusTag,
  renderErrorBanner,
  renderResultsMatrix,
  renderMatrixSkeleton,
  renderSummaryCards,
  onDismissError,
  onNavigateParentRun,
  onPauseResume,
  onOpenExpand,
  onOpenRerun,
  onExportFailures,
  onOpenCompare,
  onActivatePassing,
  onCancelRun,
  cancelling = false,
}: SharedEvalDetailPageProps<TResult, TSummary, TRunMeta>) {
  const [resultFilter, setResultFilter] = useState<ResultFilter>("all");

  const filteredResults = useMemo(() => {
    if (!enableResultFilter || isActiveRun || resultFilter === "all" || !runMeta) {
      return results;
    }
    if (resultFilter === "passed") {
      return results.filter((result) => result.aggregateScore >= runMeta.config.passThreshold);
    }
    return results.filter(
      (result) => result.aggregateScore < runMeta.config.passThreshold && result.samples.length > 0
    );
  }, [enableResultFilter, isActiveRun, resultFilter, results, runMeta]);

  if (!loaded || !runMeta) {
    return (
      <div className={cx("flex-1 overflow-y-auto", containerClassName)}>
        {renderLoadingState(loadingError)}
      </div>
    );
  }

  const hasFailedCases = results.some(
    (result) => result.aggregateScore < runMeta.config.passThreshold
  );
  const canRerun =
    (runMeta.status.status === "completed" || runMeta.status.status === "failed") && hasFailedCases;
  const passingCount = results.filter(
    (result) => result.aggregateScore >= runMeta.config.passThreshold
  ).length;
  const canActivatePassing =
    (runMeta.status.status === "completed" || runMeta.status.status === "failed") &&
    passingCount > 0;

  return (
    <div className={cx("flex-1 overflow-y-auto", containerClassName)}>
      <div className="max-w-4xl mx-auto p-8 space-y-6">
        <div className="space-y-3">
          <div className="flex items-start justify-between">
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-lg font-semibold">Eval Run</h1>
                {renderStatusTag?.(runMeta.status.status)}
                {runMeta.parentRunId ? (
                  <button
                    type="button"
                    onClick={() => onNavigateParentRun?.(runMeta.parentRunId ?? "")}
                    className="text-xs text-muted-foreground hover:text-primary hover:underline"
                    title={`Parent run: ${runMeta.parentRunId}`}
                  >
                    re-run
                  </button>
                ) : null}
              </div>
              <p className="flex items-center gap-1 text-xs text-muted-foreground font-mono mt-0.5">
                {formatText(runMeta.id)}
                {copyIdControl}
              </p>
            </div>
            <div className="flex items-center gap-1.5">
              {onCancelRun ? (
                <button
                  type="button"
                  onClick={onCancelRun}
                  disabled={cancelling}
                  aria-label="Cancel this eval run"
                  className="text-xs px-2.5 py-1 rounded-md border border-[var(--dot-red)]/30 text-[var(--dot-red)] hover:bg-[var(--accent-red)] transition-colors disabled:opacity-50 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                >
                  {cancelling ? "Cancelling..." : "Cancel"}
                </button>
              ) : null}
              {isActiveRun && onPauseResume ? (
                <button
                  type="button"
                  onClick={onPauseResume}
                  className="text-xs px-2.5 py-1 rounded-md border hover:bg-muted transition-colors"
                >
                  {isPaused ? "Resume" : "Pause"}
                </button>
              ) : null}
              {isActiveRun && onOpenExpand ? (
                <button
                  type="button"
                  onClick={onOpenExpand}
                  className="text-xs px-2.5 py-1 rounded-md border hover:bg-muted transition-colors"
                >
                  Add Tests
                </button>
              ) : null}
              {canRerun && onOpenRerun ? (
                <button
                  type="button"
                  onClick={onOpenRerun}
                  className="text-xs px-2.5 py-1 rounded-md border hover:bg-muted transition-colors"
                >
                  Re-run Failed
                </button>
              ) : null}
              {canRerun && onExportFailures ? (
                <button
                  type="button"
                  onClick={onExportFailures}
                  disabled={exporting}
                  className="text-xs px-2.5 py-1 rounded-md border hover:bg-muted transition-colors disabled:opacity-50"
                >
                  {exporting ? "Exporting..." : "Export"}
                </button>
              ) : null}
              {canRerun && onOpenCompare ? (
                <button
                  type="button"
                  onClick={onOpenCompare}
                  className="text-xs px-2.5 py-1 rounded-md border hover:bg-muted transition-colors"
                >
                  Compare
                </button>
              ) : null}
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-1.5">
            <ConfigChip label="Mode" value={runMeta.config.mode} />
            <ConfigChip
              label="Model"
              value={runMeta.config.model ?? "default"}
              title={runMeta.config.model ?? "system default"}
            />
            {runMeta.config.provider ? (
              <ConfigChip label="Provider" value={runMeta.config.provider} />
            ) : null}
            <ConfigChip label="Samples" value={String(runMeta.config.samplesPerCase)} />
            <ConfigChip label="Concurrency" value={String(runMeta.config.concurrency)} />
            <ConfigChip
              label="Threshold"
              value={`${Math.round(runMeta.config.passThreshold * 100)}%`}
            />
            {runMeta.config.retryPolicy ? (
              <ConfigChip label="Retries" value={String(runMeta.config.retryPolicy.maxRetries)} />
            ) : null}
          </div>
          {dataReadinessCard ? <div className="mt-3">{dataReadinessCard}</div> : null}
        </div>

        {reconnectingBanner}
        {error
          ? (renderErrorBanner?.(error, onDismissError ?? (() => {})) ?? (
              <div className="rounded-md accent-bar-red bg-[var(--accent-red)] px-4 py-3 text-[var(--dot-red)] text-sm">
                {error}
              </div>
            ))
          : null}
        {skippedTestCases && skippedTestCases.size > 0 ? (
          <SkippedBanner skippedTestCases={skippedTestCases} formatText={formatText} />
        ) : null}

        {(isActiveRun || runMeta.status.status === "paused") && progressBar ? progressBar : null}

        {runActivityPanel}

        {results.length > 0 ? (
          <div className="rounded-lg keyline-card p-4">
            <div className="flex items-center justify-between mb-3">
              <h3 className="section-label">Results Matrix</h3>
              {enableResultFilter && !isActiveRun ? (
                <div className="flex gap-0.5" role="tablist">
                  {(["all", "passed", "failed"] as const).map((filter) => {
                    const count =
                      filter === "all"
                        ? results.length
                        : filter === "passed"
                          ? results.filter(
                              (result) => result.aggregateScore >= runMeta.config.passThreshold
                            ).length
                          : results.filter(
                              (result) =>
                                result.aggregateScore < runMeta.config.passThreshold &&
                                result.samples.length > 0
                            ).length;

                    return (
                      <button
                        key={filter}
                        type="button"
                        role="tab"
                        aria-selected={resultFilter === filter}
                        onClick={() => setResultFilter(filter)}
                        className={cx(
                          "px-2 py-0.5 rounded text-[10px] font-medium transition-colors",
                          resultFilter === filter
                            ? "bg-foreground/8 text-foreground"
                            : "text-muted-foreground hover:text-foreground"
                        )}
                      >
                        {filter === "all" ? "All" : filter === "passed" ? "Passed" : "Failed"}
                        {count > 0 ? (
                          <span className="ml-1 tabular-nums opacity-60">{count}</span>
                        ) : null}
                      </button>
                    );
                  })}
                </div>
              ) : null}
            </div>
            {renderResultsMatrix({ results: filteredResults, runMeta })}
          </div>
        ) : isActiveRun && renderMatrixSkeleton ? (
          <div className="rounded-lg keyline-card p-4">
            <h3 className="section-label mb-3">Results Matrix</h3>
            {renderMatrixSkeleton({ samplesPerCase: runMeta.config.samplesPerCase })}
          </div>
        ) : null}

        <LiveStatsBar
          results={results}
          passThreshold={runMeta.config.passThreshold}
          summary={summary ?? null}
        />

        {summary && renderSummaryCards ? renderSummaryCards(summary) : null}

        {canActivatePassing && onActivatePassing ? (
          <button
            type="button"
            onClick={onActivatePassing}
            disabled={activatingCount !== null}
            className="w-full text-xs font-medium px-3 py-2 rounded-md border border-[var(--dot-emerald)]/30 text-[var(--dot-emerald)] hover:bg-[var(--accent-emerald)] disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {activatingCount !== null
              ? `Activating... (${activatingCount}/${passingCount})`
              : `Activate ${passingCount} Passing Test Case${passingCount > 1 ? "s" : ""}`}
          </button>
        ) : null}
      </div>
    </div>
  );
}

export function SharedEvalCompareOverlay<
  TComparison extends SharedEvalRunComparisonLike = SharedEvalRunComparisonLike,
  TRun extends SharedEvalCompareRunLike = SharedEvalCompareRunLike,
>({
  currentRunId,
  runs,
  comparison,
  loading,
  onSelectRun,
  onClose,
  buildTestCaseHref = (testCaseId) => `/tests/${testCaseId}`,
}: SharedEvalCompareOverlayProps<TComparison, TRun>) {
  const [selectedId, setSelectedId] = useState("");
  const otherRuns = useMemo(
    () => runs.filter((run) => run.id !== currentRunId),
    [currentRunId, runs]
  );

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onClose();
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [onClose]);

  return (
    <div className="fixed inset-0 z-50 bg-black/40 flex items-center justify-center p-4 animate-overlay-in">
      <div
        className="bg-background border rounded-xl shadow-2xl max-w-2xl w-full max-h-[80vh] overflow-hidden flex flex-col animate-dialog-in"
        role="dialog"
        aria-label="Compare eval runs"
      >
        <div className="flex items-center justify-between px-4 py-3 border-b">
          <h2 className="text-sm font-semibold">Compare Runs</h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close comparison"
            className="text-muted-foreground hover:text-foreground text-xs focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none rounded"
          >
            Close
          </button>
        </div>

        {!comparison ? (
          <div className="p-4 space-y-3">
            <p className="text-xs text-muted-foreground">Select another run to compare against:</p>
            <select
              value={selectedId}
              onChange={(event) => setSelectedId(event.target.value)}
              aria-label="Select eval run to compare"
              className="w-full border rounded-md px-2 py-1.5 text-sm bg-background focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
            >
              <option value="">Choose a run...</option>
              {otherRuns.map((run) => (
                <option key={run.id} value={run.id}>
                  {run.id.slice(0, 8)} — {run.config.mode} ({run.status.status})
                </option>
              ))}
            </select>
            <button
              type="button"
              disabled={!selectedId || loading}
              onClick={() => onSelectRun(selectedId)}
              className="text-xs px-3 py-1.5 rounded-md bg-primary text-primary-foreground disabled:opacity-50 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
            >
              {loading ? "Comparing..." : "Compare"}
            </button>

            {loading ? (
              <div className="space-y-3 animate-pulse">
                <div className="grid grid-cols-3 gap-3">
                  {[1, 2, 3].map((index) => (
                    <div
                      key={`eval-compare-skeleton-summary-${index}`}
                      className="h-16 rounded-lg bg-muted/50"
                    />
                  ))}
                </div>
                <div className="space-y-1.5">
                  {[1, 2, 3, 4].map((index) => (
                    <div
                      key={`eval-compare-skeleton-row-${index}`}
                      className="h-8 rounded bg-muted/30"
                    />
                  ))}
                </div>
              </div>
            ) : null}
          </div>
        ) : (
          <div className="flex-1 overflow-y-auto p-4 space-y-4">
            <div className="grid grid-cols-3 gap-3 text-center">
              <div className="p-2 rounded-lg bg-muted/50">
                <p className="text-[10px] text-muted-foreground">Score Delta</p>
                <p
                  className={cx(
                    "text-lg font-mono font-semibold",
                    comparison.scoreDelta > 0
                      ? "text-[var(--dot-emerald)]"
                      : comparison.scoreDelta < 0
                        ? "text-[var(--dot-red)]"
                        : ""
                  )}
                >
                  {comparison.scoreDelta > 0 ? "+" : ""}
                  {(comparison.scoreDelta * 100).toFixed(1)}%
                </p>
              </div>
              <div className="p-2 rounded-lg bg-muted/50">
                <p className="text-[10px] text-muted-foreground">Pass Rate Delta</p>
                <p
                  className={cx(
                    "text-lg font-mono font-semibold",
                    comparison.passRateDelta > 0
                      ? "text-[var(--dot-emerald)]"
                      : comparison.passRateDelta < 0
                        ? "text-[var(--dot-red)]"
                        : ""
                  )}
                >
                  {comparison.passRateDelta > 0 ? "+" : ""}
                  {(comparison.passRateDelta * 100).toFixed(1)}%
                </p>
              </div>
              <div className="p-2 rounded-lg bg-muted/50">
                <p className="text-[10px] text-muted-foreground">Regressions / Improvements</p>
                <p className="text-lg font-mono font-semibold">
                  <span className="text-[var(--dot-red)]">
                    {comparison.testCaseComparisons.filter((entry) => entry.regression).length}
                  </span>
                  {" / "}
                  <span className="text-[var(--dot-emerald)]">
                    {comparison.testCaseComparisons.filter((entry) => entry.improvement).length}
                  </span>
                </p>
              </div>
            </div>

            <div className="border rounded-lg overflow-hidden">
              <table className="w-full text-xs">
                <thead>
                  <tr className="bg-muted/50 text-muted-foreground">
                    <th className="text-left px-3 py-1.5 font-medium">Test Case</th>
                    <th className="text-right px-3 py-1.5 font-medium">Left</th>
                    <th className="text-right px-3 py-1.5 font-medium">Right</th>
                    <th className="text-right px-3 py-1.5 font-medium">Delta</th>
                    <th className="text-center px-3 py-1.5 font-medium">Status</th>
                  </tr>
                </thead>
                <tbody className="divide-y">
                  {comparison.testCaseComparisons.map((entry) => (
                    <tr
                      key={entry.testCaseId}
                      className={cx(
                        entry.regression && "bg-[var(--accent-red)]",
                        entry.improvement && "bg-[var(--accent-emerald)]"
                      )}
                    >
                      <td className="px-3 py-1.5 font-mono truncate max-w-32">
                        <Link
                          to={buildTestCaseHref(entry.testCaseId)}
                          className="hover:underline text-primary/80 hover:text-primary"
                          title="View test case"
                        >
                          {entry.testCaseId.slice(0, 8)}
                        </Link>
                      </td>
                      <td className="text-right px-3 py-1.5 font-mono tabular-nums">
                        {entry.leftScore != null ? `${(entry.leftScore * 100).toFixed(0)}%` : "—"}
                      </td>
                      <td className="text-right px-3 py-1.5 font-mono tabular-nums">
                        {entry.rightScore != null ? `${(entry.rightScore * 100).toFixed(0)}%` : "—"}
                      </td>
                      <td
                        className={cx(
                          "text-right px-3 py-1.5 font-mono tabular-nums",
                          entry.scoreDelta > 0
                            ? "text-[var(--dot-emerald)]"
                            : entry.scoreDelta < 0
                              ? "text-[var(--dot-red)]"
                              : ""
                        )}
                      >
                        {entry.scoreDelta > 0 ? "+" : ""}
                        {(entry.scoreDelta * 100).toFixed(1)}%
                      </td>
                      <td className="text-center px-3 py-1.5">
                        {entry.regression ? (
                          <span className="text-[var(--dot-red)] font-medium">Regression</span>
                        ) : entry.improvement ? (
                          <span className="text-[var(--dot-emerald)] font-medium">Improved</span>
                        ) : (
                          <span className="text-muted-foreground">—</span>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
