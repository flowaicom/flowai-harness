import { FlaskConicalIcon, PlusIcon, RotateCcwIcon, SearchIcon, TrashIcon } from "lucide-react";
import { type ReactNode, useMemo, useState } from "react";
import { Link } from "react-router";
import {
  type EvalSidebarRunLike,
  type EvalSidebarStatusFilter,
  filterEvalSidebarRuns,
  getEvalModeLabel,
  getShortEvalModelLabel,
} from "./eval-sidebar-model";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

type TimePeriod = "today" | "yesterday" | "thisWeek" | "thisMonth" | "older";

const TIME_PERIOD_ORDER: readonly TimePeriod[] = [
  "today",
  "yesterday",
  "thisWeek",
  "thisMonth",
  "older",
];

const TIME_PERIOD_LABELS: Record<TimePeriod, string> = {
  today: "Today",
  yesterday: "Yesterday",
  thisWeek: "This week",
  thisMonth: "This month",
  older: "Older",
};

const STATUS_FILTERS: readonly {
  readonly value: EvalSidebarStatusFilter;
  readonly label: string;
}[] = [
  { value: "all", label: "All" },
  { value: "running", label: "Running" },
  { value: "passed", label: "Passed" },
  { value: "failed", label: "Failed" },
];

function getTimePeriod(date: string | Date | undefined | null): TimePeriod {
  if (!date) return "older";
  const parsed = typeof date === "string" ? new Date(date) : date;
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const yesterday = new Date(today.getTime() - 86_400_000);
  const weekAgo = new Date(today.getTime() - 7 * 86_400_000);
  const monthAgo = new Date(today.getTime() - 30 * 86_400_000);

  if (parsed >= today) return "today";
  if (parsed >= yesterday) return "yesterday";
  if (parsed >= weekAgo) return "thisWeek";
  if (parsed >= monthAgo) return "thisMonth";
  return "older";
}

function groupByTimePeriod<TRun extends EvalSidebarRunLike>(runs: readonly TRun[]) {
  const groups = new Map<TimePeriod, TRun[]>();

  for (const run of runs) {
    const period = getTimePeriod(run.createdAt);
    const items = groups.get(period);
    if (items) items.push(run);
    else groups.set(period, [run]);
  }

  return TIME_PERIOD_ORDER.flatMap((period) => {
    const items = groups.get(period);
    return items ? [{ period, label: TIME_PERIOD_LABELS[period], items }] : [];
  });
}

