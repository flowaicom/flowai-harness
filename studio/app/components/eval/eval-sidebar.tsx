/**
 * Eval sidebar component.
 *
 * Displays eval runs grouped by time period with status indicators.
 *
 * Improvements:
 * - Time-period grouping (Today / Yesterday / This week / Older)
 * - Compact relative time ("2m", "3h", "1d")
 * - Loading skeleton
 * - Delete confirmation
 * - Empty state with icon
 *
 * @module components/eval/eval-sidebar
 */

import { FlaskConicalIcon, PlusIcon, RotateCcwIcon, SearchIcon, TrashIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useNavigate } from "react-router";
import { ConfirmDialog } from "~/components/shared/confirm-dialog";
import { useLifecycleEvent } from "~/hooks/use-lifecycle-event";
import { deleteEvalRun, listEvalRuns, listTestCaseSets } from "~/lib/api";
import type { EvalMode, EvalRunSummary } from "~/lib/domain/eval";
import { isOk } from "~/lib/domain/result";
import {
  selectActiveWorkspaceId,
  selectEvalActiveRunId,
  selectEvalRuns,
  selectEvalSessions,
  useEvaluation,
  useEvaluationActions,
  useWorkspace,
} from "~/lib/stores";
import { cn, compactRelativeTime, groupByTimePeriod } from "~/lib/utils";
import { EvalStatusDot } from "./eval-status-dot";

// ============================================================================
// Status Filter
// ============================================================================

type StatusFilter = "all" | "running" | "passed" | "failed";

const STATUS_FILTERS: readonly { readonly value: StatusFilter; readonly label: string }[] = [
  { value: "all", label: "All" },
  { value: "running", label: "Running" },
  { value: "passed", label: "Passed" },
  { value: "failed", label: "Failed" },
];

function matchesStatusFilter(run: EvalRunSummary, filter: StatusFilter): boolean {
  if (filter === "all") return true;
  if (filter === "running")
    return run.status.status === "running" || run.status.status === "paused";
  if (filter === "passed")
    return (
      run.status.status === "completed" &&
      run.status.summary.aggregateScore >= run.config.passThreshold
    );
  // "failed" = completed with low score, or explicitly failed/cancelled
  return (
    run.status.status === "failed" ||
    run.status.status === "cancelled" ||
    (run.status.status === "completed" &&
      run.status.summary.aggregateScore < run.config.passThreshold)
  );
}

// ============================================================================
// Constants
// ============================================================================

/** Capitalize a mode string for display, e.g. "sequential" → "Sequential". */
function modeLabel(mode: EvalMode): string {
  if (!mode) return "Unknown";
  return mode.charAt(0).toUpperCase() + mode.slice(1);
}

// ============================================================================
// Run Item
// ============================================================================

interface RunItemProps {
  run: EvalRunSummary;
  isSelected: boolean;
  hasActiveSession?: boolean;
  onDelete?: (id: string) => void;
}

/** Derive a short model label: "opus" from "claude-opus-4-6", etc. */
function shortModelLabel(model: string | null): string | null {
  if (!model) return null;
  // Extract the meaningful part from model IDs
  const m = model.toLowerCase();
  if (m.includes("opus")) return "opus";
  if (m.includes("sonnet")) return "sonnet";
  if (m.includes("haiku")) return "haiku";
  if (m.includes("gpt-4o")) return "4o";
  if (m.includes("gpt-4")) return "gpt4";
  if (m.includes("glm")) return "glm";
  // Fallback: last segment before any date suffix
  const parts = model.split("-");
  return parts.length > 1 ? parts.slice(0, 2).join("-") : model.slice(0, 12);
}

