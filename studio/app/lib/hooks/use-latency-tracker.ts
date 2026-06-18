/**
 * Latency tracking hook for chat performance metrics.
 *
 * Tracks:
 * - Time to first chunk (TTFC)
 * - Total response duration
 * - Chunk count and rate
 *
 * @module lib/hooks/use-latency-tracker
 */

import { useCallback, useRef } from "react";
import { scheduleIdle } from "~/lib/perf/scheduler";

export interface LatencyMetrics {
  /** Time from submit to first chunk (ms) */
  timeToFirstChunk: number | null;
  /** Time from submit to first text token (ms) — distinct from TTFC which includes tool chunks */
  timeToFirstToken: number | null;
  /** Total response duration (ms) */
  totalDuration: number | null;
  /** Number of chunks received */
  chunkCount: number;
  /** Chunks per second */
  chunkRate: number | null;
  /** Timestamp of submit */
  submitTime: number | null;
  /** Timestamp of first chunk */
  firstChunkTime: number | null;
  /** Timestamp of first text token */
  firstTokenTime: number | null;
  /** Timestamp of completion */
  completeTime: number | null;
}

export interface LatencyTracker {
  /** Mark the start of a request (user submit) */
  trackSubmit: () => void;
  /** Mark receipt of first chunk */
  trackFirstChunk: () => void;
  /** Mark receipt of first text token (not tool invocations) */
  trackFirstToken: () => void;
  /** Mark receipt of a chunk */
  trackChunk: () => void;
  /** Mark completion of response */
  trackComplete: () => void;
  /** Reset all metrics */
  reset: () => void;
  /** Get current metrics */
  getMetrics: () => LatencyMetrics;
}

export function useLatencyTracker(): LatencyTracker {
  const submitTimeRef = useRef<number | null>(null);
  const firstChunkTimeRef = useRef<number | null>(null);
  const firstTokenTimeRef = useRef<number | null>(null);
  const completeTimeRef = useRef<number | null>(null);
  const chunkCountRef = useRef(0);

  const trackSubmit = useCallback(() => {
    submitTimeRef.current = performance.now();
    firstChunkTimeRef.current = null;
    firstTokenTimeRef.current = null;
    completeTimeRef.current = null;
    chunkCountRef.current = 0;
  }, []);

  const trackFirstChunk = useCallback(() => {
    if (firstChunkTimeRef.current === null) {
      firstChunkTimeRef.current = performance.now();
      chunkCountRef.current = 1;
    }
  }, []);

  const trackFirstToken = useCallback(() => {
    if (firstTokenTimeRef.current === null) {
      firstTokenTimeRef.current = performance.now();
    }
  }, []);

  const trackChunk = useCallback(() => {
    chunkCountRef.current++;
    // Track first chunk if not already tracked
    if (firstChunkTimeRef.current === null) {
      firstChunkTimeRef.current = performance.now();
    }
  }, []);

  // biome-ignore lint/correctness/useExhaustiveDependencies: getMetrics uses refs only, stable
  const trackComplete = useCallback(() => {
    completeTimeRef.current = performance.now();

    // Defer logging to idle time (P2 fix)
    scheduleIdle(
      "latency-log",
      () => {
        const metrics = getMetrics();
        if (import.meta.env.DEV) {
          console.log("%c[Latency] Request Complete", "color: #22c55e; font-weight: bold", {
            timeToFirstChunk: metrics.timeToFirstChunk
              ? `${metrics.timeToFirstChunk.toFixed(0)}ms`
              : "N/A",
            totalDuration: metrics.totalDuration ? `${metrics.totalDuration.toFixed(0)}ms` : "N/A",
            chunkCount: metrics.chunkCount,
            chunkRate: metrics.chunkRate ? `${metrics.chunkRate.toFixed(1)}/s` : "N/A",
          });
        }
      },
      "low"
    );
  }, []);

  const reset = useCallback(() => {
    submitTimeRef.current = null;
    firstChunkTimeRef.current = null;
    firstTokenTimeRef.current = null;
    completeTimeRef.current = null;
    chunkCountRef.current = 0;
  }, []);

  const getMetrics = useCallback((): LatencyMetrics => {
    const submitTime = submitTimeRef.current;
    const firstChunkTime = firstChunkTimeRef.current;
    const firstTokenTime = firstTokenTimeRef.current;
    const completeTime = completeTimeRef.current;
    const chunkCount = chunkCountRef.current;

    const timeToFirstChunk =
      submitTime !== null && firstChunkTime !== null ? firstChunkTime - submitTime : null;

    const timeToFirstToken =
      submitTime !== null && firstTokenTime !== null ? firstTokenTime - submitTime : null;

    const totalDuration =
      submitTime !== null && completeTime !== null ? completeTime - submitTime : null;

    const chunkRate =
      totalDuration !== null && totalDuration > 0 ? (chunkCount / totalDuration) * 1000 : null;

    return {
      timeToFirstChunk,
      timeToFirstToken,
      totalDuration,
      chunkCount,
      chunkRate,
      submitTime,
      firstChunkTime,
      firstTokenTime,
      completeTime,
    };
  }, []);

  return {
    trackSubmit,
    trackFirstChunk,
    trackFirstToken,
    trackChunk,
    trackComplete,
    reset,
    getMetrics,
  };
}
