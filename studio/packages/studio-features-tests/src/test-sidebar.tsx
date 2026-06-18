import {
  BookCheckIcon,
  ChevronDownIcon,
  FlaskConicalIcon,
  PlusIcon,
  SearchIcon,
  TrashIcon,
  XIcon,
} from "lucide-react";
import { type ReactNode, useEffect, useMemo, useState } from "react";
import type { SharedTestCaseStatus } from "./domain";
import {
  collectTestSidebarTags,
  countTestCasesByStatus,
  extractTestCaseLevel,
  filterTestCasesByQuery,
  type TestSidebarCaseLike,
  type TestSidebarFilterValue,
} from "./test-sidebar-model";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

const TEST_STATUS_DOT_CLASS: Record<SharedTestCaseStatus, string> = {
  draft: "bg-muted-foreground/50",
  active: "bg-[var(--dot-emerald)]",
  archived: "bg-[var(--dot-amber)]",
};

const FILTER_ORDER: readonly TestSidebarFilterValue[] = ["all", "active", "draft", "archived"];
const FILTER_LABELS: Record<TestSidebarFilterValue, string> = {
  all: "All",
  active: "Active",
  draft: "Draft",
  archived: "Archived",
};

function StatusDot({ status }: { readonly status: SharedTestCaseStatus }) {
  return <span className={cx("status-dot", TEST_STATUS_DOT_CLASS[status])} />;
}

function BulkActionButton({
  disabled,
  destructive,
  onClick,
  label,
}: {
  readonly disabled: boolean;
  readonly destructive?: boolean;
  readonly onClick: () => void;
  readonly label: string;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={cx(
        "rounded border px-2 py-1 text-[10px] transition-colors disabled:opacity-50",
        destructive
          ? "border-destructive/30 text-destructive hover:bg-destructive/10"
          : "hover:bg-muted"
      )}
    >
      {label}
    </button>
  );
}

function FilterPills({
  current,
  counts,
  onChange,
}: {
  readonly current: TestSidebarFilterValue;
  readonly counts: Record<TestSidebarFilterValue, number>;
  readonly onChange: (value: TestSidebarFilterValue) => void;
}) {
  return (
    <div className="flex gap-0.5 border-b px-3 py-2" role="tablist">
      {FILTER_ORDER.map((value) => {
        const count = counts[value];
        const isActive = current === value;
        return (
          <button
            key={value}
            type="button"
            role="tab"
            aria-selected={isActive}
            onClick={() => onChange(value)}
            className={cx(
              "rounded-md px-2.5 py-1 text-xs font-medium transition-colors",
              isActive
                ? "bg-foreground/8 text-foreground"
                : "text-muted-foreground hover:bg-muted/60 hover:text-foreground"
            )}
          >
            {FILTER_LABELS[value]}
            {count > 0 ? (
              <span className={cx("ml-1 tabular-nums", isActive ? "opacity-70" : "opacity-50")}>
                {count}
              </span>
            ) : null}
          </button>
        );
      })}
    </div>
  );
}

function SidebarSearch({
  value,
  onChange,
}: {
  readonly value: string;
  readonly onChange: (value: string) => void;
}) {
  return (
    <div className="border-b px-3 py-2">
      <div className="flex items-center gap-2 rounded-md bg-muted/50 px-2.5 py-1.5">
        <SearchIcon className="size-3.5 shrink-0 text-muted-foreground" />
        <input
          type="text"
          value={value}
          onChange={(event) => onChange(event.target.value)}
          placeholder="Search test cases..."
          aria-label="Search test cases"
          className="flex-1 bg-transparent text-xs placeholder:text-muted-foreground/60 focus:outline-none"
        />
      </div>
    </div>
  );
}

export interface SharedTestSidebarCaseLike extends TestSidebarCaseLike {
  readonly structuredGroundTruth?: unknown | null;
}

export interface TestSidebarBulkActionResult {
  readonly nextSelectedIds?: readonly string[];
  readonly clearSelection?: boolean;
}

export interface TestSidebarBulkAction<TCase extends SharedTestSidebarCaseLike> {
  readonly key: string;
  readonly label: string | ((selectedCount: number) => string);
  readonly destructive?: boolean;
  readonly onExecute: (input: {
    readonly selectedIds: readonly string[];
    readonly selectedCases: readonly TCase[];
  }) => Promise<TestSidebarBulkActionResult | void> | TestSidebarBulkActionResult | void;
}

