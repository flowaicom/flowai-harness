/**
 * Metric cell — large numeric value with label in a bordered card.
 *
 * Law L5: All numbers use font-mono tabular-nums.
 *
 * @module components/shared/metric-cell
 */

import { cn } from "~/lib/utils";

interface MetricCellProps {
  readonly value: string | number;
  readonly label: string;
  readonly className?: string;
}

export function MetricCell({ value, label, className }: MetricCellProps) {
  return (
    <div className={cn("rounded-lg border p-3", className)}>
      <div className="text-xl font-semibold tabular-nums tracking-tight">{value}</div>
      <div className="text-xs text-muted-foreground mt-0.5">{label}</div>
    </div>
  );
}
