import type { DataReadiness } from "./data";

export type DataReadinessMode = "current" | "bound";

export type DataReadinessSelectionStatus =
  | "noSelection"
  | "noMatchingCases"
  | "unbound"
  | "partiallyUnbound"
  | "ready"
  | "empty"
  | "mixed";

export interface DataReadinessSummary {
  readonly tableCount: number;
  readonly totalRows: number;
  readonly documentCount: number;
  readonly knowledgeCount: number;
  readonly profiledTableCount: number;
  readonly bundleStatus: string;
  readonly statusLabel: "Ready" | "Empty";
}

export interface DataReadinessSelection {
  readonly readiness: DataReadiness | null;
  readonly status: DataReadinessSelectionStatus;
  readonly selectedCaseCount: number;
  readonly workspaceIds: readonly string[];
  readonly message: string;
}

export interface DataReadinessSelectableTestCase {
  readonly id: string;
  readonly dataReadiness?: DataReadiness | null;
}

export interface DataReadinessSelectableTestCaseSet {
  readonly id: string;
  readonly testCases?: readonly { readonly id: string }[];
  readonly testCaseIds?: readonly string[];
}

export function summarizeDataReadiness(readiness: DataReadiness): DataReadinessSummary {
  return {
    tableCount: readiness.targetTables.length,
    totalRows: Object.values(readiness.tableRowCounts).reduce(
      (total, rowCount) => total + rowCount,
      0
    ),
    documentCount: readiness.documents.ingested,
    knowledgeCount: readiness.knowledge.itemsExtracted,
    profiledTableCount: readiness.catalogProfile.profiledTables.length,
    bundleStatus: readiness.dataBundle?.status ?? (readiness.ready ? "ready" : "empty"),
    statusLabel: readiness.ready ? "Ready" : "Empty",
  };
}

export function dataReadinessCaption(
  readiness: DataReadiness,
  mode: DataReadinessMode = "current"
): string {
  if (!readiness.ready) {
    return `Workspace ${readiness.workspaceId} has no ingested data bundle yet. Data-dependent chat and evals should ingest or switch workspace data first.`;
  }

  const { bundleStatus } = summarizeDataReadiness(readiness);
  const source = readiness.importJobId ? ` from import ${readiness.importJobId}` : "";

  const subject = mode === "bound" ? "artifact" : "workspace";
  const verb = mode === "bound" ? "was created with" : "uses";
  return `This ${subject} ${verb} workspace ${readiness.workspaceId}'s ${bundleStatus} data bundle${source}.`;
}

function dataReadinessContextKey(readiness: DataReadiness): string {
  return [
    readiness.workspaceId,
    readiness.status,
    readiness.sourceId ?? "",
    readiness.importJobId ?? "",
    readiness.profileJobId ?? "",
    readiness.generatedAt,
  ].join("\0");
}

function testCaseIdsForSet(set: DataReadinessSelectableTestCaseSet | undefined): readonly string[] {
  if (!set) return [];
  if (Array.isArray(set.testCaseIds) && set.testCaseIds.length > 0) return set.testCaseIds;
  return set.testCases?.map((testCase) => testCase.id) ?? [];
}

export function selectDataReadinessForTestSelection(args: {
  readonly testCases: readonly DataReadinessSelectableTestCase[];
  readonly testCaseSets?: readonly DataReadinessSelectableTestCaseSet[];
  readonly selectedTestCaseIds?: readonly string[] | null;
  readonly selectedTestCaseSetId?: string | null;
  readonly selectedTestCaseSetIds?: readonly (string | null | undefined)[] | null;
}): DataReadinessSelection {
  const selectedIds = new Set(args.selectedTestCaseIds ?? []);
  const selectedSetIds = [
    ...(args.selectedTestCaseSetId ? [args.selectedTestCaseSetId] : []),
    ...(args.selectedTestCaseSetIds ?? []).filter((id): id is string => !!id),
  ];
  const hasExplicitSelection = selectedIds.size > 0 || selectedSetIds.length > 0;
  const unresolvedSetIds: string[] = [];
  for (const setId of selectedSetIds) {
    const set = args.testCaseSets?.find((candidate) => candidate.id === setId);
    if (!set) unresolvedSetIds.push(setId);
    for (const testCaseId of testCaseIdsForSet(set)) selectedIds.add(testCaseId);
  }

  if (selectedIds.size === 0) {
    if (hasExplicitSelection) {
      return {
        readiness: null,
        status: "noMatchingCases",
        selectedCaseCount: 0,
        workspaceIds: [],
        message:
          "The selected test cases or test-case sets are not loaded in Studio yet, so data context is unknown.",
      };
    }

    return {
      readiness: null,
      status: "noSelection",
      selectedCaseCount: 0,
      workspaceIds: [],
      message: "Select test cases or a test-case set to bind a workspace data context.",
    };
  }

  const selectedCases = args.testCases.filter((testCase) => selectedIds.has(testCase.id));
  const missingCaseCount = selectedIds.size - selectedCases.length;
  if (selectedCases.length === 0 || missingCaseCount > 0 || unresolvedSetIds.length > 0) {
    return {
      readiness: null,
      status: "noMatchingCases",
      selectedCaseCount: selectedCases.length,
      workspaceIds: [],
      message:
        "Some selected test cases or test-case sets are not loaded in Studio yet, so data context is unknown.",
    };
  }

  const readinessByContext = new Map<string, DataReadiness>();
  const workspaceIds = new Set<string>();
  let unboundCaseCount = 0;
  for (const testCase of selectedCases) {
    const readiness = testCase.dataReadiness ?? null;
    if (!readiness) {
      unboundCaseCount += 1;
      continue;
    }
    readinessByContext.set(dataReadinessContextKey(readiness), readiness);
    workspaceIds.add(readiness.workspaceId);
  }

  if (readinessByContext.size === 0) {
    return {
      readiness: null,
      status: "unbound",
      selectedCaseCount: selectedCases.length,
      workspaceIds: [],
      message: "The selected test cases do not carry a workspace data-readiness snapshot.",
    };
  }

  const workspaceIdList = [...workspaceIds].sort();
  if (readinessByContext.size > 1) {
    return {
      readiness: null,
      status: "mixed",
      selectedCaseCount: selectedCases.length,
      workspaceIds: workspaceIdList,
      message:
        workspaceIdList.length > 1
          ? `Selected test cases span multiple workspaces: ${workspaceIdList.join(", ")}. Split the run by workspace before evaluating.`
          : "Selected test cases were authored against multiple data snapshots. Split the run or re-author the tests against one workspace data context.",
    };
  }

  if (unboundCaseCount > 0) {
    return {
      readiness: null,
      status: "partiallyUnbound",
      selectedCaseCount: selectedCases.length,
      workspaceIds: workspaceIdList,
      message:
        "Some selected test cases do not carry a workspace data-readiness snapshot. Re-author or split the run before evaluating.",
    };
  }

  const readiness = [...readinessByContext.values()][0];
  return {
    readiness,
    status: readiness.ready ? "ready" : "empty",
    selectedCaseCount: selectedCases.length,
    workspaceIds: workspaceIdList,
    message: dataReadinessCaption(readiness, "bound"),
  };
}

export function dataReadinessGeneratedAtLabel(readiness: DataReadiness): string {
  const date = new Date(readiness.generatedAt);
  if (Number.isNaN(date.getTime())) return readiness.generatedAt;
  return date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}
