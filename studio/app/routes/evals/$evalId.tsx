import { SharedEvalCompareOverlay, SharedEvalDetailPage } from "@studio/features-evals";
import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate, useParams } from "react-router";
import { TokenUsageCard } from "~/components/eval/cost-summary-card";
import { EvalMatrix, MatrixSkeleton } from "~/components/eval/eval-matrix";
import { EvalProgressBar } from "~/components/eval/eval-progress-bar";
import { EvalStatusTag } from "~/components/eval/eval-status-tag";
import { PassAtKChart } from "~/components/eval/pass-at-k-chart";
import { RerunDialog } from "~/components/eval/rerun-dialog";
import { RunActivitySummaryPanel } from "~/components/runs/run-timeline";
import { CopyButton } from "~/components/shared/copy-button";
import { DataReadinessCard } from "~/components/shared/data-readiness-card";
import { ErrorBanner } from "~/components/shared/error-banner";
import {
  cancelEval as cancelEvalRun,
  compareEvalRuns,
  connectEvalStream,
  exportEvalFailures,
  getEvalRun,
  type RunComparisonSummary,
  rerunEvalCases,
} from "~/lib/api";
import type { EvalStatusKey, TestCaseResult, TestCaseState } from "~/lib/domain/eval";
import { isOk } from "~/lib/domain/result";
import { useRunEvents } from "~/lib/hooks/use-run-events";
import { useScramble } from "~/lib/scramble";
import {
  selectEvalDetailRun,
  selectEvalError,
  selectEvalRuns,
  selectEvalSession,
  useEvaluation,
  useEvaluationActions,
} from "~/lib/stores";

const EMPTY_LIVE_RESULTS: Map<string, TestCaseResult> = new Map();
const EMPTY_TC_STATES: Map<string, TestCaseState> = new Map();
const EMPTY_SKIPPED: Map<string, string> = new Map();

