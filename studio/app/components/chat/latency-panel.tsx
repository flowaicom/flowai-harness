/**
 * Latency panel component with live streaming metrics.
 *
 * Features:
 * - Live elapsed time during streaming (100ms interval)
 * - Session history (last 10 requests)
 * - Phase breakdown (LLM vs Tool time)
 * - Tool timing details (expandable)
 * - Cost display (tokens + USD)
 *
 * Performance optimizations:
 * - Custom memo comparators for frequent updates
 * - Memoized wall-clock calculations
 * - useCallback for stable handlers
 *
 * Backend owns authoritative metrics,
 * client tracks TTFC perspective only.
 *
 * @module components/chat/latency-panel
 */

import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CostSummary, FinishReason, RetryEvent, RetryReason, ToolTiming } from "~/lib/domain";
import {
  avgLLMLatencyMs,
  avgToolLatencyMs,
  costCacheHitRatePercent,
  inputTokensPerSecond,
  isKVMetricsZero,
  isTokenMetricsZero,
  outputTokensPerSecond,
  overheadMs,
  tokenCacheHitRatePercent,
  tokensPerSecond,
} from "~/lib/domain";
import type { StoreLatencySummary } from "~/lib/stores";
import { cn } from "~/lib/utils";

// ============================================================================
// Types
// ============================================================================

/**
 * Client-side latency metrics (perspective metrics).
 */
export interface ClientLatencyMetrics {
  /** Time from submit to first chunk (network + model thinking) */
  timeToFirstChunk: number | null;
  /** Time from submit to first text token */
  timeToFirstToken: number | null;
  /** When the request was submitted (performance.now()) */
  submitTime: number | null;
  /** Total chunks received */
  chunkCount: number;
}

/**
 * Props for the latency panel.
 */
interface LatencyPanelProps {
  /** Client-side metrics (TTFC, TTFT) */
  clientMetrics: ClientLatencyMetrics;
  /** Backend-owned latency summary (authoritative) */
  backendLatency: StoreLatencySummary | null;
  /** Cost summary (tokens + USD) */
  costSummary: CostSummary | null;
  /** Stream finish reason (null = not finished yet or "stop"). */
  finishReason?: FinishReason | null;
  /** Whether currently streaming */
  isStreaming: boolean;
  /** Close handler */
  onClose?: () => void;
  /** Panel position */
  position?: "top-right" | "bottom-right" | "bottom-left";
  /** Optional className */
  className?: string;
}

// ============================================================================
// Pure Functions
// ============================================================================

/**
 * Format milliseconds for display.
 */
