import { CheckIcon, ChevronDownIcon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router";
import { listRuns, type RunSummary } from "~/lib/api/runs";
import { listThreads } from "~/lib/api/threads";
import { isOk } from "~/lib/domain/result";
import { isStaleRunningRun } from "~/lib/domain/run-events";
import { cn, formatDateTime, formatRelativeTime } from "~/lib/utils";

const STATUS_TONE: Record<string, string> = {
  completed: "text-[var(--dot-emerald)]/55",
  failed: "text-[var(--dot-red)]/65",
  cancelled: "text-[var(--dot-amber)]/55",
  running: "text-[var(--dot-blue)]/55",
  interrupted: "text-[var(--dot-amber)]/55",
};

const ALL_FILTER_VALUE = "__all";
const EMPTY_AGENT_VALUE = "__none";
const PAGE_SIZE_OPTIONS = [25, 50, 100] as const;

interface ThreadListRow {
  readonly id?: string;
  readonly threadId?: string;
  readonly title?: string | null;
}

function shortId(id: string): string {
  return id.length <= 12 ? id : `${id.slice(0, 8)}…${id.slice(-4)}`;
}

function normalizeSearchText(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]+/g, "");
}

function fuzzyMatch(haystack: string, query: string): boolean {
  const normalizedQuery = normalizeSearchText(query);
  if (!normalizedQuery) return true;
  const normalizedHaystack = normalizeSearchText(haystack);
  if (normalizedHaystack.includes(normalizedQuery)) return true;

  let queryIndex = 0;
  for (const char of normalizedHaystack) {
    if (char === normalizedQuery[queryIndex]) queryIndex += 1;
    if (queryIndex === normalizedQuery.length) return true;
  }
  return false;
}

function threadIdOf(thread: ThreadListRow): string | null {
  return thread.threadId ?? thread.id ?? null;
}

function displayStatus(run: RunSummary): {
  readonly label: string;
  readonly tone: string;
  readonly title?: string;
} {
  if (isStaleRunningRun(run)) {
    return {
      label: "interrupted",
      tone: STATUS_TONE.interrupted,
      title:
        "This run never wrote a terminal event and has not updated recently. It likely belongs to a previous server session.",
    };
  }
  return {
    label: run.status,
    tone: STATUS_TONE[run.status] ?? "text-muted-foreground",
  };
}

function contextSearchText(
  run: RunSummary,
  threadLookup: ReadonlyMap<string, ThreadListRow>
): string {
  const thread = run.threadId ? threadLookup.get(run.threadId) : undefined;
  return [run.runId, run.threadId, thread?.title, run.operation].filter(Boolean).join(" ");
}

function RunContext({
  run,
  threadLookup,
}: {
  readonly run: RunSummary;
  readonly threadLookup: ReadonlyMap<string, ThreadListRow>;
}) {
  if (run.operation === "chat" && run.threadId) {
    const thread = threadLookup.get(run.threadId);
    const title = thread?.title?.trim() || "Untitled chat";
    return (
      <div className="min-w-0">
        <Link
          to={`/chat/${run.threadId}`}
          className="block truncate text-foreground hover:text-primary hover:underline"
          title={`${title} · ${run.threadId}`}
        >
          {title}
        </Link>
        <div className="mt-0.5 font-mono text-[10px] text-muted-foreground">
          thread {shortId(run.threadId)}
        </div>
      </div>
    );
  }

  if (run.operation === "eval") {
    return (
      <div className="min-w-0">
        <Link
          to={`/evals/${run.runId}`}
          className="block truncate text-foreground hover:text-primary hover:underline"
          title={run.runId}
        >
          Eval run
        </Link>
        <div className="mt-0.5 font-mono text-[10px] text-muted-foreground">
          {shortId(run.runId)}
        </div>
      </div>
    );
  }

  return (
    <div className="font-mono text-[10px] text-muted-foreground">
      {run.threadId ? shortId(run.threadId) : "—"}
    </div>
  );
}

