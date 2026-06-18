/**
 * Utility functions.
 *
 * @module utils
 */

import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * Merge Tailwind classes with clsx.
 */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}

/**
 * Format a date for display.
 */
export function formatDate(date: string | Date | undefined): string {
  if (!date) return "";
  const d = typeof date === "string" ? new Date(date) : date;
  return d.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: d.getFullYear() !== new Date().getFullYear() ? "numeric" : undefined,
  });
}

/**
 * Format a timestamp for display.
 */
export function formatTime(date: string | Date | undefined): string {
  if (!date) return "";
  const d = typeof date === "string" ? new Date(date) : date;
  return d.toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  });
}

/**
 * Format a date with time.
 */
export function formatDateTime(date: string | Date | undefined): string {
  if (!date) return "";
  return `${formatDate(date)} ${formatTime(date)}`;
}

/**
 * Format a relative time (e.g., "5 minutes ago").
 */
export function formatRelativeTime(date: string | Date | undefined): string {
  if (!date) return "";
  const d = typeof date === "string" ? new Date(date) : date;
  const now = new Date();
  const diff = now.getTime() - d.getTime();

  const seconds = Math.floor(diff / 1000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);

  if (days > 7) {
    return formatDate(d);
  }
  if (days > 0) {
    return `${days}d ago`;
  }
  if (hours > 0) {
    return `${hours}h ago`;
  }
  if (minutes > 0) {
    return `${minutes}m ago`;
  }
  return "just now";
}

/**
 * Format a number with thousands separators.
 */
export function formatNumber(n: number): string {
  return n.toLocaleString();
}

/**
 * Format a duration in milliseconds.
 */
export function formatDuration(ms: number): string {
  if (ms == null || Number.isNaN(ms)) return "—";
  if (ms < 1000) {
    return `${ms}ms`;
  }
  if (ms < 60000) {
    return `${(ms / 1000).toFixed(1)}s`;
  }
  const minutes = Math.floor(ms / 60000);
  const seconds = Math.floor((ms % 60000) / 1000);
  return `${minutes}m ${seconds}s`;
}

/**
 * Truncate text to a maximum length.
 */
export function truncate(text: string, maxLength: number): string {
  if (text.length <= maxLength) return text;
  return `${text.slice(0, maxLength - 3)}...`;
}

/**
 * Debounce a function.
 */
export function debounce<T extends (...args: unknown[]) => unknown>(
  fn: T,
  delay: number
): (...args: Parameters<T>) => void {
  let timeoutId: ReturnType<typeof setTimeout>;
  return (...args: Parameters<T>) => {
    clearTimeout(timeoutId);
    timeoutId = setTimeout(() => fn(...args), delay);
  };
}

/**
 * Throttle a function.
 */
export function throttle<T extends (...args: unknown[]) => unknown>(
  fn: T,
  limit: number
): (...args: Parameters<T>) => void {
  let inThrottle = false;
  return (...args: Parameters<T>) => {
    if (!inThrottle) {
      fn(...args);
      inThrottle = true;
      setTimeout(() => {
        inThrottle = false;
      }, limit);
    }
  };
}

// ============================================================================
// Time Period Grouping (for sidebar sections)
// ============================================================================

export type TimePeriod = "today" | "yesterday" | "thisWeek" | "thisMonth" | "older";

export const TIME_PERIOD_ORDER: TimePeriod[] = [
  "today",
  "yesterday",
  "thisWeek",
  "thisMonth",
  "older",
];

export const TIME_PERIOD_LABELS: Record<TimePeriod, string> = {
  today: "Today",
  yesterday: "Yesterday",
  thisWeek: "This week",
  thisMonth: "This month",
  older: "Older",
};

/** Classify a date string into a time period bucket. */
export function getTimePeriod(date: string | Date | undefined): TimePeriod {
  if (!date) return "older";
  const d = typeof date === "string" ? new Date(date) : date;
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const yesterday = new Date(today.getTime() - 86_400_000);
  const weekAgo = new Date(today.getTime() - 7 * 86_400_000);
  const monthAgo = new Date(today.getTime() - 30 * 86_400_000);

  if (d >= today) return "today";
  if (d >= yesterday) return "yesterday";
  if (d >= weekAgo) return "thisWeek";
  if (d >= monthAgo) return "thisMonth";
  return "older";
}

/** Group sorted items by time period. Returns ordered entries (only non-empty groups). */
export function groupByTimePeriod<T>(
  items: readonly T[],
  getDate: (item: T) => string
): { period: TimePeriod; label: string; items: T[] }[] {
  const groups = new Map<TimePeriod, T[]>();
  for (const item of items) {
    const period = getTimePeriod(getDate(item));
    const arr = groups.get(period);
    if (arr) arr.push(item);
    else groups.set(period, [item]);
  }
  return TIME_PERIOD_ORDER.flatMap((p) => {
    const items = groups.get(p);
    return items ? [{ period: p, label: TIME_PERIOD_LABELS[p], items }] : [];
  });
}

/**
 * Compact relative time for sidebar items (minimal space).
 * "now" / "2m" / "3h" / "1d" / "Jan 5"
 */
export function compactRelativeTime(date: string | Date | undefined): string {
  if (!date) return "";
  const d = typeof date === "string" ? new Date(date) : date;
  const now = new Date();
  const diff = now.getTime() - d.getTime();
  const minutes = Math.floor(diff / 60_000);
  const hours = Math.floor(diff / 3_600_000);
  const days = Math.floor(diff / 86_400_000);

  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m`;
  if (hours < 24) return `${hours}h`;
  if (days < 7) return `${days}d`;
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/**
 * Structural deep equality for JSON-serializable values.
 *
 * Key-order independent (unlike JSON.stringify). Handles null, primitives,
 * arrays (order-sensitive), and plain objects.
 */
export function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (a === null || b === null) return false;
  if (typeof a !== typeof b) return false;
  if (typeof a !== "object") return false;

  if (Array.isArray(a)) {
    if (!Array.isArray(b) || a.length !== b.length) return false;
    return a.every((v, i) => deepEqual(v, b[i]));
  }

  if (Array.isArray(b)) return false;

  const aObj = a as Record<string, unknown>;
  const bObj = b as Record<string, unknown>;
  const aKeys = Object.keys(aObj);
  const bKeys = Object.keys(bObj);
  if (aKeys.length !== bKeys.length) return false;
  return aKeys.every((k) => Object.hasOwn(bObj, k) && deepEqual(aObj[k], bObj[k]));
}

/**
 * Exhaustive check sentinel for discriminated unions.
 *
 * Causes a compile-time error if a switch/if-chain is not exhaustive.
 * At runtime, throws if reached (indicates a missed case).
 */
export function assertNever(x: never): never {
  throw new Error(`Unexpected discriminant: ${JSON.stringify(x)}`);
}
