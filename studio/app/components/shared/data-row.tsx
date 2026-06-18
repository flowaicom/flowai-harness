/**
 * Data row — key-value pair rendered as a flex row.
 *
 * Label on the left (muted), value on the right (medium weight).
 * Optional monospace flag for numeric/ID values.
 *
 * Law L5: All numbers use font-mono tabular-nums.
 *
 * @module components/shared/data-row
 */

import { cn } from "~/lib/utils";

interface DataRowProps {
  readonly label: string;
  readonly value: React.ReactNode;
  readonly mono?: boolean;
  readonly className?: string;
}

export function DataRow({ label, value, mono, className }: DataRowProps) {
  return (
    <div className={cn("flex items-center justify-between text-xs", className)}>
      <span className="text-muted-foreground">{label}</span>
      <span className={cn("font-medium", mono && "font-mono tabular-nums")}>{value}</span>
    </div>
  );
}
