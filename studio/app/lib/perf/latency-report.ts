/**
 * Latency report infrastructure for benchmarking and analysis.
 *
 * Structured latency reporting for scenario agent benchmarks.
 *
 * Design Principles:
 * - Immutable data structures for report snapshots
 * - Pure functions for aggregation and percentile calculation
 * - Effects isolated to collectors (at the edges)
 * - Type-safe discriminated unions for phases
 *
 * @module lib/perf/latency-report
 */

// ============================================================================
// Configuration
// ============================================================================

/** Default maximum samples to keep per category */
const DEFAULT_MAX_SAMPLES = 100;

// ============================================================================
// Core Types
// ============================================================================

/**
 * Phase timing within a single request.
 *
 * Phases are mutually exclusive time slices of a request:
 * - waiting: network + model thinking (before first chunk)
 * - streaming: receiving text/tool tokens
 * - toolExecution: time spent executing tools (wall-clock)
 * - llmThinking: time between tool results and next output
 */
export interface PhaseTimings {
  /** Time before first chunk (network + initial processing) */
  readonly waiting: number | null;
  /** Time receiving streaming content */
  readonly streaming: number | null;
  /** Wall-clock time in tool execution */
  readonly toolExecution: number | null;
  /** Time in LLM thinking (between outputs) */
  readonly llmThinking: number | null;
}

/**
 * Single tool timing record for breakdown analysis.
 */
export interface ToolRecord {
  readonly toolName: string;
  readonly toolCallId: string;
  readonly duration: number;
  readonly status: "completed" | "error";
  readonly payloadSizeBytes?: number;
}

/**
 * Token counts for a single LLM call.
 */
export interface TokenCounts {
  readonly inputTokens: number;
  readonly outputTokens: number;
  readonly cachedTokens: number;
  readonly cacheCreationTokens: number;
}

/**
 * Complete trace for a single request.
 */
export interface RequestTrace {
  readonly traceId: string;
  readonly startedAt: number;
  readonly completedAt: number;

  // Timing breakdown
  readonly totalDuration: number;
  readonly phases: PhaseTimings;

  // Tool metrics
  readonly toolRecords: readonly ToolRecord[];
  readonly totalToolTime: number;
  readonly wallClockToolTime: number;

  // Payload metrics
  readonly requestPayloadBytes?: number;
  readonly responsePayloadBytes?: number;

  // Token metrics (if available)
  readonly tokens?: TokenCounts;

  // Error tracking
  readonly retryCount: number;
  readonly hadTimeout: boolean;
  readonly errorMessage?: string;

  // Agent-specific (optional)
  readonly productSetSize?: number;
  readonly planPayloadBytes?: number;
}

/**
 * Percentile statistics for a metric.
 */
export interface PercentileStats {
  readonly min: number;
  readonly max: number;
  readonly mean: number;
  readonly p50: number;
  readonly p95: number;
  readonly p99: number;
  readonly sampleCount: number;
}

/**
 * Tool breakdown for aggregation.
 */
export interface ToolBreakdown {
  readonly toolName: string;
  readonly callCount: number;
  readonly totalDuration: number;
  readonly avgDuration: number;
  readonly p50Duration: number;
  readonly p95Duration: number;
  readonly errorCount: number;
  readonly errorRate: number;
}

/**
 * Aggregated latency report across multiple traces.
 */
export interface LatencyReport {
  readonly generatedAt: number;
  readonly traceCount: number;

  // Overall latency stats
  readonly totalRequest: PercentileStats;
  readonly timeToFirstChunk: PercentileStats;

  // Phase breakdown stats
  readonly phases: {
    readonly waiting: PercentileStats | null;
    readonly streaming: PercentileStats | null;
    readonly toolExecution: PercentileStats | null;
    readonly llmThinking: PercentileStats | null;
  };

  // Tool breakdown (sorted by total time)
  readonly toolBreakdown: readonly ToolBreakdown[];

