/**
 * New eval page — EvalConfigForm + submit → start eval → navigate to detail.
 *
 * @module routes/evals/new
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { EvalConfigForm } from "~/components/eval/eval-config-form";
import { DataReadinessCard } from "~/components/shared/data-readiness-card";
import { ErrorBanner } from "~/components/shared/error-banner";
import { listEvalCapabilities, listTestCases, startEvalStream } from "~/lib/api";
import { selectDataReadinessForTestSelection } from "~/lib/domain/data-readiness";
import type { EvalCapabilityMode } from "~/lib/domain/eval";
import { isOk } from "~/lib/domain/result";
import type { AuthoredTestCase } from "~/lib/domain/test-case";
import {
  selectEvalConfigDraft,
  selectEvalError,
  selectRunningEvalCount,
  selectTestCaseSets,
  useEvaluation,
  useEvaluationActions,
} from "~/lib/stores";

export default function NewEval() {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const config = useEvaluation(selectEvalConfigDraft);
  const testCaseSets = useEvaluation(selectTestCaseSets);
  const isRunning = useEvaluation(selectRunningEvalCount) > 0;
  const evalError = useEvaluation(selectEvalError);
  const {
    updateConfigDraft: updateConfig,
    startEval,
    interpretEvalEvent: handleEvalEvent,
    completeEval,
    setError,
    addRun,
  } = useEvaluationActions();
  const abortRef = useRef<(() => void) | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [testCases, setTestCases] = useState<AuthoredTestCase[]>([]);
  const [loadingTestCases, setLoadingTestCases] = useState(true);
  const [evalModes, setEvalModes] = useState<EvalCapabilityMode[]>([]);
  const [loadingEvalCapabilities, setLoadingEvalCapabilities] = useState(true);

  // Pre-populate testCaseIds from URL search params (e.g. from Test Detail → Run Eval)
  useEffect(() => {
    const idsParam = searchParams.get("testCaseIds");
    if (idsParam) {
      const ids = idsParam.split(",").filter(Boolean);
      if (ids.length > 0) {
        updateConfig({ testCaseIds: ids });
      }
    }
  }, [searchParams, updateConfig]);

  // Provider/model settings calls are temporarily disabled in harness mode.
  // The Python harness does not currently serve /model-config or provider-model
  // settings routes, so eval creation stays on runtime defaults.
  // useEffect(() => {
  //   loadModelConfig();
  //   loadProviderModels(undefined, false, undefined);
  // }, [loadModelConfig, loadProviderModels]);

  const dataReadinessSelection = useMemo(
    () =>
      selectDataReadinessForTestSelection({
        testCases,
        testCaseSets,
        selectedTestCaseIds: config.testCaseIds,
        selectedTestCaseSetId: config.testCaseSetId,
      }),
    [config.testCaseIds, config.testCaseSetId, testCases, testCaseSets]
  );
  const dataReadiness = dataReadinessSelection.readiness;

  // Fetch test cases from Tests tab on mount.
  // Auto-select all test cases if no URL params specify test case IDs.
  const hasUrlIds = !!searchParams.get("testCaseIds");
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional mount-only effect; config/hasUrlIds captured at mount time
  useEffect(() => {
    listTestCases()
      .then((result) => {
        if (!isOk(result)) {
          setError(`Failed to load test cases: ${result.error.message}`);
          setLoadingTestCases(false);
          return;
        }
        setTestCases(result.value);
        setLoadingTestCases(false);
        // Harness test cases do not persist active/draft/archive status.
        if (!hasUrlIds && (!config.testCaseIds || config.testCaseIds.length === 0)) {
          const ids = result.value.map((tc) => tc.id);
          if (ids.length > 0) {
            updateConfig({ testCaseIds: ids });
          }
        }
      })
      .catch((e) => {
        setError(`Failed to load test cases: ${e instanceof Error ? e.message : "unknown error"}`);
        setLoadingTestCases(false);
      });
  }, [setError]); // eslint-disable-line react-hooks/exhaustive-deps -- run once on mount

  useEffect(() => {
    let cancelled = false;
    listEvalCapabilities()
      .then((result) => {
        if (cancelled) return;
        if (!isOk(result)) {
          setError(`Failed to load eval capabilities: ${result.error.message}`);
          setLoadingEvalCapabilities(false);
          return;
        }
        setEvalModes([...result.value.modes]);
        setLoadingEvalCapabilities(false);
      })
      .catch((e) => {
        if (cancelled) return;
        setError(
          `Failed to load eval capabilities: ${e instanceof Error ? e.message : "unknown error"}`
        );
        setLoadingEvalCapabilities(false);
      });
    return () => {
      cancelled = true;
    };
  }, [setError]);

  useEffect(() => {
    if (loadingEvalCapabilities || evalModes.length === 0) return;
    const currentTargetAgentId = config.targetAgentId ?? null;
    const modeIsAvailable = evalModes.some(
      (mode) => mode.mode === config.mode && (mode.targetAgentId ?? null) === currentTargetAgentId
    );
    if (modeIsAvailable) return;
    const firstMode = evalModes[0];
    updateConfig({
      mode: firstMode.mode,
      targetAgentId: firstMode.targetAgentId ?? null,
    });
  }, [config.mode, config.targetAgentId, evalModes, loadingEvalCapabilities, updateConfig]);

  const pageError = evalError;

  const handleSubmit = useCallback(async () => {
    if (isSubmitting) return;
    setIsSubmitting(true);
    setError(null);
    let activeRunId: string | null = null;
    const result = await startEvalStream(config, {
      onEvent: (event) => {
        // On started event, add the run to the list and navigate
        if (event.type === "started") {
          activeRunId = event.runId;
          startEval(event.runId);
          addRun({
            id: event.runId,
            config: event.config,
            status: {
              status: "running",
              progress: {
                completedSamples: 0,
                totalSamples: 0,
                completedTestCases: 0,
                totalTestCases: 0,
                currentTestCaseId: null,
                elapsedMs: 0,
                estimatedRemainingMs: null,
                testCaseStates: [],
              },
            },
            resultCount: 0,
            dataReadiness: event.dataReadiness ?? dataReadiness,
            createdAt: new Date().toISOString(),
            updatedAt: new Date().toISOString(),
          });
          navigate(`/evals/${event.runId}`);
        }

        if (activeRunId) {
          handleEvalEvent(activeRunId, event);
        }
      },
      onComplete: () => {
        if (activeRunId) completeEval(activeRunId);
        abortRef.current = null;
      },
      onError: (error) => {
        setError(error.message);
        if (activeRunId) completeEval(activeRunId);
        abortRef.current = null;
      },
    });

    if (isOk(result)) {
      abortRef.current = result.value.abort;
    } else {
      setIsSubmitting(false);
      setError(result.error.message);
    }
  }, [
    isSubmitting,
    config,
    startEval,
    handleEvalEvent,
    completeEval,
    setError,
    addRun,
    navigate,
    dataReadiness,
  ]);

  const isLoadingData = loadingTestCases || loadingEvalCapabilities;

  return (
    <div className="flex-1 overflow-y-auto scroll-container">
      <div className="max-w-2xl mx-auto p-8">
        <h1 className="text-xl font-semibold mb-6">New Evaluation</h1>
        {pageError && (
          <ErrorBanner message={pageError} onDismiss={() => setError(null)} className="mb-4" />
        )}
        {isLoadingData && !pageError ? (
          <div className="space-y-4">
            <div className="h-10 w-48 bg-muted rounded animate-shimmer" />
            <div className="h-24 bg-muted rounded-lg animate-shimmer" />
            <div className="h-32 bg-muted rounded-lg animate-shimmer" />
            <div className="h-10 w-32 bg-muted rounded animate-shimmer" />
          </div>
        ) : (
          <div className="space-y-4">
            <DataReadinessCard
              readiness={dataReadiness}
              title="Eval Data Context"
              showEmpty
              emptyMessage={dataReadinessSelection.message}
            />
            <EvalConfigForm
              config={config}
              evalModes={evalModes}
              testCaseSets={testCaseSets}
              testCases={testCases}
              onUpdate={updateConfig}
              onSubmit={handleSubmit}
              isRunning={isRunning || isSubmitting}
            />
          </div>
        )}
      </div>
    </div>
  );
}
