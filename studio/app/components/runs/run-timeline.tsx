import {
  ActivityIcon,
  CheckIcon,
  ChevronDownIcon,
  ClockIcon,
  ExternalLinkIcon,
  XIcon,
} from "lucide-react";
import { Link } from "react-router";
import type { RunTimelineItem, RunTimelineStatus } from "~/lib/domain/run-events";
import { cn } from "~/lib/utils";

function stringifyJson(value: unknown): string {
  if (value === undefined) return "";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function previewJson(value: unknown): string {
  const text = stringifyJson(value).replace(/\s+/g, " ").trim();
  if (!text) return "";
  return text.length > 96 ? `${text.slice(0, 95)}…` : text;
}

function StatusIcon({ status }: { readonly status: RunTimelineStatus }) {
  if (status === "failed") return <XIcon className="size-3.5 text-red-400" />;
  if (status === "completed") return <CheckIcon className="size-3.5 text-emerald-400" />;
  if (status === "pending") return <ClockIcon className="size-3.5 text-amber-400" />;
  return <ActivityIcon className="size-3.5 text-blue-400" />;
}

function JsonDetails({
  name,
  value,
  defaultOpen = false,
}: {
  readonly name: string;
  readonly value: unknown;
  readonly defaultOpen?: boolean;
}) {
  const json = stringifyJson(value);
  if (!json) return null;
  const preview = previewJson(value);

  return (
    <details
      open={defaultOpen}
      className="group rounded-md border bg-background/40 px-2 py-1 text-xs"
    >
      <summary className="flex cursor-pointer list-none items-center gap-2 text-muted-foreground">
        <ChevronDownIcon className="size-3 transition-transform group-open:rotate-180" />
        <span className="font-medium uppercase tracking-wide">{name}</span>
        {preview && <span className="min-w-0 truncate font-mono opacity-70">{preview}</span>}
      </summary>
      <pre className="mt-2 max-h-72 overflow-auto rounded-md bg-muted/40 p-2 text-[11px] leading-relaxed text-foreground">
        {json}
      </pre>
    </details>
  );
}

export function RunTimelineItemCard({ item }: { readonly item: RunTimelineItem }) {
  const raw = item.rawEvents.map((event) => ({
    seq: event.seq,
    kind: event.kind,
    event: event.event,
    raw: event.raw,
  }));

  return (
    <li className="rounded-lg border bg-card p-3">
      <div className="flex items-start gap-3">
        <div className="mt-0.5 rounded-md bg-muted/60 p-1">
          <StatusIcon status={item.status} />
        </div>
        <div className="min-w-0 flex-1 space-y-2">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <p className="truncate text-sm font-medium">{item.label}</p>
            <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium uppercase text-muted-foreground">
              {item.status}
            </span>
            <span className="font-mono text-[10px] text-muted-foreground">
              #{item.seq}
              {item.completedSeq ? ` → #${item.completedSeq}` : ""}
            </span>
          </div>
          <p className="font-mono text-[11px] text-muted-foreground">{item.eventKind}</p>
          <div className="space-y-1.5">
            <JsonDetails name="input" value={item.input} />
            <JsonDetails name="output" value={item.output} />
            <JsonDetails name="raw event" value={raw} />
          </div>
        </div>
      </div>
    </li>
  );
}

export function RunTimeline({
  items,
  compact = false,
}: {
  readonly items: readonly RunTimelineItem[];
  readonly compact?: boolean;
}) {
  if (items.length === 0) {
    return (
      <div className="rounded-lg border p-4 text-sm text-muted-foreground">
        No run events captured yet.
      </div>
    );
  }

  const visibleItems = compact ? items.slice(0, 5) : items;

  return (
    <ol className={cn("space-y-2", compact && "space-y-1.5")}>
      {visibleItems.map((item) => (
        <RunTimelineItemCard key={item.id} item={item} />
      ))}
      {compact && items.length > visibleItems.length && (
        <li className="px-3 py-1 text-xs text-muted-foreground">
          +{items.length - visibleItems.length} more event
          {items.length - visibleItems.length === 1 ? "" : "s"}
        </li>
      )}
    </ol>
  );
}

export function RunActivityPanel({
  runId,
  items,
  loading,
  error,
  className,
}: {
  readonly runId: string | null | undefined;
  readonly items: readonly RunTimelineItem[];
  readonly loading?: boolean;
  readonly error?: string | null;
  readonly className?: string;
}) {
  const toolCount = items.filter((item) => item.kind === "tool").length;
  const subAgentCount = items.filter((item) => item.kind === "subAgent").length;

  return (
    <section className={cn("rounded-lg border bg-card p-3", className)}>
      <div className="mb-2 flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <ActivityIcon className="size-4 text-muted-foreground" />
          <div>
            <h2 className="text-sm font-medium">Run activity</h2>
            <p className="text-xs text-muted-foreground">
              {runId
                ? `${toolCount} tool${toolCount === 1 ? "" : "s"} · ${subAgentCount} sub-agent${subAgentCount === 1 ? "" : "s"}`
                : "No recent run activity"}
            </p>
          </div>
        </div>
        <Link
          to={runId ? `/runs/${runId}` : "/runs"}
          className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          Runs
          <ExternalLinkIcon className="size-3" />
        </Link>
      </div>
      {error ? (
        <p className="text-xs text-red-500">{error}</p>
      ) : loading && items.length === 0 ? (
        <div className="space-y-1">
          <div className="h-8 rounded bg-muted animate-shimmer" />
          <div className="h-8 rounded bg-muted/70 animate-shimmer" />
        </div>
      ) : runId ? (
        <RunTimeline items={items} compact />
      ) : (
        <p className="text-xs text-muted-foreground">
          Open the Runs page to inspect persisted workspace activity.
        </p>
      )}
    </section>
  );
}

export function RunActivitySummaryPanel({
  runId,
  items = [],
  loading,
  error,
  className,
}: {
  readonly runId: string | null | undefined;
  readonly items?: readonly RunTimelineItem[];
  readonly loading?: boolean;
  readonly error?: string | null;
  readonly className?: string;
}) {
  return (
    <section className={cn("rounded-lg border bg-card p-3", className)}>
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <ActivityIcon className="size-4 text-muted-foreground" />
          <div>
            <h2 className="text-sm font-medium">Run activity</h2>
            <p className="text-xs text-muted-foreground">
              {error
                ? "Run detail is available separately."
                : loading
                  ? "Loading run link..."
                  : `${items.length} event${items.length === 1 ? "" : "s"}`}
            </p>
          </div>
        </div>
        {runId && (
          <Link
            to={`/runs/${runId}`}
            className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            Go to run
            <ExternalLinkIcon className="size-3" />
          </Link>
        )}
      </div>
    </section>
  );
}

export function WorkspaceActivityLink({ className }: { readonly className?: string }) {
  return (
    <Link
      to="/runs"
      className={cn(
        "flex items-center justify-between rounded-lg border bg-card px-3 py-2 text-sm hover:bg-muted/40",
        className
      )}
    >
      <span className="flex items-center gap-2">
        <ActivityIcon className="size-4 text-muted-foreground" />
        <span>Workspace activity</span>
      </span>
      <span className="flex items-center gap-1 text-xs text-muted-foreground">
        Runs
        <ExternalLinkIcon className="size-3" />
      </span>
    </Link>
  );
}
