import type { LucideIcon } from "lucide-react";
import {
  AlertTriangleIcon,
  BarChart3Icon,
  ChevronDownIcon,
  CopyIcon,
  ExternalLinkIcon,
  InfoIcon,
  MessageSquareIcon,
  PenToolIcon,
  PlayIcon,
  SaveIcon,
  TrashIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import type { SharedTestCaseStatus } from "./domain";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

export interface SharedValidationIssueLike {
  readonly severity: "error" | "warning" | "info" | string;
  readonly message: string;
}

export interface SharedToolCallEntryLike {
  readonly invocationId: string;
  readonly index: number;
  readonly toolName: string;
  readonly args: unknown;
}

export interface SharedEvalRunSummaryLike {
  readonly id: string;
  readonly createdAt: string;
  readonly config: {
    readonly mode: string;
    readonly model?: string | null;
  };
  readonly status: {
    readonly status: string;
  };
}

export interface SharedTestCaseDetailLike {
  readonly createdAt: string;
  readonly updatedAt: string;
  readonly tags: readonly string[];
  readonly sourceSessionId?: string | null;
  readonly sourceThreadId?: string | null;
}

export interface SharedTestDetailStatusTransition<TStatus extends string = SharedTestCaseStatus> {
  readonly to: TStatus;
  readonly label: string;
  readonly icon: LucideIcon;
}

export interface SharedTestCaseDetailPageProps<
  TStatus extends string = SharedTestCaseStatus,
  TCase extends SharedTestCaseDetailLike = SharedTestCaseDetailLike,
> {
  readonly loaded: boolean;
  readonly loadingError?: string | null;
  readonly containerClassName?: string;
  readonly testCaseId?: string | null;
  readonly testCase: TCase | null;
  readonly status: TStatus;
  readonly isDirty: boolean;
  readonly saving: boolean;
  readonly deleting: boolean;
  readonly error?: string | null;
  readonly trajectoryStepCount: number;
  readonly validationIssues: readonly SharedValidationIssueLike[];
  readonly traceExpanded: boolean;
  readonly trace: readonly SharedToolCallEntryLike[] | null;
  readonly evalHistory: readonly SharedEvalRunSummaryLike[];
  readonly activeTestCaseCount: number;
  readonly transitions: readonly SharedTestDetailStatusTransition<TStatus>[];
  readonly form: ReactNode;
  readonly copyIdControl?: ReactNode;
  readonly formatText?: (value: string) => string;
  readonly renderErrorBanner?: (message: string, onDismiss: () => void) => ReactNode;
  readonly getStatusBadgeClassName?: (status: TStatus) => string | undefined;
  readonly onDismissError: () => void;
  readonly onToggleTraceExpanded: () => void;
  readonly onSave: () => void;
  readonly onTryInChat: () => void;
  readonly onRunEval: (allActive: boolean) => void;
  readonly onRefineInBuilder: () => void;
  readonly onClone: () => void;
  readonly onStatusChange: (status: TStatus) => void;
  readonly onDelete: () => void;
  readonly onOpenSourceThread: () => void;
  readonly onOpenEvalRun: (runId: string) => void;
  readonly onOpenMoreEvalRuns: () => void;
}

function StatusBadge<TStatus extends string>({
  status,
  getStatusBadgeClassName,
}: {
  readonly status: TStatus;
  readonly getStatusBadgeClassName?: (status: TStatus) => string | undefined;
}) {
  return (
    <span
      className={cx(
        "rounded-full px-2 py-0.5 text-[10px] font-medium uppercase tracking-wider",
        getStatusBadgeClassName?.(status)
      )}
    >
      {status}
    </span>
  );
}

function renderLoadingState(error?: string | null) {
  return (
    <div className="max-w-2xl mx-auto p-8 space-y-4">
      {error ? (
        <div className="rounded-md accent-bar-red bg-[var(--accent-red)] px-4 py-3 text-[var(--dot-red)] text-sm">
          {error}
        </div>
      ) : (
        <>
          <div className="h-8 w-64 rounded bg-muted animate-shimmer" />
          <div className="h-32 rounded-lg bg-muted animate-shimmer" />
          <div className="h-40 rounded-lg bg-muted animate-shimmer" />
          <div className="h-24 rounded-lg bg-muted animate-shimmer" />
        </>
      )}
    </div>
  );
}

function renderTraceArgs(args: unknown, formatText: (value: string) => string) {
  if (typeof args === "string") return formatText(args);
  return formatText(JSON.stringify(args, null, 2));
}

function statusDotClass(status: string) {
  switch (status) {
    case "completed":
      return "bg-[var(--dot-emerald)]";
    case "running":
      return "bg-[var(--dot-blue)] animate-pulse";
    case "failed":
      return "bg-[var(--dot-red)]";
    case "cancelled":
      return "bg-muted-foreground";
    default:
      return "bg-muted-foreground";
  }
}

export function SharedTestCaseDetailPage<
  TStatus extends string = SharedTestCaseStatus,
  TCase extends SharedTestCaseDetailLike = SharedTestCaseDetailLike,
>({
  loaded,
  loadingError,
  containerClassName,
  testCaseId,
  testCase,
  status,
  isDirty,
  saving,
  deleting,
  error,
  trajectoryStepCount,
  validationIssues,
  traceExpanded,
  trace,
  evalHistory,
  activeTestCaseCount,
  transitions,
  form,
  copyIdControl,
  formatText = (value) => value,
  renderErrorBanner,
  getStatusBadgeClassName,
  onDismissError,
  onToggleTraceExpanded,
  onSave,
  onTryInChat,
  onRunEval,
  onRefineInBuilder,
  onClone,
  onStatusChange,
  onDelete,
  onOpenSourceThread,
  onOpenEvalRun,
  onOpenMoreEvalRuns,
}: SharedTestCaseDetailPageProps<TStatus, TCase>) {
  if (!loaded) {
    return (
      <div className={cx("flex-1 overflow-y-auto", containerClassName)}>
        {renderLoadingState(loadingError)}
      </div>
    );
  }

  const showStatusBadge = status !== "draft";

  return (
    <div className={cx("flex-1 overflow-y-auto", containerClassName)}>
      <div className="max-w-2xl mx-auto p-8 space-y-6">
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              {showStatusBadge ? (
                <StatusBadge status={status} getStatusBadgeClassName={getStatusBadgeClassName} />
              ) : null}
              {isDirty ? (
                <span className="text-[10px] font-medium uppercase tracking-wider text-[var(--dot-amber)]">
                  Unsaved changes
                </span>
              ) : null}
            </div>
            <span className="flex items-center gap-1 text-xs font-mono text-muted-foreground">
              {formatText(testCaseId ?? "")}
              {copyIdControl}
            </span>
          </div>

          <div className="flex items-center gap-4 text-xs text-muted-foreground">
            {testCase ? (
              <>
                <span>
                  Created{" "}
                  {new Date(testCase.createdAt).toLocaleDateString(undefined, {
                    month: "short",
                    day: "numeric",
                    year: "numeric",
                  })}
                </span>
                {testCase.updatedAt !== testCase.createdAt ? (
                  <>
                    <span className="text-muted-foreground/30">|</span>
                    <span>
                      Updated{" "}
                      {new Date(testCase.updatedAt).toLocaleDateString(undefined, {
                        month: "short",
                        day: "numeric",
                        year: "numeric",
                      })}
                    </span>
                  </>
                ) : null}
                <span className="text-muted-foreground/30">|</span>
                <span>{trajectoryStepCount} trajectory steps</span>
                {testCase.tags.length > 0 ? (
                  <>
                    <span className="text-muted-foreground/30">|</span>
                    <span>{testCase.tags.map(formatText).join(", ")}</span>
                  </>
                ) : null}
                {testCase.sourceSessionId ? (
                  <>
                    <span className="text-muted-foreground/30">|</span>
                    <span>From builder</span>
                  </>
                ) : null}
                {testCase.sourceThreadId ? (
                  <>
                    <span className="text-muted-foreground/30">|</span>
                    <button
                      type="button"
                      onClick={onOpenSourceThread}
                      className="inline-flex items-center gap-1 text-primary hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      <ExternalLinkIcon className="size-3" />
                      Source thread
                    </button>
                  </>
                ) : null}
              </>
            ) : null}
          </div>
        </div>

        {error ? (
          renderErrorBanner ? (
            renderErrorBanner(error, onDismissError)
          ) : (
            <div className="rounded-md accent-bar-red bg-[var(--accent-red)] px-4 py-3 text-[var(--dot-red)] text-sm">
              {error}
            </div>
          )
        ) : null}

        {validationIssues.length > 0 ? (
          <div className="space-y-1.5">
            {validationIssues.map((issue, index) => (
              <div
                key={`${issue.severity}-${index}`}
                className={cx(
                  "flex items-start gap-2 rounded-md px-3 py-2 text-xs",
                  issue.severity === "error" &&
                    "accent-bar-red bg-[var(--accent-red)] text-[var(--dot-red)]",
                  issue.severity === "warning" &&
                    "accent-bar-amber bg-[var(--accent-amber)] text-[var(--dot-amber)]",
                  issue.severity === "info" &&
                    "accent-bar-blue bg-[var(--accent-blue)] text-[var(--dot-blue)]"
                )}
              >
                {issue.severity === "info" ? (
                  <InfoIcon className="size-3.5 shrink-0 mt-0.5" />
                ) : (
                  <AlertTriangleIcon className="size-3.5 shrink-0 mt-0.5" />
                )}
                <span>{issue.message}</span>
              </div>
            ))}
          </div>
        ) : null}

        {form}

        {testCase?.sourceThreadId ? (
          <div className="rounded-lg border border-border/50">
            <button
              type="button"
              onClick={onToggleTraceExpanded}
              aria-expanded={traceExpanded}
              aria-label={`${traceExpanded ? "Collapse" : "Expand"} source thread trace`}
              className="w-full flex items-center justify-between px-4 py-2.5 text-xs font-medium text-muted-foreground hover:text-foreground transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <span>Source Thread Trace</span>
              <ChevronDownIcon
                className={cx("size-3.5 transition-transform", traceExpanded && "rotate-180")}
              />
            </button>
            {traceExpanded ? (
              <div
                className="px-4 pb-3 pt-2 space-y-1.5"
                style={{ boxShadow: "inset 0 1px 0 var(--keyline)" }}
              >
                {trace === null ? (
                  <div className="space-y-2">
                    {Array.from({ length: 3 }, (_, index) => (
                      <div
                        key={`trace-skeleton-${index}`}
                        className="flex items-start gap-2"
                        style={{ animationDelay: `${index * 60}ms` }}
                      >
                        <div className="mt-0.5 h-3.5 w-4 shrink-0 rounded bg-muted animate-pulse" />
                        <div className="flex-1 space-y-1">
                          <div
                            className="h-3.5 rounded bg-muted animate-pulse"
                            style={{ width: `${70 - index * 12}%` }}
                          />
                        </div>
                      </div>
                    ))}
                  </div>
                ) : trace.length === 0 ? (
                  <div className="text-xs text-muted-foreground">
                    No tool calls in source thread
                  </div>
                ) : (
                  trace.map((entry) => (
                    <div key={entry.invocationId} className="flex items-start gap-2 text-xs">
                      <span className="pt-0.5 shrink-0 font-mono tabular-nums text-muted-foreground/50">
                        {entry.index + 1}.
                      </span>
                      <div className="min-w-0">
                        <span className="font-medium">{entry.toolName}</span>
                        {entry.args != null ? (
                          <pre className="mt-0.5 max-h-20 overflow-x-auto rounded bg-muted/50 px-2 py-1 text-[10px] text-muted-foreground">
                            {renderTraceArgs(entry.args, formatText)}
                          </pre>
                        ) : null}
                      </div>
                    </div>
                  ))
                )}
              </div>
            ) : null}
          </div>
        ) : null}

        {evalHistory.length > 0 ? (
          <div className="rounded-lg border border-border/50">
            <div className="flex items-center gap-2 px-4 py-2.5 text-xs font-medium text-muted-foreground">
              <BarChart3Icon className="size-3.5" />
              <span>Eval History</span>
              <span className="ml-auto tabular-nums">
                {evalHistory.length} run{evalHistory.length !== 1 ? "s" : ""}
              </span>
            </div>
            <div
              className="px-4 pb-3 space-y-1.5"
              style={{ boxShadow: "inset 0 1px 0 var(--keyline)" }}
            >
              {evalHistory.slice(0, 5).map((run) => (
                <button
                  key={run.id}
                  type="button"
                  onClick={() => onOpenEvalRun(run.id)}
                  className="w-full flex items-center gap-3 rounded-md px-2 py-1.5 text-left text-xs transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <span
                    className={cx(
                      "size-2 shrink-0 rounded-full",
                      statusDotClass(run.status.status)
                    )}
                  />
                  <span className="truncate font-mono text-muted-foreground">
                    {formatText(run.id.slice(0, 8))}
                  </span>
                  <span className="truncate text-muted-foreground/60">
                    {run.config.mode} &middot; {run.config.model ?? "default"}
                  </span>
                  <span className="ml-auto shrink-0 tabular-nums text-muted-foreground/40">
                    {new Date(run.createdAt).toLocaleDateString(undefined, {
                      month: "short",
                      day: "numeric",
                    })}
                  </span>
                </button>
              ))}
              {evalHistory.length > 5 ? (
                <button
                  type="button"
                  onClick={onOpenMoreEvalRuns}
                  className="w-full rounded py-1 text-center text-xs text-muted-foreground/60 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  +{evalHistory.length - 5} more runs
                </button>
              ) : null}
            </div>
          </div>
        ) : null}

        <div className="space-y-3 border-t pt-2">
          <div className="flex items-center gap-2 flex-wrap">
            <button
              type="button"
              onClick={onSave}
              disabled={saving || !isDirty}
              className={cx(
                "flex items-center gap-2 rounded-md px-4 py-1.5 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                isDirty
                  ? "bg-primary text-primary-foreground hover:bg-primary/90"
                  : "cursor-not-allowed bg-muted text-muted-foreground"
              )}
            >
              <SaveIcon className="size-3.5" />
              {saving ? "Saving..." : "Save"}
            </button>

            <div className="h-5 w-px bg-border" />

            <button
              type="button"
              onClick={onTryInChat}
              disabled={!testCase}
              className="flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-xs transition-colors hover:bg-muted disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              title="Open this prompt in a new chat thread"
            >
              <MessageSquareIcon className="size-3" />
              Try in Chat
            </button>

            <div className="flex items-center">
              <button
                type="button"
                onClick={() => onRunEval(false)}
                className={cx(
                  "flex items-center gap-1.5 border px-3 py-1.5 text-xs transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                  activeTestCaseCount > 1 ? "rounded-l-md" : "rounded-md"
                )}
                title="Run evaluation on this test case only"
              >
                <PlayIcon className="size-3" />
                Run Eval
              </button>
              {activeTestCaseCount > 1 ? (
                <button
                  type="button"
                  onClick={() => onRunEval(true)}
                  className="flex items-center gap-1 rounded-r-md border border-l-0 px-2 py-1.5 text-xs transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  title={`Run evaluation on all ${activeTestCaseCount} active test cases`}
                >
                  <ChevronDownIcon className="size-3" />
                  <span className="tabular-nums">{activeTestCaseCount}</span>
                </button>
              ) : null}
            </div>

            <button
              type="button"
              onClick={onRefineInBuilder}
              className="flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-xs transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              title="Open the builder agent to refine or create a related test case"
            >
              <PenToolIcon className="size-3" />
              Refine in Builder
            </button>

            <button
              type="button"
              onClick={onClone}
              disabled={!testCase}
              className="flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-xs transition-colors hover:bg-muted disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              title="Clone this test case as a new draft"
            >
              <CopyIcon className="size-3" />
              Clone
            </button>
          </div>

          <div className="flex items-center gap-2">
            {transitions.map((transition) => (
              <button
                key={transition.to}
                type="button"
                onClick={() => onStatusChange(transition.to)}
                disabled={saving}
                className="flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-xs transition-colors hover:bg-muted disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <transition.icon className="size-3" />
                {transition.label}
              </button>
            ))}

            <div className="flex-1" />

            <button
              type="button"
              onClick={onDelete}
              disabled={deleting}
              className="flex items-center gap-1.5 rounded-md border border-destructive/30 px-3 py-1.5 text-xs text-destructive transition-colors hover:bg-destructive/10 disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <TrashIcon className="size-3" />
              {deleting ? "Deleting..." : "Delete"}
            </button>
          </div>

          <div className="text-center text-[10px] text-muted-foreground/40">
            <kbd className="rounded border border-muted-foreground/20 px-1 py-0.5 text-[9px]">
              {navigator.platform?.includes("Mac") ? "Cmd" : "Ctrl"}+S
            </kbd>{" "}
            to save
          </div>
        </div>
      </div>
    </div>
  );
}
