/**
 * Eval test case drill-down route.
 *
 * Shows trajectory thread view with sample tabs, score breakdown,
 * and metrics. Supports keyboard navigation.
 *
 * Keyboard shortcuts:
 * - Escape: back to eval dashboard
 * - Left/Right: switch samples
 * - J/K: prev/next test case
 *
 * @module routes/evals/$evalId.cases.$testCaseId
 */

import { SharedEvalCaseThreadView } from "@studio/features-evals";
import { NetworkIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router";
import { ChatReplayView } from "~/components/eval/chat-replay-view";
import { TrajectoryThread } from "~/components/eval/trajectory-thread";
import { RunActivitySummaryPanel } from "~/components/runs/run-timeline";
import { ErrorBanner } from "~/components/shared/error-banner";
import { createTestCase, getEvalRun, getTestCase, updateTestCase } from "~/lib/api";
import type { SampleResult, TestCaseResult } from "~/lib/domain/eval";
import { extractSampleScore, synthesizeTrajectorySteps } from "~/lib/domain/eval";
import { isOk } from "~/lib/domain/result";
import type { AuthoredTestCase } from "~/lib/domain/test-case";
import { useRunEvents } from "~/lib/hooks/use-run-events";
import {
  selectEvalDetailRun,
  selectEvalSessionResults,
  useEvaluation,
  useEvaluationActions,
} from "~/lib/stores";

/** Pure: handle sample switching via Left/Right keys. Returns true if handled. */
function handleSampleNav(
  key: string,
  sampleIndex: number,
  sampleCount: number,
  onSampleSelect: (i: number) => void
): boolean {
  if (key === "ArrowLeft" && sampleIndex > 0) {
    onSampleSelect(sampleIndex - 1);
    return true;
  }
  if (key === "ArrowRight" && sampleIndex < sampleCount - 1) {
    onSampleSelect(sampleIndex + 1);
    return true;
  }
  return false;
}

/** Pure: handle test case switching via J/K keys. Returns true if handled. */
function handleCaseNav(
  key: string,
  testCaseIds: string[],
  testCaseId: string,
  evalId: string,
  navigate: (path: string) => void
): boolean {
  const currentIdx = testCaseIds.indexOf(testCaseId);
  if ((key === "j" || key === "J") && currentIdx < testCaseIds.length - 1) {
    navigate(`/evals/${evalId}/cases/${testCaseIds[currentIdx + 1]}?sample=0`);
    return true;
  }
  if ((key === "k" || key === "K") && currentIdx > 0) {
    navigate(`/evals/${evalId}/cases/${testCaseIds[currentIdx - 1]}?sample=0`);
    return true;
  }
  return false;
}

function useKeyboardNav(params: {
  onBack: () => void;
  sampleIndex: number;
  sampleCount: number;
  onSampleSelect: (i: number) => void;
  testCaseIds: string[];
  testCaseId: string;
  evalId: string;
  navigate: (path: string) => void;
}) {
  const {
    onBack,
    sampleIndex,
    sampleCount,
    onSampleSelect,
    testCaseIds,
    testCaseId,
    evalId,
    navigate,
  } = params;

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      if (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)
        return;

      if (e.key === "Escape") {
        onBack();
        return;
      }

      handleSampleNav(e.key, sampleIndex, sampleCount, onSampleSelect) ||
        handleCaseNav(e.key, testCaseIds, testCaseId, evalId, navigate);
    };

    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [onBack, sampleIndex, sampleCount, onSampleSelect, testCaseIds, testCaseId, evalId, navigate]);
}

