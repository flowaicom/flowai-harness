/**
 * Trace collection hook for automatic latency reporting.
 *
 * Effects at the edges. This hook observes backend latency
 * from the store and automatically collects traces for aggregated reporting.
 *
 * @module lib/hooks/use-trace-collection
 */

import { useEffect, useRef } from "react";
import { backendLatencyToTrace, type ClientTimingMetrics, getTraceCollector } from "~/lib/perf";
import { scheduleIdle } from "~/lib/perf/scheduler";
import {
  selectLatency,
  selectServerMetrics,
  selectStreamPhase,
  useConversation,
} from "~/lib/stores";

/**
 * Hook options.
 */
export interface UseTraceCollectionOptions {
  /** Enable/disable trace collection (default: true) */
  enabled?: boolean;
  /** Log traces to console in dev mode (default: true) */
  logInDev?: boolean;
}

/**
 * Automatically collect traces from backend latency data.
 *
 * This hook:
 * 1. Observes backend latency from the chat store
 * 2. Combines with client-side timing metrics
 * 3. Converts to RequestTrace via bridge
 * 4. Adds to global TraceCollector for aggregated reporting
 *
 * @param options - Configuration options
 */
export function useTraceCollection(options: UseTraceCollectionOptions = {}): void {
  const { enabled = true, logInDev = true } = options;

  // Subscribe to relevant state
  const backendLatency = useConversation(selectServerMetrics);
  const clientLatency = useConversation(selectLatency);
  const streamingState = useConversation(selectStreamPhase);

  // Track whether we've collected for the current stream
  const lastCollectedAtRef = useRef<number | null>(null);

  useEffect(() => {
    if (!enabled) return;

    // Only collect when:
    // 1. We have backend latency data
    // 2. Streaming is complete (not during or idle)
    // 3. We haven't already collected for this stream
    if (backendLatency === null || streamingState.phase !== "complete") {
      return;
    }

    // Check if we've already collected for this completion
    const currentFinishedAt = streamingState.phase === "complete" ? streamingState.finishedAt : 0;
    if (lastCollectedAtRef.current === currentFinishedAt) {
      return;
    }

    // Mark as collected
    lastCollectedAtRef.current = currentFinishedAt;

    // Build client metrics from store latency
    // Note: Store tracks Date.now() based timing, but TraceCollector expects
    // performance.now() relative values. We approximate by using relative durations.
    const clientMetrics: ClientTimingMetrics | undefined =
      clientLatency.timeToFirstChunk !== null
        ? {
            timeToFirstChunk: clientLatency.timeToFirstChunk,
            streamingDuration:
              clientLatency.totalDuration !== null && clientLatency.timeToFirstChunk !== null
                ? clientLatency.totalDuration - clientLatency.timeToFirstChunk
                : undefined,
          }
        : undefined;

    // Convert backend latency to trace (bridge pattern)
    // Use LatencySummary-compatible object from store
    const trace = backendLatencyToTrace(
      {
        totalDurationMs: backendLatency.totalDurationMs,
        phases: backendLatency.phases,
        toolTimings: backendLatency.toolTimings,
        productSetSize: backendLatency.productSetSize,
        planPayloadBytes: backendLatency.planPayloadBytes,
        retryCount: backendLatency.retryCount,
        hadTimeout: backendLatency.hadTimeout,
      },
      clientMetrics
    );

    // Add to collector (deferred to idle time)
    scheduleIdle(
      "trace-collection",
      () => {
        const collector = getTraceCollector();
        collector.addTrace(trace);

        if (logInDev && import.meta.env.DEV) {
          console.log("%c[TraceCollection] Trace collected", "color: #8b5cf6; font-weight: bold", {
            traceId: trace.traceId.slice(0, 8),
            totalDuration: `${trace.totalDuration.toFixed(0)}ms`,
            tools: trace.toolRecords.length,
            ttfc: clientMetrics?.timeToFirstChunk
              ? `${clientMetrics.timeToFirstChunk.toFixed(0)}ms`
              : "N/A",
            collectorCount: collector.count,
          });
        }
      },
      "low"
    );
  }, [backendLatency, clientLatency, streamingState, enabled, logInDev]);
}

/**
 * Reset trace collection state.
 * Call this when starting a new stream to allow collection for the next completion.
 */
export function useResetTraceCollection(): () => void {
  const lastCollectedAtRef = useRef<number | null>(null);

  return () => {
    lastCollectedAtRef.current = null;
  };
}
