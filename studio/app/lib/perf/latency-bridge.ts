/**
 * Bridge between backend LatencySummary and frontend RequestTrace.
 *
 * Backend owns authoritative timing, frontend bridges
 * to trace collection for aggregated reporting.
 *
 * @module lib/perf/latency-bridge
 */

import type { LatencySummary, ToolTiming } from "~/lib/domain/stream-part";
import type { PhaseTimings, RequestTrace, ToolRecord } from "./latency-report";

// ============================================================================
// Types
// ============================================================================

/**
 * Client-side timing metrics (optional supplement to backend data).
 *
 * These are client-perspective metrics that backend cannot measure:
 * - timeToFirstChunk: includes network latency from client perspective
 */
export interface ClientTimingMetrics {
  /** Time from submit to first chunk (client perspective) */
  timeToFirstChunk?: number;
  /** Time from first chunk to completion (client perspective) */
  streamingDuration?: number;
  /** Timestamp when request started (for traceId correlation) */
  startedAt?: number;
}

// ============================================================================
// Conversion Functions (Pure)
// ============================================================================

/**
 * Convert backend ToolTiming to ToolRecord for trace collection.
 */
function toolTimingToRecord(timing: ToolTiming): ToolRecord {
  return {
    toolName: timing.toolName,
    toolCallId: timing.toolCallId,
    duration: timing.durationMs,
    status: timing.status,
    payloadSizeBytes: timing.payloadSize,
  };
}

/**
 * Convert backend LatencySummary to RequestTrace.
 *
 * Combines authoritative backend metrics with optional client-side timing.
 * Uses backend data for all timing except client-perspective metrics
 * (TTFC, streaming duration).
 *
 * @param summary - Backend-provided latency summary
 * @param clientMetrics - Optional client-side timing supplements
 * @param traceId - Optional trace ID (defaults to crypto.randomUUID)
 * @returns RequestTrace for trace collection
 */
export function backendLatencyToTrace(
  summary: LatencySummary,
  clientMetrics?: ClientTimingMetrics,
  traceId?: string
): RequestTrace {
  const now = performance.now();
  const startedAt = clientMetrics?.startedAt ?? now - summary.totalDurationMs;
  const completedAt = startedAt + summary.totalDurationMs;

  // Convert tool timings
  const toolRecords: ToolRecord[] = summary.toolTimings.map(toolTimingToRecord);

  // Calculate total tool time (sum of all durations)
  const totalToolTime = toolRecords.reduce((sum, r) => sum + r.duration, 0);

  // Backend provides wall-clock tool time in phases.toolTimeMs
  const wallClockToolTime = summary.phases.toolTimeMs;

  // Build phases from backend data + client supplements
  // Backend knows: llmTimeMs, toolTimeMs
  // Client knows: timeToFirstChunk (waiting), streamingDuration
  const phases: PhaseTimings = {
    // Client-perspective: time before first chunk
    waiting: clientMetrics?.timeToFirstChunk ?? null,
    // Client-perspective: time receiving content
    streaming: clientMetrics?.streamingDuration ?? null,
    // Backend-authoritative: wall-clock tool time
    toolExecution: wallClockToolTime > 0 ? wallClockToolTime : null,
    // Backend-authoritative: LLM API time (approximation for thinking)
    llmThinking: summary.phases.llmTimeMs > 0 ? summary.phases.llmTimeMs : null,
  };

  return {
    traceId: traceId ?? crypto.randomUUID(),
    startedAt,
    completedAt,
    totalDuration: summary.totalDurationMs,
    phases,
    toolRecords,
    totalToolTime,
    wallClockToolTime,
    retryCount: summary.retryCount,
    hadTimeout: summary.hadTimeout,
    tokens: summary.tokenMetrics
      ? {
          inputTokens: summary.tokenMetrics.inputTokens,
          outputTokens: summary.tokenMetrics.outputTokens,
          cachedTokens: summary.tokenMetrics.cachedTokens,
          cacheCreationTokens: summary.tokenMetrics.cacheCreationTokens,
        }
      : undefined,
    productSetSize: summary.productSetSize,
    planPayloadBytes: summary.planPayloadBytes,
  };
}

/**
 * Extract minimal client metrics from useLatencyTracker data.
 *
 * This bridges the simple useLatencyTracker hook to the trace bridge.
 */
export function extractClientMetrics(
  submitTime: number | null,
  firstChunkTime: number | null,
  completeTime: number | null
): ClientTimingMetrics | undefined {
  if (submitTime === null) return undefined;

  return {
    startedAt: submitTime,
    timeToFirstChunk: firstChunkTime !== null ? firstChunkTime - submitTime : undefined,
    streamingDuration:
      firstChunkTime !== null && completeTime !== null ? completeTime - firstChunkTime : undefined,
  };
}