export interface TestSidebarPaneProps<TCase extends SharedTestSidebarCaseLike> {
  readonly cases: readonly TCase[];
  readonly filteredCases: readonly TCase[];
  readonly filterStatus: TestSidebarFilterValue;
  readonly filterTags: readonly string[];
  readonly selectedTestCaseId?: string | null;
  readonly isLoading: boolean;
  readonly error?: string | null;
  readonly nav?: ReactNode;
  readonly footer?: ReactNode;
  readonly onFilterStatusChange: (value: TestSidebarFilterValue) => void;
  readonly onFilterTagsChange: (tags: readonly string[]) => void;
  readonly onRetryLoad: () => void;
  readonly onDismissError: () => void;
  readonly onOpenTestCase: (id: string) => void;
  readonly onCreateTestCase: () => void;
  readonly onDeleteTestCase?: (id: string) => Promise<void> | void;
  readonly onRunEval?: (ids: readonly string[]) => void;
  readonly bulkActions?: readonly TestSidebarBulkAction<TCase>[];
  readonly formatText?: (value: string) => string;
  readonly showStatusFilter?: boolean;
  readonly selectionResetKey?: unknown;
}

export function TestSidebarPane<TCase extends SharedTestSidebarCaseLike>({
  cases,
  filteredCases,
  filterStatus,
  filterTags,
  selectedTestCaseId,
  isLoading,
  error,
  nav,
  footer,
  onFilterStatusChange,
  onFilterTagsChange,
  onRetryLoad,
  onDismissError,
  onOpenTestCase,
  onCreateTestCase,
  onDeleteTestCase,
  onRunEval,
  bulkActions = [],
  formatText = (value) => value,
  showStatusFilter = true,
  selectionResetKey,
}: TestSidebarPaneProps<TCase>) {
  const [searchQuery, setSearchQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [tagsExpanded, setTagsExpanded] = useState(false);
  const [multiSelect, setMultiSelect] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [batchBusy, setBatchBusy] = useState(false);

  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedQuery(searchQuery), 200);
    return () => window.clearTimeout(timer);
  }, [searchQuery]);

  useEffect(() => {
    setSelectedIds(new Set());
  }, [selectionResetKey]);

  const allTags = useMemo(() => collectTestSidebarTags(cases), [cases]);
  const statusCounts = useMemo(() => countTestCasesByStatus(cases), [cases]);
  const displayedCases = useMemo(
    () => filterTestCasesByQuery(filteredCases, debouncedQuery),
    [filteredCases, debouncedQuery]
  );
  const totalCount = cases.length;

  const selectedCases = useMemo(
    () => cases.filter((testCase) => selectedIds.has(testCase.id)),
    [cases, selectedIds]
  );

  const runEvalIds = useMemo(() => {
    if (filterStatus !== "all") {
      return filteredCases.map((testCase) => testCase.id);
    }
    const activeIds = cases
      .filter((testCase) => testCase.status === "active")
      .map((testCase) => testCase.id);
    return activeIds.length > 0 ? activeIds : cases.map((testCase) => testCase.id);
  }, [cases, filterStatus, filteredCases]);

  const runEvalTitle =
    filterStatus !== "all"
      ? `Run eval on ${filteredCases.length} ${filterStatus} cases`
      : statusCounts.active > 0
        ? `Run eval on ${statusCounts.active} active cases`
        : `Run eval on all ${totalCount} cases`;

  const handleCaseKeyDown = (event: React.KeyboardEvent<HTMLDivElement>, id: string) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onOpenTestCase(id);
    }
  };

  return (
    <div className="flex h-full flex-col">
      {nav}

      <div className="flex items-center justify-between border-b px-4 py-3">
        <h2 className="text-sm font-semibold text-foreground">
          Tests
          {totalCount > 0 ? (
            <span className="ml-1.5 font-normal tabular-nums text-muted-foreground">
              {totalCount}
            </span>
          ) : null}
        </h2>
        <div className="flex items-center gap-1">
          {totalCount > 1 ? (
            <button
              type="button"
              onClick={() => {
                setMultiSelect((previous) => !previous);
                setSelectedIds(new Set());
              }}
              className={cx(
                "rounded-md px-2 py-1 text-[10px] font-medium transition-colors",
                multiSelect ? "bg-primary/10 text-primary" : "text-muted-foreground hover:bg-muted"
              )}
            >
              {multiSelect ? "Done" : "Select"}
            </button>
          ) : null}
          {totalCount > 0 && onRunEval ? (
            <button
              type="button"
              onClick={() => onRunEval(runEvalIds)}
              className="rounded-md p-1.5 transition-colors hover:bg-muted"
              aria-label={
                filterStatus !== "all"
                  ? `Run eval on ${filteredCases.length} ${filterStatus} test cases`
                  : `Run eval on ${statusCounts.active || totalCount} test cases`
              }
              title={runEvalTitle}
            >
              <FlaskConicalIcon className="size-4" />
            </button>
          ) : null}
          <button
            type="button"
            onClick={onCreateTestCase}
            className="rounded-md p-1.5 transition-colors hover:bg-muted"
            aria-label="New test case"
          >
            <PlusIcon className="size-4" />
          </button>
        </div>
      </div>

      {showStatusFilter ? (
        <FilterPills current={filterStatus} counts={statusCounts} onChange={onFilterStatusChange} />
      ) : null}

      {allTags.length > 0 ? (
        <div className="border-b">
          <button
            type="button"
            onClick={() => setTagsExpanded((previous) => !previous)}
            className="flex w-full items-center gap-1.5 px-3 py-1.5 text-[10px] text-muted-foreground transition-colors hover:text-foreground"
          >
            <ChevronDownIcon
              className={cx("size-3 transition-transform", !tagsExpanded && "-rotate-90")}
            />
            <span className="font-medium uppercase tracking-wider">Tags</span>
            {filterTags.length > 0 ? (
              <span className="ml-auto tabular-nums text-[var(--dot-blue)]">
                {filterTags.length} selected
              </span>
            ) : null}
          </button>
          {tagsExpanded ? (
            <div className="flex flex-wrap items-center gap-1 px-3 pb-1.5">
              {allTags.map((tag) => {
                const isActive = filterTags.includes(tag);
                return (
                  <button
                    key={tag}
                    type="button"
                    onClick={() => {
                      if (isActive) {
                        onFilterTagsChange(filterTags.filter((value) => value !== tag));
                      } else {
                        onFilterTagsChange([...filterTags, tag]);
                      }
                    }}
                    className={cx(
                      "rounded border px-1.5 py-0.5 text-[10px] font-medium transition-colors",
                      isActive
                        ? "border-[var(--dot-blue)]/30 bg-[var(--accent-blue)] text-foreground"
                        : "border-border/50 bg-transparent text-muted-foreground hover:border-border hover:text-foreground"
                    )}
                  >
                    {formatText(tag)}
                  </button>
                );
              })}
              {filterTags.length > 0 ? (
                <button
                  type="button"
                  onClick={() => onFilterTagsChange([])}
                  className="ml-1 text-[10px] text-muted-foreground hover:text-foreground"
                >
                  clear
                </button>
              ) : null}
            </div>
          ) : null}
        </div>
      ) : null}

      {totalCount > 3 ? <SidebarSearch value={searchQuery} onChange={setSearchQuery} /> : null}

      {error ? (
        <div className="border-b px-3 py-2">
          <div className="flex items-center justify-between gap-2 rounded-md border border-destructive/20 bg-destructive/5 px-2.5 py-1.5 text-xs text-destructive">
            <span className="flex-1 truncate">{error}</span>
            <div className="flex shrink-0 items-center gap-1.5">
              <button type="button" onClick={onRetryLoad} className="underline">
                Retry
              </button>
              <button
                type="button"
                onClick={onDismissError}
                className="p-0.5 text-destructive/60 hover:text-destructive"
                aria-label="Dismiss"
              >
                <XIcon className="size-3.5" />
              </button>
            </div>
          </div>
        </div>
      ) : null}

      <div className="scroll-container flex-1 space-y-0.5 overflow-y-auto p-2">
        {isLoading && displayedCases.length === 0 ? (
          <div className="space-y-0.5 px-1 pt-1">
            {Array.from({ length: 8 }, (_, index) => (
              <div
                key={`skeleton-${index}`}
                className="flex items-center gap-2 px-3 py-1.5"
                style={{ animationDelay: `${index * 50}ms` }}
              >
                <div className="size-2 animate-pulse rounded-full bg-muted" />
                <div className="h-3 w-5 animate-pulse rounded bg-muted/60" />
                <div
                  className="h-3.5 max-w-full flex-1 animate-pulse rounded bg-muted"
                  style={{ maxWidth: `${80 - index * 6}%` }}
                />
              </div>
            ))}
          </div>
        ) : displayedCases.length === 0 ? (
          <div className="px-4 py-8 text-center">
            <p className="text-sm text-muted-foreground">
              {searchQuery && filterStatus !== "all"
                ? `No ${filterStatus} test cases matching "${searchQuery}"`
                : searchQuery
                  ? `No test cases matching "${searchQuery}"`
                  : filterStatus === "all"
                    ? "No test cases yet"
                    : `No ${filterStatus} test cases`}
            </p>
            {!searchQuery && filterStatus === "all" ? (
              <button
                type="button"
                onClick={onCreateTestCase}
                className="mt-2 text-xs text-primary hover:underline"
              >
                Create your first test case
              </button>
            ) : null}
          </div>
        ) : (
          displayedCases.map((testCase) =>
            multiSelect ? (
              <label
                key={testCase.id}
                className={cx(
                  "flex cursor-pointer items-center gap-2 rounded-md px-3 py-1.5 transition-colors",
                  selectedIds.has(testCase.id) ? "bg-primary/10" : "hover:bg-muted"
                )}
              >
                <input
                  type="checkbox"
                  checked={selectedIds.has(testCase.id)}
                  onChange={() => {
                    setSelectedIds((previous) => {
                      const next = new Set(previous);
                      if (next.has(testCase.id)) {
                        next.delete(testCase.id);
                      } else {
                        next.add(testCase.id);
                      }
                      return next;
                    });
                  }}
                  className="rounded border-border accent-primary"
                />
                <StatusDot status={testCase.status} />
                <span className="flex-1 truncate text-sm">
                  {formatText(testCase.name || testCase.input || "Untitled")}
                </span>
              </label>
            ) : (
              <div
                key={testCase.id}
                role="button"
                tabIndex={0}
                onClick={() => onOpenTestCase(testCase.id)}
                onKeyDown={(event) => handleCaseKeyDown(event, testCase.id)}
                className={cx(
                  "group flex items-center gap-2 rounded-md px-3 py-1.5 transition-colors",
                  testCase.id === selectedTestCaseId
                    ? "bg-primary/10 text-primary"
                    : "text-foreground hover:bg-muted"
                )}
              >
                <StatusDot status={testCase.status} />
                {extractTestCaseLevel(testCase.id) ? (
                  <span className="shrink-0 text-[10px] font-mono tabular-nums text-muted-foreground/70">
                    {extractTestCaseLevel(testCase.id)}
                  </span>
                ) : null}
                <span className="flex-1 truncate text-sm">
                  {formatText(testCase.name || testCase.input || "Untitled")}
                </span>
                {(testCase.expectedTrajectory?.length ?? 0) > 0 ? (
                  <span className="shrink-0 text-[10px] font-mono tabular-nums text-muted-foreground/50">
                    {testCase.expectedTrajectory?.length ?? 0}
                  </span>
                ) : null}
                {testCase.structuredGroundTruth ? (
                  <BookCheckIcon
                    className="size-3 shrink-0 text-[var(--dot-emerald)]/60"
                    aria-label="Has structured ground truth"
                  />
                ) : null}
                {onDeleteTestCase ? (
                  <button
                    type="button"
                    onClick={(event) => {
                      event.preventDefault();
                      event.stopPropagation();
                      void onDeleteTestCase(testCase.id);
                    }}
                    className="shrink-0 rounded p-0.5 opacity-0 transition-all hover:bg-destructive/10 hover:text-destructive group-hover:opacity-100"
                    aria-label="Delete test case"
                  >
                    <TrashIcon className="size-3" />
                  </button>
                ) : null}
              </div>
            )
          )
        )}
      </div>

      {multiSelect && selectedIds.size > 0 ? (
        <div className="flex items-center gap-1.5 border-t bg-muted/30 px-3 py-2">
          <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">
            {selectedIds.size} selected
          </span>
          <div className="flex-1" />
          {bulkActions.map((action) => (
            <BulkActionButton
              key={action.key}
              disabled={batchBusy}
              destructive={action.destructive}
              label={
                typeof action.label === "function" ? action.label(selectedIds.size) : action.label
              }
              onClick={() => {
                void (async () => {
                  setBatchBusy(true);
                  const result = await action.onExecute({
                    selectedIds: [...selectedIds],
                    selectedCases,
                  });
                  if (result?.nextSelectedIds) {
                    setSelectedIds(new Set(result.nextSelectedIds));
                  } else if (result?.clearSelection) {
                    setSelectedIds(new Set());
                  }
                  setBatchBusy(false);
                })();
              }}
            />
          ))}
        </div>
      ) : null}

      {footer ? <div className="border-t p-3">{footer}</div> : null}
    </div>
  );
}