  // Token stats (if available)
  readonly tokenStats?: {
    readonly input: PercentileStats;
    readonly output: PercentileStats;
    readonly cached: PercentileStats;
    readonly cacheCreation: PercentileStats;
  };

  // Error summary
  readonly errorRate: number;
  readonly timeoutRate: number;
  readonly avgRetries: number;
}

// ============================================================================
// Pure Functions: Percentile Calculation
// ============================================================================

/**
 * Calculate percentile from sorted array.
 * Uses linear interpolation for accuracy.
 */
function percentile(sorted: readonly number[], p: number): number {
  if (sorted.length === 0) return 0;
  if (sorted.length === 1) return sorted[0];

  const index = (p / 100) * (sorted.length - 1);
  const lower = Math.floor(index);
  const upper = Math.ceil(index);
  const fraction = index - lower;

  if (lower === upper) return sorted[lower];
  return sorted[lower] * (1 - fraction) + sorted[upper] * fraction;
}

/**
 * Calculate full percentile stats from raw samples.
 */
export function calculatePercentileStats(samples: readonly number[]): PercentileStats | null {
  if (samples.length === 0) return null;

  const sorted = [...samples].sort((a, b) => a - b);
  const sum = samples.reduce((acc, v) => acc + v, 0);

  return {
    min: sorted[0],
    max: sorted[sorted.length - 1],
    mean: sum / samples.length,
    p50: percentile(sorted, 50),
    p95: percentile(sorted, 95),
    p99: percentile(sorted, 99),
    sampleCount: samples.length,
  };
}

// ============================================================================
// Pure Functions: Tool Breakdown
// ============================================================================

/**
 * Aggregate tool records into breakdown statistics.
 */
export function aggregateToolBreakdown(records: readonly ToolRecord[]): readonly ToolBreakdown[] {
  if (records.length === 0) return [];

  // Group by tool name
  const byName = new Map<string, ToolRecord[]>();
  for (const record of records) {
    const existing = byName.get(record.toolName);
    if (existing) {
      existing.push(record);
    } else {
      byName.set(record.toolName, [record]);
    }
  }

  // Calculate stats per tool
  const breakdowns: ToolBreakdown[] = [];

  for (const [toolName, toolRecords] of byName) {
    const durations = toolRecords.map((r) => r.duration);
    const sorted = [...durations].sort((a, b) => a - b);
    const totalDuration = durations.reduce((acc, d) => acc + d, 0);
    const errorCount = toolRecords.filter((r) => r.status === "error").length;

    breakdowns.push({
      toolName,
      callCount: toolRecords.length,
      totalDuration,
      avgDuration: totalDuration / toolRecords.length,
      p50Duration: percentile(sorted, 50),
      p95Duration: percentile(sorted, 95),
      errorCount,
      errorRate: errorCount / toolRecords.length,
    });
  }

  // Sort by total duration descending
  return breakdowns.sort((a, b) => b.totalDuration - a.totalDuration);
}

// ============================================================================
// Pure Functions: Report Generation
// ============================================================================

/**
 * Generate a latency report from traces.
 */
