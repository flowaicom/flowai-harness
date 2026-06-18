import {
  type TestSidebarBulkAction,
  type TestSidebarBulkActionResult,
  TestSidebarPane,
} from "@studio/features-tests";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router";
import { ConfirmDialog } from "~/components/shared/confirm-dialog";
import { useLifecycleEvent } from "~/hooks/use-lifecycle-event";
import { batchTestCases, createTestCase, deleteTestCase, listTestCases } from "~/lib/api";
import { isOk } from "~/lib/domain/result";
import type { AuthoredTestCase } from "~/lib/domain/test-case";
import { useScramble } from "~/lib/scramble";
import {
  selectActiveWorkspaceId,
  selectSelectedTestCaseId,
  selectTestCases,
  selectTestFilterTags,
  selectTestLoadError,
  selectTestsLoading,
  useTestSuite,
  useTestSuiteActions,
  useWorkspace,
} from "~/lib/stores";

type TestSidebarFilterStatus = "all" | "active" | "draft" | "archived";

type PendingDelete =
  | { readonly kind: "single"; readonly ids: readonly [string] }
  | { readonly kind: "bulk"; readonly ids: readonly string[] };

export function TestSidebar() {
  const { s } = useScramble();
  const navigate = useNavigate();
  const allTestCases = useTestSuite(selectTestCases);
  const activeTestCaseId = useTestSuite(selectSelectedTestCaseId);
  const isLoading = useTestSuite(selectTestsLoading);
  const error = useTestSuite(selectTestLoadError);
  const filterTags = useTestSuite(selectTestFilterTags);
  const { setTestCases, setLoadPhase, removeTestCase, setFilterTags } = useTestSuiteActions();
  const activeWorkspaceId = useWorkspace(selectActiveWorkspaceId);

  const [filterStatus, setFilterStatus] = useState<TestSidebarFilterStatus>("all");
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<PendingDelete | null>(null);
  const [selectionResetKey, setSelectionResetKey] = useState(0);

  const load = useCallback(async () => {
    setLoadPhase({ phase: "loading" });
    const result = await listTestCases();
    if (isOk(result)) {
      setTestCases(result.value);
    } else {
      setLoadPhase({ phase: "failed", reason: result.error.message });
    }
  }, [setLoadPhase, setTestCases]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: reload test cases when workspace changes
  useEffect(() => {
    load();
  }, [load, activeWorkspaceId]);

  useLifecycleEvent("testCaseCreated", () => {
    load();
  });

  const filteredCases = useMemo(() => {
    const tagFiltered =
      filterTags.length > 0
        ? allTestCases.filter((testCase) => filterTags.every((tag) => testCase.tags.includes(tag)))
        : allTestCases;

    if (filterStatus === "all") {
      return tagFiltered;
    }

    return tagFiltered.filter((testCase) => testCase.status === filterStatus);
  }, [allTestCases, filterStatus, filterTags]);

  const resetSelection = useCallback(() => {
    setSelectionResetKey((value) => value + 1);
  }, []);

  const handleDelete = useCallback((id: string) => {
    setPendingDelete({ kind: "single", ids: [id] });
  }, []);

  const cloneSelected = useCallback(
    async ({
      selectedCases,
    }: {
      readonly selectedCases: readonly AuthoredTestCase[];
    }): Promise<TestSidebarBulkActionResult> => {
      let cloned = 0;
      for (const testCase of selectedCases) {
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
        });
        if (isOk(result)) {
          cloned += 1;
        }
      }

      if (cloned > 0) {
        await load();
      }
      resetSelection();
      return { clearSelection: true };
    },
    [load, resetSelection]
  );

  const bulkActions = useMemo(
    (): readonly TestSidebarBulkAction<AuthoredTestCase>[] => [
      {
        key: "clone",
        label: (selectedCount) => `Clone (${selectedCount})`,
        onExecute: ({ selectedCases }) => cloneSelected({ selectedCases }),
      },
      {
        key: "delete",
        label: "Delete",
        destructive: true,
        onExecute: ({ selectedIds }) => {
          setPendingDelete({ kind: "bulk", ids: selectedIds });
        },
      },
    ],
    [cloneSelected]
  );

  const confirmDelete = useCallback(async () => {
    if (!pendingDelete || deleteBusy) {
      return;
    }

    const ids = [...pendingDelete.ids];
    setDeleteBusy(true);

    if (pendingDelete.kind === "single") {
      const id = ids[0];
      const result = await deleteTestCase(id);
      setDeleteBusy(false);
      setPendingDelete(null);
      if (isOk(result)) {
        removeTestCase(id);
        resetSelection();
        if (id === activeTestCaseId) {
          navigate("/tests");
        }
      } else {
        setLoadPhase({ phase: "failed", reason: result.error.message });
      }
      return;
    }

    const result = await batchTestCases({ action: "delete", ids });
    setDeleteBusy(false);
    setPendingDelete(null);
    if (isOk(result)) {
      await load();
      resetSelection();
    } else {
      setLoadPhase({ phase: "failed", reason: result.error.message });
    }
  }, [
    activeTestCaseId,
    deleteBusy,
    load,
    navigate,
    pendingDelete,
    removeTestCase,
    resetSelection,
    setLoadPhase,
  ]);

  return (
    <>
      <TestSidebarPane
        cases={allTestCases}
        filteredCases={filteredCases}
        filterStatus={filterStatus}
        filterTags={filterTags}
        selectedTestCaseId={activeTestCaseId}
        isLoading={isLoading}
        error={error}
        onFilterStatusChange={setFilterStatus}
        onFilterTagsChange={(tags) => setFilterTags([...tags])}
        onRetryLoad={load}
        onDismissError={() => setLoadPhase({ phase: "idle" })}
        onOpenTestCase={(id) => navigate(`/tests/${id}`)}
        onCreateTestCase={() => navigate("/tests/new")}
        onDeleteTestCase={handleDelete}
        onRunEval={(ids) => navigate(`/evals/new?testCaseIds=${encodeURIComponent(ids.join(","))}`)}
        bulkActions={bulkActions}
        formatText={s}
        showStatusFilter={false}
        selectionResetKey={selectionResetKey}
      />
      <ConfirmDialog
        open={pendingDelete !== null}
        title={pendingDelete?.kind === "bulk" ? "Delete test cases" : "Delete test case"}
        description={
          pendingDelete?.kind === "bulk"
            ? `Delete ${pendingDelete.ids.length} selected test cases from the local Studio store? This cannot be undone.`
            : "Delete this test case from the local Studio store? This cannot be undone."
        }
        confirmLabel={deleteBusy ? "Deleting..." : "Delete"}
        tone="danger"
        onConfirm={confirmDelete}
        onCancel={() => {
          if (!deleteBusy) {
            setPendingDelete(null);
          }
        }}
      />
    </>
  );
}
