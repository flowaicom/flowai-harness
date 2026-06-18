import type { DataReadiness } from "~/lib/domain/data";
import {
  type DataReadinessMode,
  dataReadinessCaption,
  dataReadinessGeneratedAtLabel,
  summarizeDataReadiness,
} from "~/lib/domain/data-readiness";
import { cn } from "~/lib/utils";
import { SectionCard, SectionHeader } from "./section-card";

interface MetricCellProps {
  readonly value: string | number;
  readonly label: string;
}

function MetricCell({ value, label }: MetricCellProps) {
  return (
    <div className="rounded-lg bg-muted/30 px-3 py-2">
      <p className="text-base font-semibold tabular-nums text-foreground">
        {typeof value === "number" ? value.toLocaleString() : value}
      </p>
      <p className="text-[11px] text-muted-foreground">{label}</p>
    </div>
  );
}

export interface DataReadinessCardProps {
  readonly readiness: DataReadiness | null | undefined;
  readonly title?: string;
  readonly mode?: DataReadinessMode;
  readonly caption?: string;
  readonly className?: string;
  readonly showEmpty?: boolean;
  readonly emptyMessage?: string;
}

export function DataReadinessCard({
  readiness,
  title = "Workspace Data Context",
  mode = "current",
  caption,
  className,
  showEmpty = false,
  emptyMessage = "Workspace data context has not loaded yet.",
}: DataReadinessCardProps) {
  if (!readiness) {
    if (!showEmpty) return null;
    return (
      <SectionCard className={className}>
        <SectionHeader>{title}</SectionHeader>
        <p className="text-xs text-muted-foreground">{emptyMessage}</p>
      </SectionCard>
    );
  }

  const summary = summarizeDataReadiness(readiness);

  return (
    <SectionCard
      className={cn(
        readiness.ready
          ? "border-[var(--dot-emerald)]/30 bg-[var(--accent-emerald)]/25"
          : "border-[var(--dot-amber)]/30 bg-[var(--accent-amber)]/20",
        className
      )}
    >
      <div className="flex items-start justify-between gap-3">
        <SectionHeader>{title}</SectionHeader>
        <span
          className={cn(
            "rounded-full px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide",
            readiness.ready
              ? "bg-[var(--accent-emerald)] text-[var(--dot-emerald)]"
              : "bg-[var(--accent-amber)] text-[var(--dot-amber)]"
          )}
        >
          {summary.statusLabel}
        </span>
      </div>
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-5">
        <MetricCell value={summary.tableCount} label="Tables" />
        <MetricCell value={summary.totalRows} label="Rows" />
        <MetricCell value={summary.profiledTableCount} label="Profiled" />
        <MetricCell value={summary.documentCount} label="Documents" />
        <MetricCell value={summary.knowledgeCount} label="Knowledge" />
      </div>
      <p className="text-xs text-muted-foreground">
        {caption ?? dataReadinessCaption(readiness, mode)}
      </p>
      <p className="text-[10px] text-muted-foreground/70">
        Snapshot {dataReadinessGeneratedAtLabel(readiness)}
        {readiness.sourceId ? ` · Source ${readiness.sourceId}` : ""}
      </p>
    </SectionCard>
  );
}