export default function EvalCaseDetail() {
  const { evalId, testCaseId } = useParams<{ evalId: string; testCaseId: string }>();
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams();

  const detailRunFromStore = useEvaluation(selectEvalDetailRun);
  const liveResults = useEvaluation(selectEvalSessionResults(evalId ?? ""));
  const { setDetailRun } = useEvaluationActions();

  const [testCase, setTestCase] = useState<AuthoredTestCase | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const run = detailRunFromStore?.id === evalId ? detailRunFromStore : null;
  const activityRunId = run?.activityRunId ?? null;
  const shouldShowRunActivity =
    !!activityRunId && run?.status.status !== "running" && run?.status.status !== "paused";
  const {
    timelineItems: runActivityItems,
    loading: runActivityLoading,
    error: runActivityError,
  } = useRunEvents(activityRunId, { enabled: shouldShowRunActivity, poll: false });

  const sampleIndex = Number(searchParams.get("sample") ?? "0");

  // Fetch eval run if not already loaded as detail
  useEffect(() => {
    if (!evalId) return;
    if (detailRunFromStore?.id === evalId) return;
    getEvalRun(evalId).then((result) => {
      if (isOk(result)) {
        setDetailRun(result.value);
      } else {
        setLoadError(`Failed to load eval run: ${result.error.message}`);
      }
    });
  }, [evalId, detailRunFromStore, setDetailRun]);

  // Fetch test case details
  useEffect(() => {
    if (!testCaseId) return;
    getTestCase(testCaseId).then((result) => {
      if (isOk(result)) {
        setTestCase(result.value);
      } else {
        setLoadError(result.error.message);
      }
    });
  }, [testCaseId]);

  // Get result from store (live or persisted via detail run)
  const displayResults: readonly TestCaseResult[] = useMemo(() => {
    if (liveResults.size > 0) return Array.from(liveResults.values());
    return run?.results ?? [];
  }, [run, liveResults]);

  const result = displayResults.find((r) => r.testCaseId === testCaseId);

  // Ordered test case IDs for J/K navigation
  const testCaseIds = useMemo(() => displayResults.map((r) => r.testCaseId), [displayResults]);

  const handleBack = useCallback(() => {
    navigate(`/evals/${evalId}`);
  }, [navigate, evalId]);

  const handleUpdateExpected = useCallback(
    async (actualTrajectory: readonly string[]) => {
      if (!testCaseId) return;
      if (
        !window.confirm(
          "Replace the test case's expected trajectory with this sample's actual trajectory?"
        )
      )
        return;

      const result = await updateTestCase(testCaseId, {
        expectedTrajectory: [...actualTrajectory],
      });
      if (isOk(result)) {
        setTestCase(result.value);
      } else {
        setLoadError(result.error.message);
      }
    },
    [testCaseId]
  );

  // "Create Test Case from Failure" — capture actual trajectory as new test case
  const handleCreateTestCase = useCallback(
    async (actualTrajectory: readonly string[]) => {
      if (!testCase || !evalId) return;

      const result = await createTestCase({
        name: `${testCase.name || testCase.input.slice(0, 60)} (failure)`,
        description: null,
        input: testCase.input,
        expectedTrajectory: [...actualTrajectory],
        trajectoryMode: testCase.trajectoryMode,
        groundTruth: null,
        structuredGroundTruth: null,
        tags: ["from-eval-failure", `eval:${evalId.slice(0, 8)}`],
        status: "draft",
        trajectoryProvenance: [],
        trajectorySources: [],
        sourceThreadId: null,
        sourceSessionId: null,
      });

      if (isOk(result)) {
        navigate(`/tests/${result.value.id}`);
      } else {
        setLoadError(result.error.message);
      }
    },
    [testCase, evalId, navigate]
  );

  const handleSampleSelect = useCallback(
    (index: number) => {
      setSearchParams({ sample: String(index) });
    },
    [setSearchParams]
  );

  // Keyboard shortcuts
  useKeyboardNav({
    onBack: handleBack,
    sampleIndex,
    sampleCount: result?.samples.length ?? 0,
    onSampleSelect: handleSampleSelect,
    testCaseIds,
    testCaseId: testCaseId ?? "",
    evalId: evalId ?? "",
    navigate,
  });

  // Error state
  if (loadError) {
    return (
      <div className="flex-1 overflow-y-auto scroll-container">
        <div className="max-w-3xl mx-auto p-8">
          <ErrorBanner
            message={`Failed to load: ${loadError}`}
            onDismiss={() => setLoadError(null)}
          />
        </div>
      </div>
    );
  }

  // Loading state
  if (!result || !testCase) {
    return (
      <div className="flex-1 overflow-y-auto scroll-container">
        <div className="max-w-3xl mx-auto p-8 space-y-6">
          {/* Breadcrumb skeleton */}
          <div className="flex items-center justify-between">
            <div className="h-5 w-32 bg-muted rounded animate-shimmer" />
            <div className="h-5 w-24 bg-muted rounded animate-shimmer" />
          </div>
          {/* Tab bar skeleton */}
          <div className="flex gap-2">
            {Array.from({ length: 3 }, (_, i) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: static skeleton
              <div key={i} className="h-8 w-24 bg-muted rounded-full animate-shimmer" />
            ))}
          </div>
          {/* Message skeleton */}
          <div className="flex justify-end">
            <div className="h-12 w-64 bg-muted rounded-2xl animate-shimmer" />
          </div>
          {/* Step skeletons */}
          {Array.from({ length: 4 }, (_, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: static skeleton
            <div key={i} className="h-10 bg-muted rounded-lg animate-shimmer" />
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto scroll-container">
      {shouldShowRunActivity && (
        <div className="max-w-3xl mx-auto px-8 pt-8">
          <RunActivitySummaryPanel
            runId={activityRunId}
            items={runActivityItems}
            loading={runActivityLoading}
            error={runActivityError}
          />
        </div>
      )}
      <SharedEvalCaseThreadView<SampleResult, TestCaseResult>
        evalId={evalId ?? ""}
        testCaseId={testCaseId ?? ""}
        result={result}
        input={testCase.input}
        expectedTrajectory={testCase.expectedTrajectory}
        trajectoryMode={testCase.trajectoryMode}
        selectedSampleIndex={sampleIndex}
        onSampleSelect={handleSampleSelect}
        onBack={handleBack}
        onUpdateExpected={handleUpdateExpected}
        onCreateTestCase={handleCreateTestCase}
        passThreshold={run?.config.passThreshold}
        formatText={(value) => value}
        getSampleScore={extractSampleScore}
        editTestCaseHref={`/tests/${testCaseId ?? ""}`}
        refineInBuilderHref={`/tests/new?prefill=${encodeURIComponent(testCase.input)}`}
        openThreadHref={(threadId) => `/chat/${threadId}`}
        renderSampleAccessory={(sample) =>
          sample.trace?.traceId ? (
            <Link
              to={`/evals/${evalId}/cases/${testCaseId}/traces/${sample.trace.traceId}?sample=${sampleIndex}`}
              className="flex items-center gap-1 px-2 py-1.5 rounded-md text-xs text-muted-foreground hover:text-foreground hover:bg-muted transition-colors"
              title="Open persisted trace"
            >
              <NetworkIcon className="size-3" />
              <span className="hidden sm:inline">Open Trace</span>
            </Link>
          ) : null
        }
        renderChatReplay={({ threadId }) => <ChatReplayView threadId={threadId} />}
        renderTrajectory={(sample) => (
          <TrajectoryThread
            input={testCase.input}
            steps={synthesizeTrajectorySteps(
              sample.actualTrajectory,
              testCase.expectedTrajectory,
              testCase.trajectoryMode
            )}
            sample={sample}
            passThreshold={run?.config.passThreshold}
          />
        )}
      />
    </div>
  );
}