function RunItem({ run, isSelected, hasActiveSession, onDelete }: RunItemProps) {
  const modelLabel = shortModelLabel(run.config.model);
  const isCompleted = run.status.status === "completed";
  const score = isCompleted ? run.status.summary.aggregateScore : null;
  const isPassing = score !== null && score >= run.config.passThreshold;

  return (
    <Link
      to={`/evals/${run.id}`}
      prefetch="intent"
      className={cn(
        "group flex items-center gap-2 px-3 py-1.5 rounded-md transition-colors",
        isSelected ? "bg-primary/10 text-primary" : "hover:bg-muted text-foreground"
      )}
    >
      <EvalStatusDot
        status={run.status.status}
        pulse={!!hasActiveSession || run.status.status === "running"}
      />
      {run.parentRunId && (
        <RotateCcwIcon
          className="size-3 text-muted-foreground/50 shrink-0"
          aria-label="Re-run of a previous eval"
        />
      )}
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-1.5">
          <span className="text-sm truncate">{modeLabel(run.config.mode)}</span>
          {modelLabel && (
            <span className="text-[9px] px-1 py-px rounded bg-muted text-muted-foreground shrink-0">
              {modelLabel}
            </span>
          )}
        </div>
      </div>
      {run.resultCount > 0 && (
        <span className="text-[10px] tabular-nums text-muted-foreground/40 shrink-0">
          {run.resultCount}
        </span>
      )}
      {score !== null && (
        <span
          className={cn(
            "text-[10px] font-mono tabular-nums shrink-0",
            isPassing ? "text-[var(--dot-emerald)]" : "text-[var(--dot-red)]"
          )}
        >
          {Math.round(score * 100)}%
        </span>
      )}
      <span className="text-[10px] text-muted-foreground/70 tabular-nums shrink-0 group-hover:hidden">
        {compactRelativeTime(run.createdAt)}
      </span>
      {onDelete && (
        <button
          type="button"
          onClick={(e) => {
            e.preventDefault();
            e.stopPropagation();
            onDelete(run.id);
          }}
          className="hidden group-hover:block p-0.5 rounded hover:bg-destructive/10 hover:text-destructive transition-all shrink-0 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none focus-visible:block"
          aria-label="Delete eval run"
        >
          <TrashIcon className="size-3" />
        </button>
      )}
    </Link>
  );
}

// ============================================================================
// Main Component
// ============================================================================

