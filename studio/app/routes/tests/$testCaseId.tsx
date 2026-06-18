import { SharedTestCaseDetailPage } from "@studio/features-tests";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useParams } from "react-router";
import { ConfirmDialog } from "~/components/shared/confirm-dialog";
import { CopyButton } from "~/components/shared/copy-button";
import { DataReadinessCard } from "~/components/shared/data-readiness-card";
import { ErrorBanner } from "~/components/shared/error-banner";
import { TestCaseForm } from "~/components/tests/test-case-form";
import type { ValidationIssue } from "~/lib/api";
import {
  createTestCase,
  deleteTestCase,
  getTestCase,
  getThreadTrace,
  listEvalRuns,
  updateTestCase,
  validateTestCase,
} from "~/lib/api";
import type { EvalRunSummary, TrajectoryMode } from "~/lib/domain/eval";
import { isOk } from "~/lib/domain/result";
import type {
  AuthoredTestCase,
  GroundTruth,
  TestCaseStatus,
  ToolCallEntry,
} from "~/lib/domain/test-case";
import {
  formatStructuredGroundTruthJson,
  parseStructuredGroundTruthJson,
  TEST_STATUS_BADGE_CLASS,
} from "~/lib/domain/test-case";
import { useScramble } from "~/lib/scramble";
import {
  selectAvailableTools,
  selectTestCases,
  useTestSuite,
  useTestSuiteActions,
} from "~/lib/stores";

interface FormSnapshot {
  readonly name: string;
  readonly description: string;
  readonly input: string;
  readonly trajectory: string;
  readonly mode: TrajectoryMode;
  readonly structuredGroundTruthJson: string;
  readonly tags: string;
  readonly status: TestCaseStatus;
}

const EMPTY_FORM: FormSnapshot = {
  name: "",
  description: "",
  input: "",
  trajectory: "",
  mode: "unordered",
  structuredGroundTruthJson: "",
  tags: "",
  status: "draft",
};

function snapshotFromTestCase(testCase: AuthoredTestCase): FormSnapshot {
  return {
    name: testCase.name,
    description: testCase.description ?? "",
    input: testCase.input,
    trajectory: testCase.expectedTrajectory.join("\n"),
    mode: testCase.trajectoryMode,
    structuredGroundTruthJson: formatStructuredGroundTruthJson(
      testCase.structuredGroundTruth ?? null
    ),
    tags: testCase.tags.join(", "),
    status: testCase.status,
  };
}