function formatMs(ms: number | null): string {
  if (ms === null) return "-";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

/**
 * Get latency quality indicator.
 */
function getLatencyQuality(ttfc: number | null): "good" | "fair" | "poor" | "unknown" {
  if (ttfc === null) return "unknown";
  if (ttfc < 500) return "good";
  if (ttfc < 1500) return "fair";
  return "poor";
}

const qualityColors = {
  good: "text-[var(--dot-emerald)]",
  fair: "text-[var(--dot-amber)]",
  poor: "text-[var(--dot-red)]",
  unknown: "text-muted-foreground/60",
} as const;

/**
 * Format cost as USD.
 */
function _formatCost(usd: number): string {
  if (usd < 0.001) return "<$0.001";
  if (usd < 0.01) return `$${usd.toFixed(4)}`;
  if (usd < 1) return `$${usd.toFixed(3)}`;
  return `$${usd.toFixed(2)}`;
}

/**
 * Format token count.
 */
function formatTokens(tokens: number): string {
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`;
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(1)}k`;
  return tokens.toString();
}

/**
 * Format tokens per second throughput.
 */
function formatThroughput(tps: number | null): string {
  if (tps === null) return "-";
  if (tps >= 1000) return `${(tps / 1000).toFixed(1)}k/s`;
  return `${tps.toFixed(1)}/s`;
}

/**
 * Format percentage (0-100).
 */
function formatPercent(ratio: number | null): string {
  if (ratio === null) return "-";
  return `${(ratio * 100).toFixed(1)}%`;
}

/**
 * Format bytes for display.
 */
function formatBytes(bytes: number): string {
  if (bytes >= 1_048_576) return `${(bytes / 1_048_576).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${bytes} B`;
}

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
      return "Context Length";
    case "server_error":
      return "Server Error";
    case "network_error":
      return "Network Error";
    case "content_filter":
      return "Content Filter";
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

/**
 * Calculate wall-clock tool time (merging overlapping intervals).
 */
function calculateWallClockToolTime(toolTimings: readonly ToolTiming[]): number {
  if (toolTimings.length === 0) return 0;

  // Extract intervals
  const intervals: Array<[number, number]> = [];
  for (const tool of toolTimings) {
    if (tool.durationMs > 0) {
      // Approximate: use durationMs as the interval length
      intervals.push([0, tool.durationMs]);
    }
  }

  if (intervals.length === 0) return 0;

  // For backend metrics, we use the sum (backend already accounts for parallelism)
  return toolTimings.reduce((sum, t) => sum + t.durationMs, 0);
}

// ============================================================================
// Session History
// ============================================================================

interface HistoryEntry {
  timestamp: number;
  ttfc: number | null;
  totalDuration: number | null;
  toolCount: number;
}

/**
 * Calculate session averages.
 */
function calculateAverages(history: HistoryEntry[]): {
  avgTtfc: number | null;
  avgDuration: number | null;
} {
  const ttfcs = history.map((h) => h.ttfc).filter((v): v is number => v !== null);
  const durations = history.map((h) => h.totalDuration).filter((v): v is number => v !== null);

  return {
    avgTtfc: ttfcs.length > 0 ? Math.round(ttfcs.reduce((a, b) => a + b, 0) / ttfcs.length) : null,
    avgDuration:
      durations.length > 0
        ? Math.round(durations.reduce((a, b) => a + b, 0) / durations.length)
        : null,
  };
}

// ============================================================================
// Sub-components
// ============================================================================

interface MetricCardProps {
  label: string;
  value: string;
  sublabel?: string;
  quality?: "good" | "fair" | "poor" | "unknown";
  isActive?: boolean;
}

/**
 * Custom comparator for MetricCard.
 */
function areMetricCardPropsEqual(prev: MetricCardProps, next: MetricCardProps): boolean {
  return (
    prev.label === next.label &&
    prev.value === next.value &&
    prev.sublabel === next.sublabel &&
    prev.quality === next.quality &&
    prev.isActive === next.isActive
  );
}

const qualityLabels: Record<string, string> = {
  good: "good",
  fair: "fair",
  poor: "poor",
};

const MetricCard = memo(function MetricCard({
  label,
  value,
  sublabel,
  quality,
  isActive,
}: MetricCardProps) {
  const qualityText = quality && quality !== "unknown" ? qualityLabels[quality] : undefined;
  return (
    <div
      className={cn("bg-muted/50 rounded p-2", isActive && "ring-1 ring-ring/30")}
      aria-label={qualityText ? `${label}: ${value} (${qualityText})` : `${label}: ${value}`}
    >
      <div className="text-[10px] text-muted-foreground truncate">{label}</div>
      <div className={cn("font-mono font-medium text-sm", quality && qualityColors[quality])}>
        {isActive && value === "-" ? <span className="animate-pulse">...</span> : value}
        {qualityText && (
          <span className="ml-1 text-[10px] font-normal opacity-70">({qualityText})</span>
        )}
      </div>
      {sublabel && <div className="text-[10px] text-muted-foreground/60">{sublabel}</div>}
    </div>
  );
}, areMetricCardPropsEqual);

interface ToolRowProps {
  toolName: string;
  durationMs: number;
  status: "completed" | "error";
}

const ToolRow = memo(function ToolRow({ toolName, durationMs, status }: ToolRowProps) {
  const isError = status === "error";

  return (
    <div className="flex items-center justify-between px-1 py-0.5 text-[11px]">
      <span className={cn("truncate", isError && "text-[var(--dot-red)]/80")}>{toolName}</span>
      <span
        className={cn(
          "font-mono text-[10px] shrink-0 ml-2 tabular-nums text-muted-foreground",
          isError && "text-[var(--dot-red)]/80"
        )}
      >
        {formatMs(durationMs)}
      </span>
    </div>
  );
});

interface PhaseBarProps {
  llmTimeMs: number;
  toolTimeMs: number;
  totalDurationMs: number;
}

const PhaseBar = memo(function PhaseBar({ llmTimeMs, toolTimeMs, totalDurationMs }: PhaseBarProps) {
  const { llmPercent, toolPercent, otherPercent } = useMemo(() => {
    if (totalDurationMs === 0) return { llmPercent: 0, toolPercent: 0, otherPercent: 0 };
    const llm = Math.round((llmTimeMs / totalDurationMs) * 100);
    const tool = Math.round((toolTimeMs / totalDurationMs) * 100);
    const other = Math.max(0, 100 - llm - tool);
    return { llmPercent: llm, toolPercent: tool, otherPercent: other };
  }, [llmTimeMs, toolTimeMs, totalDurationMs]);

  return (
    <div className="space-y-1">
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
      <div className="flex flex-wrap gap-3 text-[10px] text-muted-foreground">
        <div className="flex items-center gap-1">
          <span className="w-2 h-2 rounded-full bg-[var(--dot-blue)]" />
          <span>
            LLM {formatMs(llmTimeMs)} ({llmPercent}%)
          </span>
        </div>
        <div className="flex items-center gap-1">
          <span className="w-2 h-2 rounded-full bg-[var(--dot-purple)]" />
          <span>
            Tools {formatMs(toolTimeMs)} ({toolPercent}%)
          </span>
        </div>
      </div>
    </div>
  );
});

// ============================================================================
// Main Component
// ============================================================================

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: UI component with conditional rendering
export const LatencyPanel = memo(function LatencyPanel({
  clientMetrics,
  backendLatency,
  costSummary,
  finishReason,
  isStreaming,
  onClose,
  position = "bottom-right",
  className,
}: LatencyPanelProps) {
  const [isExpanded, setIsExpanded] = useState(true);
  const [showToolDetails, setShowToolDetails] = useState(false);
  const [elapsed, setElapsed] = useState<number | null>(null);
  const [history, setHistory] = useState<HistoryEntry[]>([]);

  // Track previous streaming state for history
  const prevStreamingRef = useRef(isStreaming);

  // Stable toggle handlers
  const handleToggleExpand = useCallback(() => setIsExpanded((prev) => !prev), []);
  const handleToggleToolDetails = useCallback(() => setShowToolDetails((prev) => !prev), []);

  // Live elapsed time during streaming
  useEffect(() => {
    if (!isStreaming || clientMetrics.submitTime === null) {
      setElapsed(null);
      return;
    }

    const updateElapsed = () => {
      if (clientMetrics.submitTime !== null) {
        setElapsed(Math.round(performance.now() - clientMetrics.submitTime));
      }
    };

    updateElapsed();
    const interval = setInterval(updateElapsed, 100);
    return () => clearInterval(interval);
  }, [isStreaming, clientMetrics.submitTime]);

  // Save to history when streaming completes
  useEffect(() => {
    if (prevStreamingRef.current && !isStreaming && backendLatency) {
      setHistory((prev) => [
        {
          timestamp: Date.now(),
          ttfc: clientMetrics.timeToFirstChunk,
          totalDuration: backendLatency.totalDurationMs,
          toolCount: backendLatency.toolTimings.length,
        },
        ...prev.slice(0, 9),
      ]);
    }
    prevStreamingRef.current = isStreaming;
  }, [isStreaming, backendLatency, clientMetrics.timeToFirstChunk]);

  // Memoize tool time calculation
  const totalToolTime = useMemo(
    () => (backendLatency ? calculateWallClockToolTime(backendLatency.toolTimings) : 0),
    [backendLatency]
  );

  const quality = getLatencyQuality(clientMetrics.timeToFirstChunk);
  const displayDuration = isStreaming ? elapsed : (backendLatency?.totalDurationMs ?? null);
  const toolCount = backendLatency?.toolTimings.length ?? 0;

  // Session averages
  const averages = useMemo(() => calculateAverages(history), [history]);

  const positionClasses = {
    "top-right": "top-4 right-4",
    "bottom-right": "bottom-20 right-4",
    "bottom-left": "bottom-20 left-4",
  };

  return (
    <div
      className={cn(
        "fixed z-50 bg-popover/95 backdrop-blur border border-border rounded-lg shadow-lg min-w-[320px] max-w-[400px]",
        positionClasses[position],
        className
      )}
    >
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-border/50 bg-muted/30 rounded-t-lg">
        <div className="flex items-center gap-2">
          <svg
            className={cn(
              "w-4 h-4",
              isStreaming ? "animate-pulse text-[var(--dot-blue)]" : qualityColors[quality]
            )}
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            aria-hidden="true"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M13 10V3L4 14h7v7l9-11h-7z"
            />
          </svg>
          <span className="text-sm font-medium text-foreground">Performance</span>
          {isStreaming && (
            <span className="text-xs bg-[var(--accent-blue)] text-[var(--dot-blue)] px-1.5 py-0.5 rounded animate-pulse">
              Live
            </span>
          )}
        </div>
        <div className="flex items-center gap-1">
          {!isStreaming && backendLatency && (
            <button
              type="button"
              onClick={() => {
                const report = {
                  timestamp: new Date().toISOString(),
                  client: {
                    ttfcMs: clientMetrics.timeToFirstChunk,
                    ttftMs: clientMetrics.timeToFirstToken,
                    chunkCount: clientMetrics.chunkCount,
                  },
                  backend: {
                    totalDurationMs: backendLatency.totalDurationMs,
                    ttftMs: backendLatency.ttftMs ?? null,
                    firstTextMs: backendLatency.firstTextMs ?? null,
                    phases: backendLatency.phases,
                    tools: backendLatency.toolTimings ?? [],
                    tokenMetrics: backendLatency.tokenMetrics ?? null,
                    kvMetrics: backendLatency.kvMetrics ?? null,
                  },
                  cost: costSummary ?? null,
                };
                const blob = new Blob([JSON.stringify(report, null, 2)], {
                  type: "application/json",
                });
                const url = URL.createObjectURL(blob);
                const a = document.createElement("a");
                a.href = url;
                a.download = `latency-${Date.now()}.json`;
                a.click();
                URL.revokeObjectURL(url);
              }}
              className="p-1 hover:bg-muted rounded transition-colors"
              aria-label="Export metrics as JSON"
              title="Export metrics as JSON"
            >
              <svg
                className="w-4 h-4 text-muted-foreground"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                aria-hidden="true"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M12 10v6m0 0l-3-3m3 3l3-3m2 8H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"
                />
              </svg>
            </button>
          )}
          <button
            type="button"
            onClick={handleToggleExpand}
            className="p-1 hover:bg-muted rounded transition-colors"
            aria-label={isExpanded ? "Collapse" : "Expand"}
          >
            <svg
              className={cn(
                "w-4 h-4 text-muted-foreground transition-transform",
                isExpanded && "rotate-180"
              )}
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              aria-hidden="true"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M19 9l-7 7-7-7"
              />
            </svg>
          </button>
          {onClose && (
            <button
              type="button"
              onClick={onClose}
              className="p-1 hover:bg-muted rounded transition-colors"
              aria-label="Close"
            >
              <svg
                className="w-4 h-4 text-muted-foreground"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                aria-hidden="true"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M6 18L18 6M6 6l12 12"
                />
              </svg>
            </button>
          )}
        </div>
      </div>

      {/* Content */}
      {isExpanded && (
        <div className="p-3 space-y-3">
          {/* Duration Banner */}
          {displayDuration !== null && (
            <div
              className={cn(
                "border rounded-lg p-3",
                isStreaming
                  ? "bg-[var(--accent-blue)] border-[var(--dot-blue)]/30"
                  : "bg-gradient-to-r from-[var(--dot-emerald)]/20 to-[var(--dot-blue)]/20 border-[var(--dot-emerald)]/30"
              )}
            >
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <svg
                    className={cn(
                      "w-5 h-5",
                      isStreaming
                        ? "text-[var(--dot-blue)] animate-pulse"
                        : "text-[var(--dot-emerald)]"
                    )}
                    fill="none"
                    viewBox="0 0 24 24"
                    stroke="currentColor"
                    aria-hidden="true"
                  >
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
                    />
                  </svg>
                  <span className="font-medium">{isStreaming ? "Elapsed" : "Total Duration"}</span>
                </div>
                <span
                  className={cn(
                    "text-2xl font-bold font-mono",
                    isStreaming && "text-[var(--dot-blue)]"
                  )}
                >
                  {formatMs(displayDuration)}
                </span>
              </div>
              {!isStreaming && backendLatency && (
                <div className="mt-2 text-xs text-muted-foreground">
                  {toolCount} tool{toolCount !== 1 ? "s" : ""} &bull;{" "}
                  {backendLatency.phases.llmCalls} LLM call
                  {backendLatency.phases.llmCalls !== 1 ? "s" : ""} &bull;{" "}
                  {clientMetrics.chunkCount} chunks
                </div>
              )}
            </div>
          )}

          {/* Core Metrics */}
          <div className="space-y-2">
            <div className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
              Timing
            </div>
            <div className="grid grid-cols-3 gap-2 text-sm">
              <MetricCard
                label="First Chunk"
                value={formatMs(clientMetrics.timeToFirstChunk)}
                sublabel="TTFC (client)"
                quality={quality}
                isActive={isStreaming && clientMetrics.timeToFirstChunk === null}
              />
              <MetricCard
                label="First Token"
                value={formatMs(clientMetrics.timeToFirstToken)}
                sublabel="TTFT (client)"
                isActive={isStreaming && clientMetrics.timeToFirstToken === null}
              />
              <MetricCard
                label="Tool Time"
                value={formatMs(totalToolTime || null)}
                sublabel={
                  backendLatency?.totalDurationMs && totalToolTime
                    ? `${Math.round((totalToolTime / backendLatency.totalDurationMs) * 100)}%`
                    : undefined
                }
                isActive={isStreaming && toolCount > 0}
              />
            </div>
            {/* Backend TTFT metrics (authoritative) */}
            {backendLatency &&
              (backendLatency.ttftMs != null || backendLatency.firstTextMs != null) &&
              !isStreaming && (
                <div className="grid grid-cols-2 gap-2 text-sm mt-2">
                  {backendLatency.ttftMs != null && (
                    <MetricCard
                      label="TTFT (backend)"
                      value={formatMs(backendLatency.ttftMs)}
                      sublabel="Time to first token"
                    />
                  )}
                  {backendLatency.firstTextMs != null && (
                    <MetricCard
                      label="First Text (backend)"
                      value={formatMs(backendLatency.firstTextMs)}
                      sublabel="Time to first text delta"
                    />
                  )}
                </div>
              )}
          </div>

          {/* Phase Breakdown */}
          {backendLatency && !isStreaming && (
            <div className="space-y-2">
              <div className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
                Phase Breakdown
              </div>
              <PhaseBar
                llmTimeMs={backendLatency.phases.llmTimeMs}
                toolTimeMs={backendLatency.phases.toolTimeMs}
                totalDurationMs={backendLatency.totalDurationMs}
              />
            </div>
          )}

          {/* Tool Details */}
          {toolCount > 0 && backendLatency && !isStreaming && (
            <div className="space-y-2">
              <button
                type="button"
                onClick={handleToggleToolDetails}
                className="flex items-center gap-2 text-xs font-medium text-muted-foreground uppercase tracking-wider hover:text-foreground transition-colors w-full"
              >
                <svg
                  className="w-3 h-3"
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  aria-hidden="true"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
                  />
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
                  />
                </svg>
                <span>Tools ({toolCount})</span>
                <svg
                  className={cn(
                    "w-3 h-3 ml-auto transition-transform",
                    showToolDetails && "rotate-180"
                  )}
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  aria-hidden="true"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M19 9l-7 7-7-7"
                  />
                </svg>
              </button>

              {showToolDetails && (
                <div className="max-h-48 overflow-y-auto scroll-container divide-y divide-border">
                  {[...backendLatency.toolTimings]
                    .sort((a, b) => b.durationMs - a.durationMs)
                    .map((tool) => (
                      <ToolRow
                        key={tool.toolCallId}
                        toolName={tool.toolName}
                        durationMs={tool.durationMs}
                        status={tool.status}
                      />
                    ))}
                </div>
              )}
            </div>
          )}

          {/* KV Metrics (Plan Persistence) */}
          {backendLatency?.kvMetrics &&
            !isKVMetricsZero(backendLatency.kvMetrics) &&
            !isStreaming && (
              <div className="space-y-2 pt-2 border-t border-border/50">
                <div className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
                  Plan Persistence
                </div>
                <div className="grid grid-cols-2 gap-2">
                  <MetricCard
                    label="Written"
                    value={formatBytes(backendLatency.kvMetrics.bytesWritten)}
                    sublabel={`${backendLatency.kvMetrics.putCount} put${backendLatency.kvMetrics.putCount !== 1 ? "s" : ""}`}
                  />
                  <MetricCard
                    label="Read"
                    value={formatBytes(backendLatency.kvMetrics.bytesRead)}
                    sublabel={`${backendLatency.kvMetrics.getCount} get${backendLatency.kvMetrics.getCount !== 1 ? "s" : ""}`}
                  />
                </div>
                <div className="text-[10px] text-muted-foreground/60">
                  KV duration: {formatMs(backendLatency.kvMetrics.kvDurationMs)}
                </div>
              </div>
            )}

          {/* Token Metrics (Backend Reported) */}
          {backendLatency?.tokenMetrics &&
            !isTokenMetricsZero(backendLatency.tokenMetrics) &&
            !isStreaming && (
              <div className="space-y-2 pt-2 border-t border-border/50">
                <div className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
                  Tokens (Backend)
                </div>
                <div className="grid grid-cols-3 gap-2">
                  <MetricCard
                    label="Input"
                    value={formatTokens(backendLatency.tokenMetrics.inputTokens)}
                  />
                  <MetricCard
                    label="Output"
                    value={formatTokens(backendLatency.tokenMetrics.outputTokens)}
                  />
                  <MetricCard
                    label="Cached"
                    value={formatTokens(backendLatency.tokenMetrics.cachedTokens)}
                    sublabel={(() => {
                      const rate = tokenCacheHitRatePercent(backendLatency.tokenMetrics);
                      return rate !== null ? `${rate.toFixed(1)}%` : "-";
                    })()}
                  />
                </div>
                {backendLatency.tokenMetrics.cacheCreationTokens > 0 && (
                  <div className="text-[10px] text-muted-foreground/70">
                    {formatTokens(backendLatency.tokenMetrics.cacheCreationTokens)} cache write
                    {backendLatency.tokenMetrics.cacheCreationTokens === 1 ? "" : "s"}
                  </div>
                )}
              </div>
            )}

          {/* Throughput & Performance Derived Metrics */}
          {backendLatency && !isStreaming && (
            <div className="space-y-2 pt-2 border-t border-border/50">
              <div className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
                Throughput
              </div>
              <div className="grid grid-cols-2 gap-2">
                <MetricCard
                  label="Input tok/s"
                  value={formatThroughput(inputTokensPerSecond(backendLatency))}
                  sublabel="Prompt processing"
                />
                <MetricCard
                  label="Output tok/s"
                  value={formatThroughput(outputTokensPerSecond(backendLatency))}
                  sublabel="Generation speed"
                />
              </div>
              <div className="grid grid-cols-3 gap-2 mt-2">
                <MetricCard
                  label="Total tok/s"
                  value={formatThroughput(tokensPerSecond(backendLatency))}
                  sublabel="Combined"
                />
                <MetricCard
                  label="Avg LLM"
                  value={formatMs(avgLLMLatencyMs(backendLatency))}
                  sublabel={`${backendLatency.phases.llmCalls} call${backendLatency.phases.llmCalls !== 1 ? "s" : ""}`}
                />
                <MetricCard
                  label="Overhead"
                  value={formatMs(overheadMs(backendLatency))}
                  sublabel={formatPercent(
                    overheadMs(backendLatency) / backendLatency.totalDurationMs
                  )}
                />
              </div>
              {backendLatency.toolTimings.length > 0 && (
                <div className="grid grid-cols-1 gap-2 mt-2">
                  <MetricCard
                    label="Avg Tool Latency"
                    value={formatMs(avgToolLatencyMs(backendLatency))}
                    sublabel={`${backendLatency.toolTimings.length} tool${backendLatency.toolTimings.length !== 1 ? "s" : ""} executed`}
                  />
                </div>
              )}
            </div>
          )}

          {/* Cost Summary */}
          {costSummary && !isStreaming && (
            <div className="space-y-1 pt-2 border-t border-border/50">
              <div className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
                Cost
              </div>
              <div className="flex items-center justify-between text-xs text-muted-foreground">
                <span>
                  {formatTokens(costSummary.totalPromptTokens)} in /{" "}
                  {formatTokens(costSummary.totalCompletionTokens)} out
                </span>
                <span className="font-medium">{formatTokens(costSummary.totalTokens)} total</span>
              </div>
              {/* Cache hit rate */}
              {costCacheHitRatePercent(costSummary) !== null && (
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span>{formatTokens(costSummary.totalCacheReadInputTokens ?? 0)} cached</span>
                  <span className="text-[var(--dot-emerald)] font-medium">
                    {costCacheHitRatePercent(costSummary)?.toFixed(1)}% cache hit
                  </span>
                </div>
              )}
              {(costSummary.totalCacheCreationInputTokens ?? 0) > 0 && (
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span>
                    {formatTokens(costSummary.totalCacheCreationInputTokens ?? 0)} cache writes
                  </span>
                  <span className="font-medium text-[var(--dot-blue)]">prefix population</span>
                </div>
              )}
              {/* Per-agent breakdown */}
              {costSummary.agents.length > 0 && (
                <div className="space-y-0.5 pt-1">
                  {costSummary.agents.map((agent, i) => (
                    <div
                      key={`${agent.agentName}-${i}`}
                      className="flex items-center justify-between text-[10px] text-muted-foreground/60"
                    >
                      <span className="flex items-center gap-1">
                        <span className="w-1.5 h-1.5 rounded-full bg-[var(--dot-blue)] inline-block" />
                        {agent.agentName}
                        <span className="text-muted-foreground/30">
                          {agent.model.split("/").pop()}
                        </span>
                      </span>
                      <span className="font-mono">
                        {formatTokens(agent.usage.promptTokens)} /{" "}
                        {formatTokens(agent.usage.completionTokens)}
                      </span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}

          {/* Finish Reason (non-normal termination) */}
          {finishReason && finishReason !== "stop" && !isStreaming && (
            <div className="flex items-center gap-2 text-xs pt-1">
              <span
                className={cn(
                  "inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-medium",
                  finishReason === "length" && "bg-[var(--accent-amber)] text-[var(--dot-amber)]",
                  finishReason === "content-filter" &&
                    "bg-[var(--accent-red)] text-[var(--dot-red)]",
                  finishReason === "tool-calls" && "bg-[var(--accent-blue)] text-[var(--dot-blue)]"
                )}
              >
                {finishReason === "length" && "Context limit reached"}
                {finishReason === "content-filter" && "Content filtered"}
                {finishReason === "tool-calls" && "Ended at tool call"}
              </span>
            </div>
          )}

          {/* Session History */}
          {history.length > 1 && (
            <div className="pt-2 border-t border-border/50">
              <div className="text-xs font-medium text-muted-foreground uppercase tracking-wider mb-2">
                Session ({history.length} requests)
              </div>
              <div className="grid grid-cols-2 gap-2 text-xs">
                <div className="bg-muted/50 rounded p-2">
                  <div className="text-muted-foreground/60">Avg TTFC</div>
                  <div className="font-mono font-medium">{formatMs(averages.avgTtfc)}</div>
                </div>
                <div className="bg-muted/50 rounded p-2">
                  <div className="text-muted-foreground/60">Avg Total</div>
                  <div className="font-mono font-medium">{formatMs(averages.avgDuration)}</div>
                </div>
              </div>
            </div>
          )}

          {/* Warnings & Retry Breakdown */}
          {backendLatency &&
            (backendLatency.retryCount > 0 ||
              backendLatency.hadTimeout ||
              (backendLatency.retryEvents && backendLatency.retryEvents.length > 0)) && (
              <div className="space-y-2">
                {/* Summary warning */}
                <div className="flex items-center gap-2 text-xs text-[var(--dot-amber)] bg-[var(--accent-amber)] rounded px-2 py-1">
                  <svg
                    className="w-4 h-4"
                    fill="none"
                    viewBox="0 0 24 24"
                    stroke="currentColor"
                    aria-hidden="true"
                  >
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
                    />
                  </svg>
                  <span>
                    {backendLatency.retryCount > 0 && `${backendLatency.retryCount} retries`}
                    {backendLatency.retryCount > 0 && backendLatency.hadTimeout && " • "}
                    {backendLatency.hadTimeout && "Timeout occurred"}
                  </span>
                </div>

                {/* Categorized retry breakdown */}
                {backendLatency.retryEvents && backendLatency.retryEvents.length > 0 && (
                  <div className="space-y-1">
                    <div className="text-[10px] text-muted-foreground uppercase tracking-wider">
                      Retry Breakdown
                    </div>
                    <div className="flex flex-wrap gap-1">
                      {Array.from(countRetrysByReason(backendLatency.retryEvents)).map(
                        ([reason, count]) => (
                          <span
                            key={reason}
                            className={cn(
                              "text-[10px] px-1.5 py-0.5 rounded",
                              getRetryReasonColor(reason)
                            )}
                          >
                            {formatRetryReason(reason)}: {count}
                          </span>
                        )
                      )}
                    </div>
                  </div>
                )}
              </div>
            )}
        </div>
      )}
    </div>
  );
});