function ReconnectingBanner({
  attempt,
  delayMs,
  startedAt,
}: {
  readonly attempt: number;
  readonly delayMs: number;
  readonly startedAt: number;
}) {
  const [remainingMs, setRemainingMs] = useState(delayMs);

  useEffect(() => {
    const tick = () => {
      const elapsed = Date.now() - startedAt;
      setRemainingMs(Math.max(0, delayMs - elapsed));
    };
    tick();
    const id = setInterval(tick, 250);
    return () => clearInterval(id);
  }, [delayMs, startedAt]);

  const remainingSec = Math.ceil(remainingMs / 1000);

  return (
    <div className="flex items-center gap-2 rounded-md accent-bar-amber bg-[var(--accent-amber)] px-3 py-1.5 text-xs text-[var(--dot-amber)]">
      <span className="status-dot bg-[var(--dot-amber)] animate-pulse" />
      {remainingMs > 0 ? (
        <span>
          Reconnecting in {remainingSec}s (attempt {attempt})
        </span>
      ) : (
        <span>Reconnecting... (attempt {attempt})</span>
      )}
    </div>
  );
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: host orchestration remains app-owned.
export default function EvalDetail() {
  const { evalId } = useParams<{ evalId: string }>();
  const navigate = useNavigate();
  const { s } = useScramble();

  const runs = useEvaluation(selectEvalRuns);
  const detailRun = useEvaluation(selectEvalDetailRun);
  const session = useEvaluation(selectEvalSession(evalId ?? ""));
  const isRunning = session?.runPhase.phase === "running";
  const isPaused = session?.runPhase.phase === "paused";
  const liveProgress = session?.liveProgress ?? null;
  const liveResults = session?.liveResults ?? EMPTY_LIVE_RESULTS;
  const testCaseStates = session?.testCaseStates ?? EMPTY_TC_STATES;
  const evalError = useEvaluation(selectEvalError);
  const skippedTestCases = session?.skippedTestCases ?? EMPTY_SKIPPED;
  const {
    setDetailRun,
    selectRun: setActiveRun,
    interpretEvalEvent: handleEvalEventRaw,
    completeEval: completeEvalRaw,
    cancelEval: cancelEvalSession,
    startEval,
    setError,
  } = useEvaluationActions();

  const handleEvalEvent = useCallback(
    (event: import("~/lib/domain/eval").EvalEvent) => handleEvalEventRaw(evalId ?? "", event),
    [evalId, handleEvalEventRaw]
  );
  const completeEval = useCallback(() => completeEvalRaw(evalId ?? ""), [completeEvalRaw, evalId]);

  const abortRef = useRef<(() => void) | null>(null);
  const [reconnecting, setReconnecting] = useState<{
    readonly attempt: number;
    readonly delayMs: number;
    readonly startedAt: number;
  } | null>(null);
  const [rerunDialogOpen, setRerunDialogOpen] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [compareOpen, setCompareOpen] = useState(false);
  const [comparison, setComparison] = useState<RunComparisonSummary | null>(null);
  const [compareLoading, setCompareLoading] = useState(false);
  const [cancelling, setCancelling] = useState(false);

  useEffect(() => {
    if (!evalId) return;
    setActiveRun(evalId);

    const existing = detailRun?.id === evalId;
    if (!existing) {
      getEvalRun(evalId).then((result) => {
        if (isOk(result)) {
          setDetailRun(result.value);
        } else {
          setError(`Failed to load eval run: ${result.error.message}`);
        }
      });
    }
  }, [detailRun, evalId, setActiveRun, setDetailRun, setError]);

  useEffect(() => {
    if (!evalId) return;
    if (session) return;

    const summary = runs.find((run) => run.id === evalId);
    if (!summary || summary.status.status !== "running") return;

    let cancelled = false;
    let attempt = 0;
    const maxRetries = 5;
    const baseDelayMs = 1000;

    startEval(evalId);

    const connect = async () => {
      while (!cancelled && attempt <= maxRetries) {
        if (attempt > 0) {
          const delay = baseDelayMs * 2 ** (attempt - 1);
          setReconnecting({ attempt, delayMs: delay, startedAt: Date.now() });
          await new Promise((resolve) => setTimeout(resolve, delay));
          if (cancelled) return;
        }

        const result = await connectEvalStream(evalId, {
          onEvent: handleEvalEvent,
          onComplete: () => {
            setReconnecting(null);
            completeEval();
            abortRef.current = null;
          },
          onError: (error) => {
            if (error.code === "NETWORK_ERROR" && attempt < maxRetries) {
              attempt++;
              abortRef.current = null;
              void connect();
              return;
            }
            setReconnecting(null);
            completeEval();
            setError(error.message);
            abortRef.current = null;
            getEvalRun(evalId).then((response) => {
              if (isOk(response)) {
                setDetailRun(response.value);
              }
            });
          },
        });

        if (isOk(result)) {
          setReconnecting(null);
          abortRef.current = result.value.abort;
          attempt = 0;
          return;
        }

        if (result.error.code === "NETWORK_ERROR" && attempt < maxRetries) {
          attempt++;
          continue;
        }

        setReconnecting(null);
        setError(result.error.message);
        return;
      }
    };

    void connect();

    return () => {
      cancelled = true;
      setReconnecting(null);
      abortRef.current?.();
      abortRef.current = null;
    };
  }, [completeEval, evalId, handleEvalEvent, runs, session, setDetailRun, setError, startEval]);

  const prevRunningRef = useRef(false);
  useEffect(() => {
    const wasRunning = prevRunningRef.current;
    const nowIdle = !isRunning && !isPaused;
    prevRunningRef.current = isRunning || isPaused;

    if (wasRunning && nowIdle && evalId) {
      getEvalRun(evalId).then((result) => {
        if (isOk(result)) {
          setDetailRun(result.value);
        }
      });
    }
  }, [evalId, isPaused, isRunning, setDetailRun]);

  useEffect(() => {
    return () => {
      abortRef.current?.();
    };
  }, []);

  const handleCellClick = useCallback(
    (testCaseId: string, sampleIndex: number) => {
      navigate(`/evals/${evalId}/cases/${testCaseId}?sample=${sampleIndex}`);
    },
    [evalId, navigate]
  );

  const handleRerunSubmit = useCallback(
    async (testCaseIds: string[]) => {
      if (!evalId) return;
      const rerunId = crypto.randomUUID();
      startEval(rerunId);
      const result = await rerunEvalCases(evalId, testCaseIds, {
        onEvent: (event) => handleEvalEventRaw(rerunId, event),
        onComplete: () => completeEvalRaw(rerunId),
        onError: (error) => {
          setError(error.message);
          completeEvalRaw(rerunId);
        },
      });

      if (!isOk(result)) {
        completeEvalRaw(rerunId);
      }
    },
    [completeEvalRaw, evalId, handleEvalEventRaw, setError, startEval]
  );

  const handleExportFailures = useCallback(async () => {
    if (!evalId) return;
    setExporting(true);
    const result = await exportEvalFailures(evalId);
    if (!isOk(result)) {
      setError(result.error.message);
    }
    setExporting(false);
  }, [evalId, setError]);

  const handleCancelRun = useCallback(async () => {
    if (!evalId || cancelling) return;
    setCancelling(true);
    const result = await cancelEvalRun(evalId);
    abortRef.current?.();
    abortRef.current = null;
    cancelEvalSession(evalId);
    if (!isOk(result)) {
      setError(result.error.message);
    }
    const refreshed = await getEvalRun(evalId);
    if (isOk(refreshed)) {
      setDetailRun(refreshed.value);
    }
    setCancelling(false);
  }, [cancelEvalSession, cancelling, evalId, setDetailRun, setError]);

  const run = detailRun?.id === evalId ? detailRun : null;
  const runMeta = run ?? runs.find((entry) => entry.id === evalId) ?? null;
  const activityRunId = run?.activityRunId ?? null;
  const isActiveRun = !!session && (isRunning || isPaused);
  const isEvalRunning =
    isActiveRun || runMeta?.status.status === "running" || runMeta?.status.status === "paused";
  const shouldShowRunActivity = !!activityRunId && !isEvalRunning;
  const {
    timelineItems: runActivityItems,
    loading: runActivityLoading,
    error: runActivityError,
  } = useRunEvents(activityRunId, {
    enabled: shouldShowRunActivity,
    poll: false,
  });

  const allResults: readonly TestCaseResult[] = session
    ? Array.from(liveResults.values())
    : (run?.results ?? []);
  const summary = runMeta?.status.status === "completed" ? runMeta.status.summary : null;
  const canCancelRun = !!runMeta && (isActiveRun || runMeta.status.status === "running");
  const hasFailedCases =
    !!runMeta && allResults.some((result) => result.aggregateScore < runMeta.config.passThreshold);
  const canRerun =
    !!runMeta &&
    (runMeta.status.status === "completed" || runMeta.status.status === "failed") &&
    hasFailedCases;

  return (
    <>
      <SharedEvalDetailPage
        loaded={!!runMeta}
        containerClassName="scroll-container"
        runMeta={runMeta}
        results={allResults}
        summary={summary}
        isActiveRun={isActiveRun}
        isPaused={isPaused}
        exporting={exporting}
        error={evalError}
        skippedTestCases={skippedTestCases}
        formatText={s}
        copyIdControl={runMeta ? <CopyButton text={runMeta.id} label="Copy eval run ID" /> : null}
        reconnectingBanner={
          reconnecting ? (
            <ReconnectingBanner
              attempt={reconnecting.attempt}
              delayMs={reconnecting.delayMs}
              startedAt={reconnecting.startedAt}
            />
          ) : null
        }
        progressBar={liveProgress ? <EvalProgressBar progress={liveProgress} /> : null}
        dataReadinessCard={
          runMeta?.dataReadiness ? (
            <DataReadinessCard
              readiness={runMeta.dataReadiness}
              title="Run Data Context"
              mode="bound"
            />
          ) : null
        }
        runActivityPanel={
          shouldShowRunActivity ? (
            <RunActivitySummaryPanel
              runId={activityRunId}
              items={runActivityItems}
              loading={runActivityLoading}
              error={runActivityError}
            />
          ) : null
        }
        enableResultFilter
        renderStatusTag={(status) => <EvalStatusTag status={status as EvalStatusKey} />}
        renderErrorBanner={(message, onDismiss) => (
          <ErrorBanner message={message} onDismiss={onDismiss} />
        )}
        renderResultsMatrix={({ results, runMeta: matrixRunMeta }) => (
          <EvalMatrix
            results={results as readonly TestCaseResult[]}
            samplesPerCase={matrixRunMeta.config.samplesPerCase}
            passThreshold={matrixRunMeta.config.passThreshold}
            testCaseStates={testCaseStates}
            onCellClick={handleCellClick}
            onTestCaseClick={(testCaseId) =>
              navigate(`/evals/${evalId}/cases/${testCaseId}?sample=0`)
            }
          />
        )}
        renderMatrixSkeleton={({ samplesPerCase }) => (
          <MatrixSkeleton rows={4} cols={samplesPerCase} />
        )}
        renderSummaryCards={(currentSummary) =>
          runMeta ? (
            <div className="grid gap-4 grid-cols-2">
              <div className="rounded-lg keyline-card p-4">
                <PassAtKChart
                  results={currentSummary.passAtK}
                  threshold={runMeta.config.passThreshold}
                />
              </div>
              <TokenUsageCard usage={currentSummary.totalUsage} />
            </div>
          ) : null
        }
        onDismissError={() => setError(null)}
        onNavigateParentRun={(parentRunId) => navigate(`/evals/${parentRunId}`)}
        onCancelRun={canCancelRun ? handleCancelRun : undefined}
        cancelling={cancelling}
        onOpenRerun={canRerun ? () => setRerunDialogOpen(true) : undefined}
        onExportFailures={canRerun ? handleExportFailures : undefined}
        onOpenCompare={canRerun ? () => setCompareOpen(true) : undefined}
      />

      {runMeta && canRerun ? (
        <RerunDialog
          open={rerunDialogOpen}
          onOpenChange={setRerunDialogOpen}
          results={allResults}
          passThreshold={runMeta.config.passThreshold}
          onSubmit={handleRerunSubmit}
        />
      ) : null}

      {compareOpen ? (
        <SharedEvalCompareOverlay
          currentRunId={evalId ?? ""}
          runs={runs}
          comparison={comparison}
          loading={compareLoading}
          onSelectRun={async (otherId) => {
            setCompareLoading(true);
            const result = await compareEvalRuns(evalId ?? "", otherId);
            if (isOk(result)) {
              setComparison(result.value);
            }
            setCompareLoading(false);
          }}
          onClose={() => {
            setCompareOpen(false);
            setComparison(null);
          }}
          buildTestCaseHref={(testCaseId) => `/tests/${testCaseId}`}
        />
      ) : null}
    </>
  );
}