function RowsDropdown({
  value,
  onChange,
}: {
  readonly value: number;
  readonly onChange: (size: number) => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((prev) => !prev)}
        aria-haspopup="listbox"
        aria-expanded={open}
        className="flex h-9 min-w-24 items-center justify-between gap-2 rounded-lg border border-border/70 bg-background px-3 text-sm text-foreground shadow-sm transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <span className="font-mono tabular-nums">{value}</span>
        <ChevronDownIcon
          className={cn("size-4 text-muted-foreground transition-transform", open && "rotate-180")}
        />
      </button>
      {open && (
        <div className="absolute bottom-full right-0 z-50 mb-2 w-28 overflow-hidden rounded-lg border border-border bg-popover p-1 text-popover-foreground shadow-xl">
          {PAGE_SIZE_OPTIONS.map((size) => (
            <button
              key={size}
              type="button"
              role="option"
              aria-selected={size === value}
              onClick={() => {
                onChange(size);
                setOpen(false);
              }}
              className={cn(
                "flex w-full items-center justify-between rounded-md px-2 py-1.5 text-sm transition-colors",
                size === value
                  ? "bg-muted text-foreground"
                  : "text-muted-foreground hover:bg-muted/60 hover:text-foreground"
              )}
            >
              <span className="font-mono tabular-nums">{size}</span>
              {size === value && <CheckIcon className="size-3.5 text-primary" />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function PaginationControls({
  pageSize,
  onPageSizeChange,
  pageRangeStart,
  pageRangeEnd,
  filteredCount,
  totalCount,
  currentPage,
  totalPages,
  onPrevious,
  onNext,
}: {
  readonly pageSize: number;
  readonly onPageSizeChange: (size: number) => void;
  readonly pageRangeStart: number;
  readonly pageRangeEnd: number;
  readonly filteredCount: number;
  readonly totalCount: number;
  readonly currentPage: number;
  readonly totalPages: number;
  readonly onPrevious: () => void;
  readonly onNext: () => void;
}) {
  return (
    <div className="sticky bottom-0 z-20 flex items-center justify-between gap-3 border-t bg-background/95 px-6 py-3 text-xs text-muted-foreground backdrop-blur supports-[backdrop-filter]:bg-background/80">
      <span>
        Showing {pageRangeStart}-{pageRangeEnd} of {filteredCount} filtered runs
        {filteredCount !== totalCount ? ` (${totalCount} total)` : ""}
      </span>
      <div className="flex items-center gap-3">
        <label className="flex items-center gap-2">
          <span>Rows</span>
          <RowsDropdown value={pageSize} onChange={onPageSizeChange} />
        </label>
        <button
          type="button"
          onClick={onPrevious}
          disabled={currentPage <= 1}
          className="rounded-md border px-2 py-1 text-xs font-medium transition-colors hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
        >
          Previous
        </button>
        <span className="font-mono">
          Page {currentPage} / {totalPages}
        </span>
        <button
          type="button"
          onClick={onNext}
          disabled={currentPage >= totalPages}
          className="rounded-md border px-2 py-1 text-xs font-medium transition-colors hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
        >
          Next
        </button>
      </div>
    </div>
  );
}

/** Run list — one row per run across chat / eval / profile / knowledge. */
export default function RunsIndex() {
  const [runs, setRuns] = useState<readonly RunSummary[]>([]);
  const [threads, setThreads] = useState<readonly ThreadListRow[]>([]);
  const [contextQuery, setContextQuery] = useState("");
  const [operationFilter, setOperationFilter] = useState(ALL_FILTER_VALUE);
  const [statusFilter, setStatusFilter] = useState(ALL_FILTER_VALUE);
  const [agentFilter, setAgentFilter] = useState(ALL_FILTER_VALUE);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState<number>(PAGE_SIZE_OPTIONS[0]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    Promise.all([listRuns(), listThreads()]).then(([runsResult, threadsResult]) => {
      if (cancelled) return;
      if (isOk(runsResult)) {
        setRuns(runsResult.value);
        setError(null);
      } else {
        setError(runsResult.error.message);
      }
      if (isOk(threadsResult)) {
        setThreads(threadsResult.value as unknown as ThreadListRow[]);
      }
      setLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const threadLookup = useMemo(() => {
    const byId = new Map<string, ThreadListRow>();
    for (const thread of threads) {
      const id = threadIdOf(thread);
      if (id) byId.set(id, thread);
    }
    return byId;
  }, [threads]);

  const operationOptions = useMemo(
    () => Array.from(new Set(runs.map((run) => run.operation).filter(Boolean))).sort(),
    [runs]
  );
  const statusOptions = useMemo(
    () => Array.from(new Set(runs.map((run) => displayStatus(run).label).filter(Boolean))).sort(),
    [runs]
  );
  const agentOptions = useMemo(
    () => Array.from(new Set(runs.map((run) => run.agentId || EMPTY_AGENT_VALUE))).sort(),
    [runs]
  );
  const filteredRuns = useMemo(
    () =>
      runs.filter((run) => {
        const status = displayStatus(run).label;
        const agent = run.agentId || EMPTY_AGENT_VALUE;
        return (
          fuzzyMatch(contextSearchText(run, threadLookup), contextQuery) &&
          (operationFilter === ALL_FILTER_VALUE || run.operation === operationFilter) &&
          (statusFilter === ALL_FILTER_VALUE || status === statusFilter) &&
          (agentFilter === ALL_FILTER_VALUE || agent === agentFilter)
        );
      }),
    [agentFilter, contextQuery, operationFilter, runs, statusFilter, threadLookup]
  );
  const hasFilters =
    contextQuery.trim() ||
    operationFilter !== ALL_FILTER_VALUE ||
    statusFilter !== ALL_FILTER_VALUE ||
    agentFilter !== ALL_FILTER_VALUE;
  const totalPages = Math.max(1, Math.ceil(filteredRuns.length / pageSize));
  const currentPage = Math.min(page, totalPages);
  const pageStart = (currentPage - 1) * pageSize;
  const paginatedRuns = filteredRuns.slice(pageStart, pageStart + pageSize);
  const pageRangeStart = filteredRuns.length === 0 ? 0 : pageStart + 1;
  const pageRangeEnd = Math.min(filteredRuns.length, pageStart + pageSize);

  useEffect(() => {
    setPage(1);
  }, [agentFilter, contextQuery, operationFilter, pageSize, statusFilter]);

  useEffect(() => {
    if (page > totalPages) setPage(totalPages);
  }, [page, totalPages]);

  const clearFilters = () => {
    setContextQuery("");
    setOperationFilter(ALL_FILTER_VALUE);
    setStatusFilter(ALL_FILTER_VALUE);
    setAgentFilter(ALL_FILTER_VALUE);
  };

  if (loading) {
    return <p className="p-6 text-sm text-muted-foreground">Loading runs…</p>;
  }
  if (error) {
    return <p className="p-6 text-sm text-red-600">{error}</p>;
  }
  if (runs.length === 0) {
    return (
      <p className="p-6 text-sm text-muted-foreground">
        No runs yet. Start a chat or eval to see runs here.
      </p>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="shrink-0 border-b px-6 py-3">
        <div className="grid gap-3 md:grid-cols-[minmax(240px,1fr)_150px_150px_150px_auto]">
          <label className="space-y-1">
            <span className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
              Context
            </span>
            <input
              type="search"
              value={contextQuery}
              onChange={(event) => setContextQuery(event.target.value)}
              placeholder="Thread title or id"
              className="form-input h-9 w-full text-sm"
            />
          </label>
          <label className="space-y-1">
            <span className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
              Operation
            </span>
            <select
              value={operationFilter}
              onChange={(event) => setOperationFilter(event.target.value)}
              className="form-select h-9 w-full text-sm"
            >
              <option value={ALL_FILTER_VALUE}>All operations</option>
              {operationOptions.map((operation) => (
                <option key={operation} value={operation}>
                  {operation}
                </option>
              ))}
            </select>
          </label>
          <label className="space-y-1">
            <span className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
              Status
            </span>
            <select
              value={statusFilter}
              onChange={(event) => setStatusFilter(event.target.value)}
              className="form-select h-9 w-full text-sm"
            >
              <option value={ALL_FILTER_VALUE}>All statuses</option>
              {statusOptions.map((status) => (
                <option key={status} value={status}>
                  {status}
                </option>
              ))}
            </select>
          </label>
          <label className="space-y-1">
            <span className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
              Agent Type
            </span>
            <select
              value={agentFilter}
              onChange={(event) => setAgentFilter(event.target.value)}
              className="form-select h-9 w-full text-sm"
            >
              <option value={ALL_FILTER_VALUE}>All agents</option>
              {agentOptions.map((agent) => (
                <option key={agent} value={agent}>
                  {agent === EMPTY_AGENT_VALUE ? "No agent" : agent}
                </option>
              ))}
            </select>
          </label>
          <div className="flex items-end gap-2">
            <button
              type="button"
              onClick={clearFilters}
              disabled={!hasFilters}
              className="h-9 rounded-md border px-3 text-xs font-medium text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
            >
              Clear
            </button>
          </div>
        </div>
      </div>

      {filteredRuns.length === 0 ? (
        <div className="flex min-h-0 flex-1 flex-col">
          <p className="flex-1 px-6 py-8 text-sm text-muted-foreground">
            No runs match these filters.
          </p>
          <PaginationControls
            pageSize={pageSize}
            onPageSizeChange={setPageSize}
            pageRangeStart={pageRangeStart}
            pageRangeEnd={pageRangeEnd}
            filteredCount={filteredRuns.length}
            totalCount={runs.length}
            currentPage={currentPage}
            totalPages={totalPages}
            onPrevious={() => setPage((current) => Math.max(1, current - 1))}
            onNext={() => setPage((current) => Math.min(totalPages, current + 1))}
          />
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 flex-col">
          <div className="min-h-0 flex-1 overflow-y-auto scroll-container">
            <table className="w-full text-sm">
              <thead className="sticky top-0 z-10 border-b bg-background text-left text-xs uppercase text-muted-foreground">
                <tr>
                  <th className="px-6 py-2 font-medium">Run</th>
                  <th className="px-3 py-2 font-medium">Context</th>
                  <th className="px-3 py-2 font-medium">Operation</th>
                  <th className="px-3 py-2 font-medium">Agent</th>
                  <th className="px-3 py-2 font-medium">Status</th>
                  <th className="px-3 py-2 font-medium">Events</th>
                  <th className="px-3 py-2 font-medium">Updated</th>
                </tr>
              </thead>
              <tbody>
                {paginatedRuns.map((run) => {
                  const status = displayStatus(run);
                  return (
                    <tr key={run.runId} className="border-b hover:bg-muted/40">
                      <td className="px-6 py-2 font-mono text-xs">
                        <Link to={`/runs/${run.runId}`} className="text-primary hover:underline">
                          {shortId(run.runId)}
                        </Link>
                      </td>
                      <td className="max-w-[320px] px-3 py-2">
                        <RunContext run={run} threadLookup={threadLookup} />
                      </td>
                      <td className="px-3 py-2">{run.operation}</td>
                      <td className="px-3 py-2">{run.agentId || "—"}</td>
                      <td className={cn("px-3 py-2 font-medium", status.tone)} title={status.title}>
                        {status.label}
                      </td>
                      <td className="px-3 py-2 tabular-nums">{run.eventCount}</td>
                      <td
                        className="px-3 py-2 text-xs text-muted-foreground"
                        title={formatDateTime(run.updatedAt)}
                      >
                        {formatRelativeTime(run.updatedAt)}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
          <PaginationControls
            pageSize={pageSize}
            onPageSizeChange={setPageSize}
            pageRangeStart={pageRangeStart}
            pageRangeEnd={pageRangeEnd}
            filteredCount={filteredRuns.length}
            totalCount={runs.length}
            currentPage={currentPage}
            totalPages={totalPages}
            onPrevious={() => setPage((current) => Math.max(1, current - 1))}
            onNext={() => setPage((current) => Math.min(totalPages, current + 1))}
          />
        </div>
      )}
    </div>
  );
}
