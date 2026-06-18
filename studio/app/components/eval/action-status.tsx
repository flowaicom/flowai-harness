import type { ActionStatus } from "~/lib/domain/eval";
import { cn } from "~/lib/utils";

export const ACTION_STATUS_COLORS: Record<ActionStatus, string> = {
  exact: "bg-[var(--dot-emerald)]",
  products_wrong: "bg-[var(--dot-amber)]",
  scope_wrong: "bg-[var(--dot-orange)]",
  both_wrong: "bg-[var(--dot-red)]",
  missing: "bg-muted-foreground/50",
  extra: "bg-[var(--dot-purple)]",
};

export const ACTION_STATUS_LABELS: Record<ActionStatus, string> = {
  exact: "exact",
  products_wrong: "products",
  scope_wrong: "scope",
  both_wrong: "both",
  missing: "missing",
  extra: "extra",
};

export function ActionStatusPill({ status }: { readonly status: ActionStatus }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full px-1.5 py-0.5 text-[10px] font-medium text-white",
        ACTION_STATUS_COLORS[status]
      )}
    >
      {ACTION_STATUS_LABELS[status]}
    </span>
  );
}