function compactRelativeTime(date: string | Date | undefined | null): string {
  if (!date) return "";
  const parsed = typeof date === "string" ? new Date(date) : date;
  const now = new Date();
  const diff = now.getTime() - parsed.getTime();
  const minutes = Math.floor(diff / 60_000);
  const hours = Math.floor(diff / 3_600_000);
  const days = Math.floor(diff / 86_400_000);

  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m`;
  if (hours < 24) return `${hours}h`;
  if (days < 7) return `${days}d`;
  return parsed.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

interface RunItemProps<TRun extends EvalSidebarRunLike> {
  readonly run: TRun;
  readonly isSelected: boolean;
  readonly pulse: boolean;
  readonly onDelete?: (id: string) => void;
  readonly renderStatusDot: (input: { readonly run: TRun; readonly pulse: boolean }) => ReactNode;
}

function RunItem<TRun extends EvalSidebarRunLike>({
  run,
  isSelected,
  pulse,
  onDelete,
  renderStatusDot,
}: RunItemProps<TRun>) {
  const modelLabel = getShortEvalModelLabel(run.config.model);
  const isCompleted = run.status.status === "completed";
  const score = isCompleted ? run.status.summary.aggregateScore : null;
  const isPassing = score !== null && score >= run.config.passThreshold;

  return (
    <Link
      to={`/evals/${run.id}`}
      className={cx(
        "group flex items-center gap-2 rounded-md px-3 py-1.5 transition-colors",
        isSelected ? "bg-primary/10 text-primary" : "text-foreground hover:bg-muted"
      )}
    >
      {renderStatusDot({ run, pulse })}
      {run.parentRunId ? (
        <RotateCcwIcon
          className="size-3 shrink-0 text-muted-foreground/50"
          aria-label="Re-run of a previous eval"
        />
      ) : null}
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="truncate text-sm">{getEvalModeLabel(run.config.mode)}</span>
          {modelLabel ? (
            <span className="shrink-0 rounded bg-muted px-1 py-px text-[9px] text-muted-foreground">
              {modelLabel}
            </span>
          ) : null}
        </div>
      </div>
      {run.resultCount > 0 ? (
        <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground/40">
          {run.resultCount}
        </span>
      ) : null}
      {score !== null ? (
        <span
          className={cx(
            "shrink-0 text-[10px] font-mono tabular-nums",
            isPassing ? "text-[var(--dot-emerald)]" : "text-[var(--dot-red)]"
          )}
        >
          {Math.round(score * 100)}%
        </span>
      ) : null}
      <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground/70 group-hover:hidden">
        {compactRelativeTime(run.createdAt)}
      </span>
      {onDelete ? (
        <button
          type="button"
          onClick={(event) => {
            event.preventDefault();
            event.stopPropagation();
            onDelete(run.id);
          }}
          className="hidden shrink-0 rounded p-0.5 transition-all hover:bg-destructive/10 hover:text-destructive group-hover:block"
          aria-label="Delete eval run"
        >
          <TrashIcon className="size-3" />
        </button>
      ) : null}
    </Link>
  );
}

export interface SharedEvalSidebarProps<TRun extends EvalSidebarRunLike> {
  readonly runs: readonly TRun[];
  readonly activeRunId?: string | null;
  readonly isLoading: boolean;
  readonly nav?: ReactNode;
  readonly footer?: ReactNode;
  readonly renderStatusDot: (input: { readonly run: TRun; readonly pulse: boolean }) => ReactNode;
  readonly shouldPulseRun?: (run: TRun) => boolean;
  readonly onNewEval: () => void;
  readonly onDeleteRun?: (id: string) => void;
}

export function SharedEvalSidebar<TRun extends EvalSidebarRunLike>({
  runs,
  activeRunId,
  isLoading,
  nav,
  footer,
  renderStatusDot,
  shouldPulseRun,
  onNewEval,
  onDeleteRun,
}: SharedEvalSidebarProps<TRun>) {
  const [search, setSearch] = useState("");
  const [statusFilter, setStatusFilter] = useState<EvalSidebarStatusFilter>("all");

  const filteredRuns = useMemo(
    () => filterEvalSidebarRuns(runs, search, statusFilter),
    [runs, search, statusFilter]
  );
  const groups = useMemo(() => groupByTimePeriod(filteredRuns), [filteredRuns]);
  const showFilters = runs.length > 3;

  return (
    <div className="flex h-full flex-col">
      {nav}

      <div className="flex items-center justify-between border-b px-4 py-3">
        <h2 className="text-sm font-semibold text-foreground">
          Evals
          {runs.length > 0 ? (
            <span className="ml-1.5 font-normal tabular-nums text-muted-foreground">
              {runs.length}
            </span>
          ) : null}
        </h2>
        <button
          type="button"
          onClick={onNewEval}
          className="rounded-md p-1.5 transition-colors hover:bg-muted"
          aria-label="New eval"
        >
          <PlusIcon className="size-4" />
        </button>
      </div>

      {showFilters ? (
        <div className="space-y-2 border-b px-3 py-2">
          <div className="relative">
            <SearchIcon className="absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
            <input
              type="text"
              placeholder="Search by mode, model, ID..."
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              className="w-full rounded-md bg-muted/50 py-1.5 pl-7 pr-2 text-xs placeholder:text-muted-foreground/50 focus:outline-none focus:ring-1 focus:ring-ring"
            />
          </div>
          <div className="flex gap-1">
            {STATUS_FILTERS.map((filter) => (
              <button
                key={filter.value}
                type="button"
                onClick={() => setStatusFilter(filter.value)}
                className={cx(
                  "rounded-full px-2 py-0.5 text-[10px] transition-colors",
                  statusFilter === filter.value
                    ? "bg-primary text-primary-foreground"
                    : "bg-muted/60 text-muted-foreground hover:bg-muted"
                )}
              >
                {filter.label}
              </button>
            ))}
          </div>
        </div>
      ) : null}

      <div className="flex-1 overflow-y-auto p-2 space-y-0.5">
        {isLoading && runs.length === 0 ? (
          <div className="space-y-0.5 px-1 pt-1">
            {Array.from({ length: 5 }, (_, index) => (
              <div
                key={`eval-sidebar-skeleton-${index}`}
                className="flex items-center gap-2 px-3 py-1.5"
                style={{ animationDelay: `${index * 50}ms` }}
              >
                <div className="size-2 rounded-full bg-muted animate-pulse" />
                <div
                  className="h-3.5 flex-1 rounded bg-muted animate-pulse"
                  style={{ maxWidth: `${70 - index * 8}%` }}
                />
                <div className="h-3 w-8 rounded bg-muted/60 animate-pulse" />
                <div className="h-3 w-6 rounded bg-muted/40 animate-pulse" />
              </div>
            ))}
          </div>
        ) : runs.length === 0 ? (
          <div className="px-4 py-8 text-center">
            <FlaskConicalIcon className="mx-auto mb-2 size-8 text-muted-foreground/30" />
            <p className="text-sm text-muted-foreground">No eval runs yet</p>
            <button
              type="button"
              onClick={onNewEval}
              className="mt-2 text-xs text-primary hover:underline"
            >
              Create your first eval
            </button>
          </div>
        ) : filteredRuns.length === 0 ? (
          <div className="px-4 py-6 text-center">
            <p className="text-xs text-muted-foreground">No runs match filters</p>
          </div>
        ) : (
          groups.map((group) => (
            <div key={group.period}>
              <div className="px-3 pb-1 pt-2 first:pt-0">
                <span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground/60">
                  {group.label}
                </span>
              </div>
              {group.items.map((run) => (
                <RunItem
                  key={run.id}
                  run={run}
                  isSelected={run.id === activeRunId}
                  pulse={shouldPulseRun?.(run) ?? run.status.status === "running"}
                  onDelete={onDeleteRun}
                  renderStatusDot={renderStatusDot}
                />
              ))}
            </div>
          ))
        )}
      </div>

      {footer ? <div className="flex justify-end border-t p-3">{footer}</div> : null}
    </div>
  );
}
