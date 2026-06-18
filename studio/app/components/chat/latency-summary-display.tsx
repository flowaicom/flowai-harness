/**
 * Latency summary display component.
 *
 * Pure view component for backend-provided latency metrics.
 * Backend owns computation, frontend is representational.
 *
 * Performance optimizations:
 * - Memoized component with custom comparator
 * - useMemo for expensive computations
 * - useCallback for stable handlers
 *
 * @module components/chat/latency-summary-display
 */

import { memo, useCallback, useMemo, useState } from "react";
import type { RetryEvent, RetryReason } from "~/lib/domain";
import type { StoreLatencySummary } from "~/lib/stores";
import { cn } from "~/lib/utils";

// ============================================================================
// Pure Formatting Functions
// ============================================================================

/**
 * Format milliseconds for display.
 * Pure function: same input always produces same output.
 */
function formatMs(ms: number | null | undefined): string {
  if (ms == null) return "—";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

/**
 * Format bytes for display.
 */
function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null) return "—";
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)}MB`;
}

/**
 * Get quality indicator color based on duration.
 */
function getLatencyQuality(ms: number | null): "good" | "fair" | "poor" | "unknown" {
  if (ms === null) return "unknown";
  if (ms < 500) return "good";
  if (ms < 1500) return "fair";
  return "poor";
}

const qualityColors = {
  good: "text-[var(--dot-emerald)] bg-[var(--accent-emerald)]",
  fair: "text-[var(--dot-amber)] bg-[var(--accent-amber)]",
  poor: "text-[var(--dot-red)] bg-[var(--accent-red)]",
  unknown: "text-muted-foreground bg-muted/50",
} as const;

/**
 * Human-readable label for retry reasons.
 */
function formatRetryReason(reason: RetryReason): string {
  switch (reason) {
    case "rate_limit":
      return "Rate Limit";
    case "timeout":
      return "Timeout";
    case "context_length":
      return "Ctx Length";
    case "server_error":
      return "Server";
    case "network_error":
      return "Network";
    case "content_filter":
      return "Filter";
    default:
      return "Unknown";
  }
}

/**
 * Color class for retry reason severity.
 */
function getRetryReasonColor(reason: RetryReason): string {
  switch (reason) {
    case "rate_limit":
      return "text-[var(--dot-amber)] bg-[var(--accent-amber)]";
    case "timeout":
      return "text-[var(--dot-orange)] bg-[var(--accent-orange)]";
    case "context_length":
      return "text-[var(--dot-red)] bg-[var(--accent-red)]";
    case "server_error":
      return "text-[var(--dot-red)] bg-[var(--accent-red)]";
    case "network_error":
      return "text-[var(--dot-orange)] bg-[var(--accent-orange)]";
    case "content_filter":
      return "text-[var(--dot-purple)] bg-[var(--accent-purple)]";
    default:
      return "text-muted-foreground bg-muted/50";
  }
}

/**
 * Count retry events by reason.
 */
function countRetrysByReason(events: readonly RetryEvent[]): Map<RetryReason, number> {
  const counts = new Map<RetryReason, number>();
  for (const event of events) {
    counts.set(event.reason, (counts.get(event.reason) ?? 0) + 1);
  }
  return counts;
}

// ============================================================================
// Props
// ============================================================================

interface LatencySummaryDisplayProps {
  /** Backend-provided latency summary */
  summary: StoreLatencySummary;
  /** Optional class name */
  className?: string;
}

// ============================================================================
// Phase Breakdown Component (Memoized)
// ============================================================================

interface PhaseBreakdownProps {
  llmTimeMs: number;
  toolTimeMs: number;
  llmCalls: number;
  totalDurationMs: number;
}

const PhaseBreakdown = memo(function PhaseBreakdown({
  llmTimeMs,
  toolTimeMs,
  llmCalls,
  totalDurationMs,
}: PhaseBreakdownProps) {
  // Memoize percentages
  const { llmPercent, toolPercent, otherPercent } = useMemo(() => {
    if (totalDurationMs === 0) return { llmPercent: 0, toolPercent: 0, otherPercent: 0 };
    const llm = Math.round((llmTimeMs / totalDurationMs) * 100);
    const tool = Math.round((toolTimeMs / totalDurationMs) * 100);
    const other = Math.max(0, 100 - llm - tool);
    return { llmPercent: llm, toolPercent: tool, otherPercent: other };
  }, [llmTimeMs, toolTimeMs, totalDurationMs]);

  return (
    <div className="space-y-2">
      <div className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
        Phase Breakdown
      </div>

      {/* Visual bar */}
      <div className="flex h-2 rounded-full overflow-hidden bg-muted">
        {llmPercent > 0 && (
          <div
            className="bg-[var(--dot-blue)] transition-all duration-300"
            style={{ width: `${llmPercent}%` }}
            title={`LLM: ${formatMs(llmTimeMs)} (${llmPercent}%)`}
          />
        )}
        {toolPercent > 0 && (
          <div
            className="bg-[var(--dot-purple)] transition-all duration-300"
            style={{ width: `${toolPercent}%` }}
            title={`Tools: ${formatMs(toolTimeMs)} (${toolPercent}%)`}
          />
        )}
        {otherPercent > 0 && (
          <div
            className="bg-muted-foreground/30 transition-all duration-300"
            style={{ width: `${otherPercent}%` }}
            title={`Other: ${otherPercent}%`}
          />
        )}
      </div>

      {/* Legend */}
      <div className="flex flex-wrap gap-4 text-xs text-muted-foreground">
        <div className="flex items-center gap-1.5">
          <span className="w-2 h-2 rounded-full bg-[var(--dot-blue)]" />
          <span>
            LLM: {formatMs(llmTimeMs)} ({llmPercent}%)
          </span>
        </div>
        <div className="flex items-center gap-1.5">
          <span className="w-2 h-2 rounded-full bg-[var(--dot-purple)]" />
          <span>
            Tools: {formatMs(toolTimeMs)} ({toolPercent}%)
          </span>
        </div>
        <div className="flex items-center gap-1.5">
          <span className="w-2 h-2 rounded-full bg-muted-foreground/30" />
          <span>Other: {otherPercent}%</span>
        </div>
      </div>

      {/* LLM calls count */}
      <div className="text-xs text-muted-foreground">
        {llmCalls} LLM call{llmCalls !== 1 ? "s" : ""}
      </div>
    </div>
  );
});

// ============================================================================
// Tool Timing Row Component (Memoized)
// ============================================================================

interface ToolTimingRowProps {
  toolName: string;
  durationMs: number;
  status: "completed" | "error";
  payloadSize?: number;
}

const ToolTimingRow = memo(
  function ToolTimingRow({ toolName, durationMs, status, payloadSize }: ToolTimingRowProps) {
    return (
      <div className="flex items-center justify-between py-1.5 text-xs">
        <div className="flex items-center gap-2 min-w-0">
          {status === "completed" ? (
            <svg
              aria-hidden="true"
              className="w-3.5 h-3.5 text-[var(--dot-emerald)] flex-shrink-0"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M5 13l4 4L19 7"
              />
            </svg>
          ) : (
            <svg
              aria-hidden="true"
              className="w-3.5 h-3.5 text-[var(--dot-red)] flex-shrink-0"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M6 18L18 6M6 6l12 12"
              />
            </svg>
          )}
          <span className="font-mono truncate">{toolName}</span>
        </div>
        <div className="flex items-center gap-3 text-muted-foreground flex-shrink-0">
          {payloadSize != null && <span>{formatBytes(payloadSize)}</span>}
          <span className="font-medium text-foreground">{formatMs(durationMs)}</span>
        </div>
      </div>
    );
  },
  (prev, next) =>
    prev.toolName === next.toolName &&
    prev.durationMs === next.durationMs &&
    prev.status === next.status &&
    prev.payloadSize === next.payloadSize
);

// ============================================================================
// Main Component (Memoized with custom comparator)
// ============================================================================

/**
 * Custom comparator for latency summary.
 * Only re-render when summary data actually changes.
 */
const areLatencySummariesEqual = (
  prev: LatencySummaryDisplayProps,
  next: LatencySummaryDisplayProps
): boolean => {
  if (prev.className !== next.className) return false;

  const p = prev.summary;
  const n = next.summary;

  // Compare scalar values
  if (
    p.totalDurationMs !== n.totalDurationMs ||
    p.retryCount !== n.retryCount ||
    p.hadTimeout !== n.hadTimeout ||
    p.productSetSize !== n.productSetSize ||
    p.planPayloadBytes !== n.planPayloadBytes
  )
    return false;

  // Compare phases
  if (
    p.phases.llmTimeMs !== n.phases.llmTimeMs ||
    p.phases.toolTimeMs !== n.phases.toolTimeMs ||
    p.phases.llmCalls !== n.phases.llmCalls
  )
    return false;

  // Compare tool timings length
  if (p.toolTimings.length !== n.toolTimings.length) return false;

  return true;
};

export const LatencySummaryDisplay = memo(function LatencySummaryDisplay({
  summary,
  className,
}: LatencySummaryDisplayProps) {
  const [isOpen, setIsOpen] = useState(false);

  // Stable toggle handler
  const handleToggle = useCallback(() => setIsOpen((prev) => !prev), []);

  // Memoize tool timings sorted by duration
  const sortedTools = useMemo(
    () => [...summary.toolTimings].sort((a, b) => b.durationMs - a.durationMs),
    [summary.toolTimings]
  );

  const quality = getLatencyQuality(summary.phases.llmTimeMs);

  return (
    <div className={cn("border border-border rounded-lg overflow-hidden", className)}>
      {/* Header - Always visible */}
      <button
        type="button"
        onClick={handleToggle}
        className="w-full flex items-center justify-between px-3 py-2 bg-muted/50 hover:bg-muted transition-colors"
      >
        <span className="flex items-center gap-2 text-sm font-medium text-foreground">
          <svg
            aria-hidden="true"
            className="w-4 h-4"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
            />
          </svg>
          Performance
        </span>

        <div className="flex items-center gap-3">
          {/* Quick stats */}
          <span className={cn("px-2 py-0.5 rounded text-xs font-medium", qualityColors[quality])}>
            {formatMs(summary.totalDurationMs)}
          </span>

          {summary.toolTimings.length > 0 && (
            <span className="text-xs text-muted-foreground">
              {summary.toolTimings.length} tool{summary.toolTimings.length !== 1 ? "s" : ""}
            </span>
          )}

          {(summary.retryCount > 0 || summary.hadTimeout) && (
            <span className="text-xs text-[var(--dot-amber)]">
              {summary.retryCount > 0 && `${summary.retryCount} retries`}
              {summary.hadTimeout && " timeout"}
            </span>
          )}

          <svg
            aria-hidden="true"
            className={cn(
              "w-4 h-4 text-muted-foreground transition-transform",
              isOpen && "rotate-180"
            )}
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
          >
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
          </svg>
        </div>
      </button>

      {/* Expanded content */}
      {isOpen && (
        <div className="px-3 py-3 text-sm border-t border-border space-y-4">
          {/* Phase breakdown */}
          <PhaseBreakdown
            llmTimeMs={summary.phases.llmTimeMs}
            toolTimeMs={summary.phases.toolTimeMs}
            llmCalls={summary.phases.llmCalls}
            totalDurationMs={summary.totalDurationMs}
          />

          {/* TTFT Metrics (Backend Authoritative) */}
          {(summary.ttftMs != null || summary.firstTextMs != null) && (
            <div className="flex flex-wrap gap-4 text-xs text-muted-foreground pt-2 border-t border-border/50">
              {summary.ttftMs != null && (
                <div>
                  <span className="text-muted-foreground">TTFT:</span>{" "}
                  <span className="font-medium">{formatMs(summary.ttftMs)}</span>
                </div>
              )}
              {summary.firstTextMs != null && (
                <div>
                  <span className="text-muted-foreground">First Text:</span>{" "}
                  <span className="font-medium">{formatMs(summary.firstTextMs)}</span>
                </div>
              )}
            </div>
          )}

          {/* Tool timings */}
          {sortedTools.length > 0 && (
            <div className="space-y-1">
              <div className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
                Tool Execution
              </div>
              <div className="divide-y divide-border/50">
                {sortedTools.map((tool) => (
                  <ToolTimingRow
                    key={tool.toolCallId}
                    toolName={tool.toolName}
                    durationMs={tool.durationMs}
                    status={tool.status}
                    payloadSize={tool.payloadSize}
                  />
                ))}
              </div>
            </div>
          )}

          {/* Agent-specific metrics */}
          {(summary.productSetSize != null || summary.planPayloadBytes != null) && (
            <div className="flex flex-wrap gap-4 text-xs text-muted-foreground pt-2 border-t border-border/50">
              {summary.productSetSize != null && (
                <div>
                  <span className="text-muted-foreground">Product Set:</span>{" "}
                  <span className="font-medium">{summary.productSetSize.toLocaleString()}</span>
                </div>
              )}
              {summary.planPayloadBytes != null && (
                <div>
                  <span className="text-muted-foreground">Plan Size:</span>{" "}
                  <span className="font-medium">{formatBytes(summary.planPayloadBytes)}</span>
                </div>
              )}
            </div>
          )}

          {/* Error indicators with categorized retry breakdown */}
          {(summary.retryCount > 0 ||
            summary.hadTimeout ||
            (summary.retryEvents && summary.retryEvents.length > 0)) && (
            <div className="space-y-2">
              <div className="flex items-center gap-2 text-xs text-[var(--dot-amber)] bg-[var(--accent-amber)] rounded px-2 py-1">
                <svg
                  aria-hidden="true"
                  className="w-4 h-4"
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
                  />
                </svg>
                <span>
                  {summary.retryCount > 0 &&
                    `${summary.retryCount} retry attempt${summary.retryCount !== 1 ? "s" : ""}`}
                  {summary.retryCount > 0 && summary.hadTimeout && " • "}
                  {summary.hadTimeout && "Request timed out"}
                </span>
              </div>

              {/* Categorized retry breakdown */}
              {summary.retryEvents && summary.retryEvents.length > 0 && (
                <div className="flex flex-wrap gap-1">
                  {Array.from(countRetrysByReason(summary.retryEvents)).map(([reason, count]) => (
                    <span
                      key={reason}
                      className={cn(
                        "text-[10px] px-1.5 py-0.5 rounded",
                        getRetryReasonColor(reason)
                      )}
                    >
                      {formatRetryReason(reason)}: {count}
                    </span>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}, areLatencySummariesEqual);
