/**
 * Generic status tag (pill badge with dot + label).
 *
 * Parameterized by color map and label map.
 *
 * @module components/shared/status-tag
 */

import { cn } from "~/lib/utils";
import { StatusDot } from "./status-dot";

interface StatusTagProps<K extends string> {
  status: K;
  label?: string;
  colorMap: Record<K, string>;
  labelMap: Record<K, string>;
  className?: string;
}

export function StatusTag<K extends string>({
  status,
  label,
  colorMap,
  labelMap,
  className,
}: StatusTagProps<K>) {
  const color = colorMap[status];

  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md text-xs font-medium",
        className
      )}
      style={{
        backgroundColor: `${color}18`,
        color,
      }}
    >
      <StatusDot status={status} colorMap={colorMap} size={8} />
      {label ?? labelMap[status]}
    </span>
  );
}