export function generateLatencyReport(traces: readonly RequestTrace[]): LatencyReport | null {
  if (traces.length === 0) return null;

  // Extract metrics arrays
  const totalDurations = traces.map((t) => t.totalDuration);
  const ttfcValues = traces.map((t) => t.phases.waiting).filter((v): v is number => v !== null);
  const waitingValues = traces.map((t) => t.phases.waiting).filter((v): v is number => v !== null);
  const streamingValues = traces
    .map((t) => t.phases.streaming)
    .filter((v): v is number => v !== null);
  const toolExecValues = traces
    .map((t) => t.phases.toolExecution)
    .filter((v): v is number => v !== null);
  const llmThinkingValues = traces
    .map((t) => t.phases.llmThinking)
    .filter((v): v is number => v !== null);

  // Aggregate all tool records
  const allToolRecords = traces.flatMap((t) => t.toolRecords);

  // Calculate error/timeout stats
  const errorCount = traces.filter((t) => t.errorMessage).length;
  const timeoutCount = traces.filter((t) => t.hadTimeout).length;
  const totalRetries = traces.reduce((sum, t) => sum + t.retryCount, 0);

  // Token stats if available
  const tracesWithTokens = traces.filter(
    (t): t is RequestTrace & { tokens: TokenCounts } => t.tokens !== undefined
  );

  // Build token stats only if we have data
  let tokenStats:
    | {
        readonly input: PercentileStats;
        readonly output: PercentileStats;
        readonly cached: PercentileStats;
        readonly cacheCreation: PercentileStats;
      }
    | undefined;

  if (tracesWithTokens.length > 0) {
    const inputStats = calculatePercentileStats(tracesWithTokens.map((t) => t.tokens.inputTokens));
    const outputStats = calculatePercentileStats(
      tracesWithTokens.map((t) => t.tokens.outputTokens)
    );
    const cachedStats = calculatePercentileStats(
      tracesWithTokens.map((t) => t.tokens.cachedTokens)
    );
    const cacheCreationStats = calculatePercentileStats(
      tracesWithTokens.map((t) => t.tokens.cacheCreationTokens)
    );

    // Only set if all stats are valid (they always will be if we have data)
    if (inputStats && outputStats && cachedStats && cacheCreationStats) {
      tokenStats = {
        input: inputStats,
        output: outputStats,
        cached: cachedStats,
        cacheCreation: cacheCreationStats,
      };
    }
  }

  // Calculate total request stats (guaranteed to exist since traces.length > 0)
  const totalRequestStats = calculatePercentileStats(totalDurations) ?? {
    min: 0,
    max: 0,
    mean: 0,
    p50: 0,
    p95: 0,
    p99: 0,
    sampleCount: 0,
  };

  return {
    generatedAt: Date.now(),
    traceCount: traces.length,

    totalRequest: totalRequestStats,
    timeToFirstChunk: calculatePercentileStats(ttfcValues) ?? {
      min: 0,
      max: 0,
      mean: 0,
      p50: 0,
      p95: 0,
      p99: 0,
      sampleCount: 0,
    },

    phases: {
      waiting: calculatePercentileStats(waitingValues),
      streaming: calculatePercentileStats(streamingValues),
      toolExecution: calculatePercentileStats(toolExecValues),
      llmThinking: calculatePercentileStats(llmThinkingValues),
    },

    toolBreakdown: aggregateToolBreakdown(allToolRecords),

    tokenStats,

    errorRate: errorCount / traces.length,
    timeoutRate: timeoutCount / traces.length,
    avgRetries: totalRetries / traces.length,
  };
}

// ============================================================================
// Stateful Collector (Effect at the Edge)
// ============================================================================

/**
 * Trace collector for accumulating benchmarks.
 *
 * This is the only stateful part - isolated at the edge.
 * Uses ringbuffer semantics to bound memory.
 */
export class TraceCollector {
  private traces: RequestTrace[] = [];
  private readonly maxSamples: number;

  constructor(maxSamples: number = DEFAULT_MAX_SAMPLES) {
    this.maxSamples = maxSamples;
  }

  /**
   * Add a trace to the collector.
   */
  addTrace(trace: RequestTrace): void {
    this.traces.push(trace);

    // Evict oldest if over capacity
    if (this.traces.length > this.maxSamples) {
      this.traces.shift();
    }
  }

  /**
   * Get all traces (readonly snapshot).
   */
  getTraces(): readonly RequestTrace[] {
    return [...this.traces];
  }

  /**
   * Generate report from collected traces.
   */
  generateReport(): LatencyReport | null {
    return generateLatencyReport(this.traces);
  }

  /**
   * Clear all traces.
   */
  clear(): void {
    this.traces = [];
  }