export function EvalSidebar() {
  const navigate = useNavigate();
  const runs = useEvaluation(selectEvalRuns);
  const activeRunId = useEvaluation(selectEvalActiveRunId);
  const evalSessions = useEvaluation(selectEvalSessions);
  const { setRuns, setTestCaseSets, removeRun, selectRun } = useEvaluationActions();

  const activeWorkspaceId = useWorkspace(selectActiveWorkspaceId);
  const [isLoading, setIsLoading] = useState(true);
  const [search, setSearch] = useState("");
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [pendingDeleteRunId, setPendingDeleteRunId] = useState<string | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);

  // Reusable loader for runs + test case sets
  const load = useCallback(async () => {
    setIsLoading(true);
    const [runsResult, setsResult] = await Promise.all([listEvalRuns(), listTestCaseSets()]);
    if (isOk(runsResult)) setRuns(runsResult.value);
    if (isOk(setsResult)) setTestCaseSets(setsResult.value);
    setIsLoading(false);
  }, [setRuns, setTestCaseSets]);

  // Load on mount and when workspace changes
  // biome-ignore lint/correctness/useExhaustiveDependencies: reload eval data when workspace changes
  useEffect(() => {
    load();
  }, [load, activeWorkspaceId]);

  // Auto-refresh when an eval completes (e.g. background run finishes)
  useLifecycleEvent("evalCompleted", () => {
    load();
  });

  // Auto-refresh test case sets when new test cases are created
  useLifecycleEvent("testCaseCreated", () => {
    load();
  });

  const handleNewEval = useCallback(() => {
    selectRun(null);
    navigate("/evals/new");
  }, [selectRun, navigate]);

  const handleDeleteRun = useCallback((id: string) => {
    setPendingDeleteRunId(id);
  }, []);

  const confirmDeleteRun = useCallback(async () => {
    if (!pendingDeleteRunId || deleteBusy) return;
    const id = pendingDeleteRunId;
    setDeleteBusy(true);
    const result = await deleteEvalRun(id);
    setDeleteBusy(false);
    setPendingDeleteRunId(null);
    if (isOk(result)) {
      removeRun(id);
      if (id === activeRunId) {
        navigate("/evals");
      }
    }
  }, [pendingDeleteRunId, deleteBusy, removeRun, activeRunId, navigate]);

  // Filter runs by search + status
  const filteredRuns = useMemo(() => {
    const q = search.trim().toLowerCase();
    return runs.filter((r) => {
      if (!matchesStatusFilter(r, statusFilter)) return false;
      if (!q) return true;
      return (
        r.id.toLowerCase().includes(q) ||
        r.config.mode.toLowerCase().includes(q) ||
        (r.config.model ?? "").toLowerCase().includes(q)
      );
    });
  }, [runs, search, statusFilter]);

  // Group by time period
  const groups = useMemo(() => groupByTimePeriod(filteredRuns, (r) => r.createdAt), [filteredRuns]);

  const showFilters = runs.length > 3;

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="px-4 py-3 border-b flex items-center justify-between">
        <h2 className="font-semibold text-sm text-foreground">
          Evals
          {runs.length > 0 && (
            <span className="text-muted-foreground font-normal ml-1.5 tabular-nums">
              {runs.length}
            </span>
          )}
        </h2>
        <button
          type="button"
          onClick={handleNewEval}
          className="p-1.5 rounded-md hover:bg-muted transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          aria-label="New eval"
        >
          <PlusIcon className="size-4" />
        </button>
      </div>

      {/* Search + Filter */}
      {showFilters && (
        <div className="px-3 py-2 border-b space-y-2">
          <div className="relative">
            <SearchIcon className="absolute left-2 top-1/2 -translate-y-1/2 size-3.5 text-muted-foreground" />
            <input
              type="text"
              placeholder="Search by mode, model, ID..."
              aria-label="Search eval runs"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="w-full text-xs bg-muted/50 rounded-md pl-7 pr-2 py-1.5 placeholder:text-muted-foreground/50 focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none"
            />
          </div>
          <div className="flex gap-1">
            {STATUS_FILTERS.map((f) => (
              <button
                key={f.value}
                type="button"
                onClick={() => setStatusFilter(f.value)}
                className={cn(
                  "text-[10px] px-2 py-0.5 rounded-full transition-colors",
                  statusFilter === f.value
                    ? "bg-primary text-primary-foreground"
                    : "bg-muted/60 text-muted-foreground hover:bg-muted"
                )}
              >
                {f.label}
              </button>
            ))}
          </div>
        </div>
      )}

      {/* Run List */}
      <div className="flex-1 overflow-y-auto scroll-container p-2 space-y-0.5">
        {isLoading && runs.length === 0 ? (
          <div className="space-y-0.5 px-1 pt-1">
            {Array.from({ length: 5 }, (_, i) => (
              <div
                // biome-ignore lint/suspicious/noArrayIndexKey: static skeleton
                key={i}
                className="flex items-center gap-2 px-3 py-1.5"
                style={{ animationDelay: `${i * 50}ms` }}
              >
                <div className="size-2 rounded-full bg-muted animate-pulse" />
                <div
                  className="h-3.5 bg-muted rounded animate-pulse flex-1"
                  style={{ maxWidth: `${70 - i * 8}%` }}
                />
                <div className="w-8 h-3 bg-muted/60 rounded animate-pulse" />
                <div className="w-6 h-3 bg-muted/40 rounded animate-pulse" />
              </div>
            ))}
          </div>
        ) : runs.length === 0 ? (
          <div className="text-center py-8 px-4">
            <FlaskConicalIcon className="size-8 mx-auto mb-2 text-muted-foreground/30" />
            <p className="text-sm text-muted-foreground">No eval runs yet</p>
            <button
              type="button"
              onClick={handleNewEval}
              className="mt-2 text-xs text-primary hover:underline rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
            >
              Create your first eval
            </button>
          </div>
        ) : filteredRuns.length === 0 ? (
          <div className="text-center py-6 px-4">
            <p className="text-xs text-muted-foreground">No runs match filters</p>
          </div>
        ) : (
          groups.map((group) => (
            <div key={group.period}>
              <div className="px-3 pt-2 pb-1 first:pt-0">
                <span className="text-[10px] font-medium text-muted-foreground/60 uppercase tracking-wider">
                  {group.label}
                </span>
              </div>
              {group.items.map((run) => (
                <RunItem
                  key={run.id}
                  run={run}
                  isSelected={run.id === activeRunId}
                  hasActiveSession={evalSessions.has(run.id)}
                  onDelete={handleDeleteRun}
                />
              ))}
            </div>
          ))
        )}
      </div>

      <ConfirmDialog
        open={pendingDeleteRunId !== null}
        title="Delete eval run"
        description="This removes the eval run and its saved artifact from the local Studio store. This cannot be undone."
        confirmLabel={deleteBusy ? "Deleting..." : "Delete"}
        tone="danger"
        onConfirm={confirmDeleteRun}
        onCancel={() => {
          if (!deleteBusy) setPendingDeleteRunId(null);
        }}
      />
    </div>
  );
}
