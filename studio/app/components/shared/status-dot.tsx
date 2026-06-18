/**
 * Generic status indicator dot.
 *
 * Colored circle parameterized by a color map.
 * Optional pulse animation for active statuses.
 *
 * @module components/shared/status-dot
 */

import { cn } from "~/lib/utils";

interface StatusDotProps<K extends string> {
  status: K;
  colorMap: Record<K, string>;
  size?: number;
  pulse?: boolean;
  pulseStatuses?: readonly K[];
  className?: string;
}

export function StatusDot<K extends string>({
  status,
  colorMap,
  size = 12,
  pulse,
  pulseStatuses,
  className,
}: StatusDotProps<K>) {
  const color = colorMap[status];
  const shouldPulse = pulse ?? pulseStatuses?.includes(status) ?? false;

  return (
    <span
      className={cn(
        "inline-block rounded-full shrink-0",
        shouldPulse && "animate-pulse",
        className
      )}
      style={{
        width: size,
        height: size,
        backgroundColor: color,
      }}
      title={status}
    />
  );
}