  /**
   * Get trace count.
   */
  get count(): number {
    return this.traces.length;
  }
}

// ============================================================================
// Singleton Instance
// ============================================================================

let globalCollector: TraceCollector | null = null;

/**
 * Get the global trace collector instance.
 */
export function getTraceCollector(): TraceCollector {
  if (!globalCollector) {
    globalCollector = new TraceCollector();
  }
  return globalCollector;
}

/**
 * Reset the global trace collector.
 */
export function resetTraceCollector(): void {
  globalCollector?.clear();
}

// ============================================================================
// Format Utilities
// ============================================================================

/**
 * Format milliseconds for display.
 */
export function formatMs(ms: number | null): string {
  if (ms === null) return "—";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

/**
 * Format percentile stats for display.
 */
export function formatPercentileStats(stats: PercentileStats | null): string {
  if (!stats || stats.sampleCount === 0) return "No data";
  return `p50: ${formatMs(stats.p50)}, p95: ${formatMs(stats.p95)}, n=${stats.sampleCount}`;
}

/**
 * Generate markdown summary table.
 */
export function generateMarkdownSummary(report: LatencyReport): string {
  const lines: string[] = [
    `## Latency Report`,
    `Generated: ${new Date(report.generatedAt).toISOString()}`,
    `Traces: ${report.traceCount}`,
    ``,
    `### Overall Latency`,
    `| Metric | p50 | p95 | Mean | Min | Max |`,
    `|--------|-----|-----|------|-----|-----|`,
    `| Total Request | ${formatMs(report.totalRequest.p50)} | ${formatMs(report.totalRequest.p95)} | ${formatMs(report.totalRequest.mean)} | ${formatMs(report.totalRequest.min)} | ${formatMs(report.totalRequest.max)} |`,
    `| Time to First Chunk | ${formatMs(report.timeToFirstChunk.p50)} | ${formatMs(report.timeToFirstChunk.p95)} | ${formatMs(report.timeToFirstChunk.mean)} | ${formatMs(report.timeToFirstChunk.min)} | ${formatMs(report.timeToFirstChunk.max)} |`,
    ``,
    `### Phase Breakdown`,
  ];

  const addPhase = (name: string, stats: PercentileStats | null) => {
    if (stats && stats.sampleCount > 0) {
      lines.push(
        `| ${name} | ${formatMs(stats.p50)} | ${formatMs(stats.p95)} | ${formatMs(stats.mean)} |`
      );
    }
  };

  lines.push(`| Phase | p50 | p95 | Mean |`);
  lines.push(`|-------|-----|-----|------|`);
  addPhase("Waiting", report.phases.waiting);
  addPhase("Streaming", report.phases.streaming);
  addPhase("Tool Execution", report.phases.toolExecution);
  addPhase("LLM Thinking", report.phases.llmThinking);

  if (report.toolBreakdown.length > 0) {
    lines.push(``);
    lines.push(`### Tool Breakdown (Top Contributors)`);
    lines.push(`| Tool | Calls | Total Time | Avg | p50 | p95 |`);
    lines.push(`|------|-------|------------|-----|-----|-----|`);

    for (const tool of report.toolBreakdown.slice(0, 10)) {
      lines.push(
        `| ${tool.toolName} | ${tool.callCount} | ${formatMs(tool.totalDuration)} | ${formatMs(tool.avgDuration)} | ${formatMs(tool.p50Duration)} | ${formatMs(tool.p95Duration)} |`
      );
    }
  }

  lines.push(``);
  lines.push(`### Error Summary`);
  lines.push(`- Error Rate: ${(report.errorRate * 100).toFixed(1)}%`);
  lines.push(`- Timeout Rate: ${(report.timeoutRate * 100).toFixed(1)}%`);
  lines.push(`- Avg Retries: ${report.avgRetries.toFixed(2)}`);

  return lines.join("\n");
}