function snapshotsEqual(left: FormSnapshot, right: FormSnapshot): boolean {
  return (
    left.name === right.name &&
    left.description === right.description &&
    left.input === right.input &&
    left.trajectory === right.trajectory &&
    left.mode === right.mode &&
    left.structuredGroundTruthJson === right.structuredGroundTruthJson &&
    left.tags === right.tags &&
    left.status === right.status
  );
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: host orchestration remains app-owned.
export default function TestCaseDetail() {
  const { testCaseId } = useParams<{ testCaseId: string }>();
  const navigate = useNavigate();
  const { s } = useScramble();
  const {
    selectTestCase: setActiveTestCase,
    replaceTestCase: replaceInStore,
    removeTestCase: removeFromStore,
    addTestCase: addToStore,
  } = useTestSuiteActions();
  const testCases = useTestSuite(selectTestCases);
  const availableTools = useTestSuite(selectAvailableTools);

  const [form, setForm] = useState<FormSnapshot>(EMPTY_FORM);
  const [saving, setSaving] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [testCase, setTestCase] = useState<AuthoredTestCase | null>(null);
  const [traceExpanded, setTraceExpanded] = useState(false);
  const [trace, setTrace] = useState<readonly ToolCallEntry[] | null>(null);
  const [validationIssues, setValidationIssues] = useState<ValidationIssue[]>([]);
  const [evalHistory, setEvalHistory] = useState<readonly EvalRunSummary[]>([]);

  const snapshotRef = useRef<FormSnapshot>(EMPTY_FORM);
  const busyRef = useRef(false);

  const isDirty = useMemo(
    () => loaded && !snapshotsEqual(form, snapshotRef.current),
    [form, loaded]
  );
  const trajectoryStepCount = useMemo(
    () =>
      form.trajectory
        .split(/\r?\n/)
        .map((value) => value.trim())
        .filter(Boolean).length,
    [form.trajectory]
  );
  const activeTestCaseIds = useMemo(
    () => testCases.filter((entry) => entry.status === "active").map((entry) => entry.id),
    [testCases]
  );

  const setField = useCallback(<K extends keyof FormSnapshot>(key: K, value: FormSnapshot[K]) => {
    setForm((previous) => ({ ...previous, [key]: value }));
  }, []);

  const populateForm = useCallback((nextTestCase: AuthoredTestCase) => {
    const snapshot = snapshotFromTestCase(nextTestCase);
    setForm(snapshot);
    snapshotRef.current = snapshot;
    setTestCase(nextTestCase);
  }, []);

  useEffect(() => {
    if (!testCaseId) return;
    setActiveTestCase(testCaseId);

    const existing = testCases.find((entry) => entry.id === testCaseId);
    if (existing) {
      populateForm(existing);
      setLoaded(true);
    }

    getTestCase(testCaseId)
      .then((result) => {
        if (isOk(result)) {
          populateForm(result.value);
          replaceInStore(testCaseId, result.value);
          setLoaded(true);
        } else if (!existing) {
          setError(result.error.message);
        }
      })
      .catch((caught) => {
        if (!existing) {
          setError(caught instanceof Error ? caught.message : "Failed to load test case");
        }
      });

    return () => setActiveTestCase(null);
    // testCases is read for store-first hydration; including it directly loops after replaceInStore.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [testCaseId, populateForm, replaceInStore, setActiveTestCase, testCases.find]);

  useEffect(() => {
    if (!testCaseId || !loaded) return;
    validateTestCase(testCaseId).then((result) => {
      if (isOk(result)) {
        setValidationIssues(result.value.issues);
      }
    });
  }, [loaded, testCaseId]);

  useEffect(() => {
    if (!testCaseId) return;
    listEvalRuns({ testCaseId }).then((result) => {
      if (isOk(result)) {
        setEvalHistory(result.value);
      }
    });
  }, [testCaseId]);

  const handleSave = useCallback(async () => {
    if (!testCaseId || !form.input.trim()) {
      setError("Input is required");
      return;
    }

    const expectedTrajectory = form.trajectory
      .split(/\r?\n/)
      .map((value) => value.trim())
      .filter(Boolean);

    if (availableTools.length > 0) {
      const toolNames = new Set(availableTools.map((tool) => tool.name));
      const unknown = expectedTrajectory.filter((toolName) => !toolNames.has(toolName));
      if (unknown.length > 0) {
        setError(`Unknown tools in trajectory: ${unknown.join(", ")}`);
        return;
      }
    }

    let structuredGroundTruth: GroundTruth | null = null;
    try {
      structuredGroundTruth = parseStructuredGroundTruthJson(form.structuredGroundTruthJson);
    } catch (caught) {
      setError(
        caught instanceof Error ? caught.message : "Structured ground truth must be valid JSON."
      );
      return;
    }

    setSaving(true);
    setError(null);

    const result = await updateTestCase(testCaseId, {
      name: form.name.trim() || form.input.slice(0, 80).trim() || "Untitled",
      description: form.description.trim() || null,
      input: form.input.trim(),
      expectedTrajectory,
      trajectoryMode: form.mode,
      structuredGroundTruth,
      tags: form.tags
        .split(",")
        .map((value) => value.trim())
        .filter(Boolean),
      status: form.status,
    });

    setSaving(false);

    if (isOk(result)) {
      replaceInStore(testCaseId, result.value);
      setTestCase(result.value);
      const saved = snapshotFromTestCase(result.value);
      snapshotRef.current = saved;
      setForm(saved);
    } else {
      setError(result.error.message);
    }
  }, [availableTools, form, replaceInStore, testCaseId]);

  const confirmDelete = useCallback(async () => {
    if (!testCaseId) return;
    setDeleteDialogOpen(false);
    setDeleting(true);
    const result = await deleteTestCase(testCaseId);
    setDeleting(false);
    if (isOk(result)) {
      removeFromStore(testCaseId);
      navigate("/tests");
    } else {
      setError(result.error.message);
    }
  }, [navigate, removeFromStore, testCaseId]);

  const handleRunEval = useCallback(
    (allActive: boolean) => {
      if (!testCaseId) return;
      const selectedIds =
        allActive && activeTestCaseIds.length > 0 ? activeTestCaseIds : [testCaseId];
      const errors = validationIssues.filter((issue) => issue.severity === "error");
      if (!allActive && errors.length > 0) {
        if (
          !window.confirm(
            `This test case has ${errors.length} validation error(s):\n\n` +
              errors.map((issue) => `• ${issue.message}`).join("\n") +
              "\n\nRun eval anyway?"
          )
        ) {
          return;
        }
      }
      navigate(`/evals/new?testCaseIds=${encodeURIComponent(selectedIds.join(","))}`);
    },
    [activeTestCaseIds, navigate, testCaseId, validationIssues]
  );

  const handleClone = useCallback(async () => {
    if (!testCase || busyRef.current) return;
    busyRef.current = true;
    try {
      const result = await createTestCase({
        name: `${testCase.name} (copy)`,
        description: testCase.description,
        input: testCase.input,
        expectedTrajectory: [...testCase.expectedTrajectory],
        trajectoryMode: testCase.trajectoryMode,
        groundTruth: testCase.groundTruth,
        structuredGroundTruth: testCase.structuredGroundTruth,
        tags: [...testCase.tags],
        status: "draft",
        trajectoryProvenance: [],
        trajectorySources: [],
        sourceThreadId: null,
        sourceSessionId: null,
        finalResponse: testCase.finalResponse ?? null,
      });
      if (isOk(result)) {
        addToStore(result.value);
        navigate(`/tests/${result.value.id}`);
      } else {
        setError(result.error.message);
      }
    } finally {
      busyRef.current = false;
    }
  }, [addToStore, navigate, testCase]);

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key === "s") {
        event.preventDefault();
        if (!saving) {
          void handleSave();
        }
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [handleSave, saving]);

  useEffect(() => {
    if (!traceExpanded || trace !== null) return;
    const threadId = testCase?.sourceThreadId;
    if (!threadId) return;
    getThreadTrace(threadId).then((result) => {
      if (isOk(result)) {
        setTrace(result.value.toolCalls);
      }
    });
  }, [testCase?.sourceThreadId, trace, traceExpanded]);

  useEffect(() => {
    if (!isDirty) return;
    const handler = (event: BeforeUnloadEvent) => {
      event.preventDefault();
    };
    window.addEventListener("beforeunload", handler);
    return () => window.removeEventListener("beforeunload", handler);
  }, [isDirty]);

  return (
    <>
      <SharedTestCaseDetailPage<TestCaseStatus, AuthoredTestCase>
        loaded={loaded}
        loadingError={error}
        containerClassName="scroll-container"
        testCaseId={testCaseId}
        testCase={testCase}
        status={form.status}
        isDirty={isDirty}
        saving={saving}
        deleting={deleting}
        error={loaded ? error : null}
        trajectoryStepCount={trajectoryStepCount}
        validationIssues={validationIssues}
        traceExpanded={traceExpanded}
        trace={trace}
        evalHistory={evalHistory}
        activeTestCaseCount={activeTestCaseIds.length}
        transitions={[]}
        form={
          <>
            {testCase?.dataReadiness ? (
              <DataReadinessCard
                readiness={testCase.dataReadiness}
                title="Test Data Context"
                mode="bound"
              />
            ) : null}
            <TestCaseForm
              name={form.name}
              onNameChange={(value) => setField("name", value)}
              nameReadOnly
              description={form.description}
              onDescriptionChange={(value) => setField("description", value)}
              input={form.input}
              onInputChange={(value) => setField("input", value)}
              trajectory={form.trajectory}
              onTrajectoryChange={(value) => setField("trajectory", value)}
              mode={form.mode}
              onModeChange={(value) => setField("mode", value)}
              structuredGroundTruthJson={form.structuredGroundTruthJson}
              onStructuredGroundTruthJsonChange={(value) =>
                setField("structuredGroundTruthJson", value)
              }
              tags={form.tags}
              onTagsChange={(value) => setField("tags", value)}
              availableTools={availableTools}
            />
          </>
        }
        copyIdControl={
          testCaseId ? <CopyButton text={testCaseId} label="Copy test case ID" /> : null
        }
        formatText={s}
        renderErrorBanner={(message, onDismiss) => (
          <ErrorBanner message={message} onDismiss={onDismiss} />
        )}
        getStatusBadgeClassName={(status) => TEST_STATUS_BADGE_CLASS[status]}
        onDismissError={() => setError(null)}
        onToggleTraceExpanded={() => setTraceExpanded((previous) => !previous)}
        onSave={() => {
          void handleSave();
        }}
        onTryInChat={() => navigate("/playground")}
        onRunEval={handleRunEval}
        onRefineInBuilder={() => navigate(`/tests/new?prefill=${encodeURIComponent(form.input)}`)}
        onClone={() => {
          void handleClone();
        }}
        onStatusChange={(status) => setField("status", status)}
        onDelete={() => setDeleteDialogOpen(true)}
        onOpenSourceThread={() => {
          if (testCase?.sourceThreadId) {
            navigate(`/chat/${testCase.sourceThreadId}`);
          }
        }}
        onOpenEvalRun={(runId) => navigate(`/evals/${runId}/cases/${testCaseId}`)}
        onOpenMoreEvalRuns={() => navigate("/evals")}
      />
      <ConfirmDialog
        open={deleteDialogOpen}
        title="Delete test case"
        description="Delete this test case from the local Studio store? This cannot be undone."
        confirmLabel={deleting ? "Deleting..." : "Delete"}
        tone="danger"
        onConfirm={confirmDelete}
        onCancel={() => {
          if (!deleting) {
            setDeleteDialogOpen(false);
          }
        }}
      />
    </>
  );
}
