/**
 * AI SDK Data Stream Protocol types.
 *
 * These types match the Rust backend's StreamPart enum exactly.
 * The protocol defines 11 distinct part types for streaming LLM responses.
 *
 * Algebraic Properties:
 * - StreamPart is a closed sum type (discriminated union)
 * - Exhaustive pattern matching ensures all cases are handled
 * - Serialization is deterministic (same input → same JSON)
 *
 * @module domain/stream-part
 */

// ============================================================================
// Core Stream Part Types (Matches Rust backend)
// ============================================================================

/**
 * Text delta - incremental text token from LLM.
 */
export interface TextPart {
  readonly type: "text";
  readonly text: string;
}

/**
 * Reasoning delta - chain-of-thought (model-dependent).
 */
export interface ReasoningPart {
  readonly type: "reasoning";
  readonly text: string;
}

/**
 * Step boundary marker.
 */
export interface StepStartPart {
  readonly type: "step-start";
}

/**
 * Tool invocation state.
 */
export type ToolState = "call" | "result";

/**
 * Tool invocation lifecycle event.
 */
export interface ToolInvocationPart {
  readonly type: "tool-invocation";
  readonly toolInvocationId: string;
  readonly toolName: string;
  readonly args: unknown;
  readonly state: ToolState;
  readonly result?: unknown;
}

/**
 * Sub-agent activation event.
 */
export interface ToolAgentPart {
  readonly type: "tool-agent";
  readonly agentName: string;
  readonly toolInvocationId: string;
  readonly state: ToolState;
}

/**
 * Token usage statistics.
 */
export interface TokenUsage {
  readonly promptTokens: number;
  readonly completionTokens: number;
  readonly cacheReadInputTokens?: number;
  readonly cacheCreationInputTokens?: number;
  readonly totalTokens: number;
}

/**
 * Per-agent usage data.
 */
export interface AgentUsage {
  readonly agentName: string;
  readonly model: string;
  readonly usage: TokenUsage;
}

/**
 * Sub-agent completion with usage metrics.
 */
export interface DataToolAgentPart {
  readonly type: "data-tool-agent";
  readonly data: AgentUsage;
}

/**
 * File registration event.
 */
export interface FileRegistration {
  readonly fileId: string;
  readonly filename: string;
  readonly threadId: string;
  readonly timestamp: string;
}

/**
 * File available for download.
 */
export interface DataFileRegisteredPart {
  readonly type: "data-file-registered";
  readonly data: FileRegistration;
}

/**
 * Cost summary for the entire stream.
 */
export interface CostSummary {
  readonly agents: AgentUsage[];
  readonly totalPromptTokens: number;
  readonly totalCompletionTokens: number;
  readonly totalCacheReadInputTokens?: number;
  readonly totalCacheCreationInputTokens?: number;
  readonly totalTokens: number;
}

/**
 * Aggregated cost summary (emitted at stream end).
 */
export interface DataCostSummaryPart {
  readonly type: "data-cost-summary";
  readonly data: CostSummary;
}

/**
 * Tool progress update — monotonic phase transitions from long-running tools.
 *
 * Emitted by ProgressEmitter on the backend. Frontend accumulates these
 * into a single ToolProgressMessagePart per tool (update-in-place semantics).
 */
export interface ToolProgressPart {
  readonly type: "tool-progress";
  readonly toolName: string;
  /** Tool call ID for precise correlation (present when backend provides it). */
  readonly toolCallId?: string;
  readonly label: string;
  readonly phaseIndex: number;
  readonly totalPhases: number;
  readonly milestone?: Record<string, unknown>;
}

/**
 * Pre-computed CommandCard payload (approval DSL).
 */
export interface CommandCardPayload {
  readonly dsl: string;
}

/**
 * Pre-computed CommandCard event — DSL for direct rendering.
 *
 * Emitted by the hook interpreter when a tool result contains `approvalDsl`.
 * Separated from the tool result channel so the LLM never echoes raw JSON.
 */
export interface DataFlowUIPart {
  readonly type: "data-flow-ui";
  readonly data: CommandCardPayload;
}

// ============================================================================
// Latency Summary Types (Backend-Owned Metrics)
// ============================================================================

/**
 * Tool execution status.
 */
export type ToolStatus = "completed" | "error";

/**
 * Timing record for a single tool execution.
 */
export interface ToolTiming {
  readonly toolName: string;
  readonly toolCallId: string;
  readonly durationMs: number;
  readonly status: ToolStatus;
  readonly payloadSize?: number;
}

/**
 * Phase breakdown within a request.
 *
 * - llmTimeMs: Time spent in LLM API calls
 * - toolTimeMs: Wall-clock time in tool execution
 * - llmCalls: Number of LLM round-trips
 */
export interface PhaseBreakdown {
  readonly llmTimeMs: number;
  readonly toolTimeMs: number;
  readonly llmCalls: number;
}

/**
 * KV store metrics for latency panel.
 *
 * Tracks bytes read/written and operation counts for plan persistence.
 */
export interface KVMetrics {
  readonly bytesWritten: number;
  readonly bytesRead: number;
  readonly kvDurationMs: number;
  readonly putCount: number;
  readonly getCount: number;
}

/**
 * Zero element for KVMetrics monoid.
 */
export const kvMetricsZero: KVMetrics = {
  bytesWritten: 0,
  bytesRead: 0,
  kvDurationMs: 0,
  putCount: 0,
  getCount: 0,
};

/**
 * Check if KV metrics are zero.
 */
export const isKVMetricsZero = (m: KVMetrics): boolean =>
  m.bytesWritten === 0 &&
  m.bytesRead === 0 &&
  m.kvDurationMs === 0 &&
  m.putCount === 0 &&
  m.getCount === 0;

/**
 * LLM token usage metrics.
 *
 * Tracks input/output tokens for cost attribution and capacity planning.
 */
export interface TokenMetrics {
  readonly inputTokens: number;
  readonly outputTokens: number;
  readonly cachedTokens: number;
  readonly cacheCreationTokens: number;
}

/**
 * Zero element for TokenMetrics monoid.
 */
export const tokenMetricsZero: TokenMetrics = {
  inputTokens: 0,
  outputTokens: 0,
  cachedTokens: 0,
  cacheCreationTokens: 0,
};

/**
 * Check if token metrics are zero.
 */
export const isTokenMetricsZero = (m: TokenMetrics): boolean =>
  m.inputTokens === 0 &&
  m.outputTokens === 0 &&
  m.cachedTokens === 0 &&
  m.cacheCreationTokens === 0;

/**
 * Cache hit rate as a ratio (0.0 to 1.0).
 */
export const tokenCacheHitRate = (m: TokenMetrics): number | null => {
  if (m.inputTokens === 0) return null;
  return m.cachedTokens / m.inputTokens;
};

/**
 * Cache hit rate as a percentage.
 */
export const tokenCacheHitRatePercent = (m: TokenMetrics): number | null => {
  const rate = tokenCacheHitRate(m);
  return rate !== null ? rate * 100 : null;
};

/**
 * Combine two KVMetrics values (monoid operation).
 *
 * Laws:
 * - Identity: combine(zero, a) = a = combine(a, zero)
 * - Associativity: combine(combine(a, b), c) = combine(a, combine(b, c))
 */
export const combineKVMetrics = (a: KVMetrics, b: KVMetrics): KVMetrics => ({
  bytesWritten: a.bytesWritten + b.bytesWritten,
  bytesRead: a.bytesRead + b.bytesRead,
  kvDurationMs: a.kvDurationMs + b.kvDurationMs,
  putCount: a.putCount + b.putCount,
  getCount: a.getCount + b.getCount,
});

/**
 * Combine two TokenMetrics values (monoid operation).
 */
export const combineTokenMetrics = (a: TokenMetrics, b: TokenMetrics): TokenMetrics => ({
  inputTokens: a.inputTokens + b.inputTokens,
  outputTokens: a.outputTokens + b.outputTokens,
  cachedTokens: a.cachedTokens + b.cachedTokens,
  cacheCreationTokens: a.cacheCreationTokens + b.cacheCreationTokens,
});

// ============================================================================
// Derived Latency Metrics (Computed, Not Stored)
// ============================================================================

/**
 * Calculate tokens per second throughput.
 *
 * Returns null if duration is zero or token metrics unavailable.
 */
export const tokensPerSecond = (latency: LatencySummary): number | null => {
  if (latency.totalDurationMs === 0 || !latency.tokenMetrics) return null;
  const totalTokens = latency.tokenMetrics.inputTokens + latency.tokenMetrics.outputTokens;
  return (totalTokens / latency.totalDurationMs) * 1000;
};

/**
 * Calculate output tokens per second (generation throughput).
 */
export const outputTokensPerSecond = (latency: LatencySummary): number | null => {
  if (latency.totalDurationMs === 0 || !latency.tokenMetrics) return null;
  return (latency.tokenMetrics.outputTokens / latency.totalDurationMs) * 1000;
};

/**
 * Calculate input tokens per second (prompt processing speed).
 *
 * This measures how fast the LLM processes the input context.
 * Higher values indicate faster prompt ingestion.
 */
export const inputTokensPerSecond = (latency: LatencySummary): number | null => {
  if (latency.totalDurationMs === 0 || !latency.tokenMetrics) return null;
  return (latency.tokenMetrics.inputTokens / latency.totalDurationMs) * 1000;
};

/**
 * Calculate LLM utilization as a ratio (0.0 to 1.0).
 *
 * Measures what fraction of total time was spent in LLM API calls.
 */
export const llmUtilization = (latency: LatencySummary): number | null => {
  if (latency.totalDurationMs === 0) return null;
  return latency.phases.llmTimeMs / latency.totalDurationMs;
};

/**
 * Calculate tool utilization as a ratio (0.0 to 1.0).
 *
 * Measures what fraction of total time was spent in tool execution.
 */
export const toolUtilization = (latency: LatencySummary): number | null => {
  if (latency.totalDurationMs === 0) return null;
  return latency.phases.toolTimeMs / latency.totalDurationMs;
};

/**
 * Calculate average tokens per LLM call.
 */
export const tokensPerLLMCall = (latency: LatencySummary): number | null => {
  if (latency.phases.llmCalls === 0 || !latency.tokenMetrics) return null;
  const totalTokens = latency.tokenMetrics.inputTokens + latency.tokenMetrics.outputTokens;
  return totalTokens / latency.phases.llmCalls;
};

/**
 * Calculate average LLM call latency.
 */
export const avgLLMLatencyMs = (latency: LatencySummary): number | null => {
  if (latency.phases.llmCalls === 0) return null;
  return latency.phases.llmTimeMs / latency.phases.llmCalls;
};

/**
 * Calculate average tool execution latency.
 */
export const avgToolLatencyMs = (latency: LatencySummary): number | null => {
  if (latency.toolTimings.length === 0) return null;
  const totalToolTime = latency.toolTimings.reduce((sum, t) => sum + t.durationMs, 0);
  return totalToolTime / latency.toolTimings.length;
};

/**
 * Calculate overhead time (total - LLM - tools).
 *
 * This includes network latency, serialization, and orchestration.
 */
export const overheadMs = (latency: LatencySummary): number => {
  const accounted = latency.phases.llmTimeMs + latency.phases.toolTimeMs;
  return Math.max(0, latency.totalDurationMs - accounted);
};

/**
 * Count retries by reason.
 */
export const retryCountByReason = (events: readonly RetryEvent[]): Record<RetryReason, number> => {
  const counts: Record<RetryReason, number> = {
    rate_limit: 0,
    timeout: 0,
    context_length: 0,
    server_error: 0,
    network_error: 0,
    content_filter: 0,
    unknown: 0,
  };
  for (const event of events) {
    counts[event.reason]++;
  }
  return counts;
};

// ============================================================================
// Retry Types
// ============================================================================

/**
 * Categorized reasons for retries during request processing.
 */
export type RetryReason =
  | "rate_limit"
  | "timeout"
  | "context_length"
  | "server_error"
  | "network_error"
  | "content_filter"
  | "unknown";

/**
 * A single retry event with reason and timing.
 */
export interface RetryEvent {
  readonly reason: RetryReason;
  readonly attempt: number;
  readonly durationMs: number;
}

/**
 * Complete latency summary for a request.
 *
 * Emitted by backend at stream end. Frontend just displays.
 */
export interface LatencySummary {
  readonly totalDurationMs: number;
  readonly phases: PhaseBreakdown;
  readonly toolTimings: readonly ToolTiming[];
  /** KV store metrics (bytes read/written, operation counts) */
  readonly kvMetrics?: KVMetrics;
  /** LLM token usage metrics */
  readonly tokenMetrics?: TokenMetrics;
  /** Time to first token in milliseconds (streaming latency) */
  readonly ttftMs?: number;
  /** Time to first text delta in milliseconds */
  readonly firstTextMs?: number;
  readonly productSetSize?: number;
  readonly planPayloadBytes?: number;
  readonly retryCount: number;
  /** Categorized retry events */
  readonly retryEvents?: readonly RetryEvent[];
  readonly hadTimeout: boolean;
}

/**
 * Latency metrics summary (emitted at stream end).
 */
export interface DataLatencySummaryPart {
  readonly type: "data-latency-summary";
  readonly data: LatencySummary;
}

/**
 * Finish reason for stream termination.
 */
export type FinishReason = "stop" | "tool-calls" | "length" | "content-filter";

/**
 * Stream completion event.
 */
export interface FinishPart {
  readonly type: "finish";
  readonly finishReason: FinishReason;
  readonly usage: TokenUsage;
}

/**
 * Error information.
 */
export interface ErrorInfo {
  readonly message: string;
  readonly code?: string;
}

/**
 * Error event (non-recoverable).
 */
export interface ErrorPart {
  readonly type: "error";
  readonly error: ErrorInfo;
}

/**
 * Custom/extension event (forward-compat for domain-specific events).
 */
export interface CustomPart {
  readonly type: "custom";
  readonly name: string;
  readonly data: unknown;
}

// ============================================================================
// Stream Part Union (Closed Sum Type)
// ============================================================================

/**
 * The 14 distinct part types of the AI SDK Data Stream Protocol.
 *
 * This is a CLOSED sum type - exhaustive pattern matching is enforced.
 */
export type StreamPart =
  | TextPart
  | ReasoningPart
  | StepStartPart
  | ToolInvocationPart
  | ToolAgentPart
  | ToolProgressPart
  | DataToolAgentPart
  | DataFileRegisteredPart
  | DataCostSummaryPart
  | DataFlowUIPart
  | DataLatencySummaryPart
  | FinishPart
  | ErrorPart
  | CustomPart;

// ============================================================================
// Type Guards (Pure Predicates)
// ============================================================================

export const isTextPart = (part: StreamPart): part is TextPart => part.type === "text";

export const isReasoningPart = (part: StreamPart): part is ReasoningPart =>
  part.type === "reasoning";

export const isStepStartPart = (part: StreamPart): part is StepStartPart =>
  part.type === "step-start";

export const isToolInvocationPart = (part: StreamPart): part is ToolInvocationPart =>
  part.type === "tool-invocation";

export const isToolAgentPart = (part: StreamPart): part is ToolAgentPart =>
  part.type === "tool-agent";

export const isDataToolAgentPart = (part: StreamPart): part is DataToolAgentPart =>
  part.type === "data-tool-agent";

export const isDataFileRegisteredPart = (part: StreamPart): part is DataFileRegisteredPart =>
  part.type === "data-file-registered";

export const isDataCostSummaryPart = (part: StreamPart): part is DataCostSummaryPart =>
  part.type === "data-cost-summary";

export const isDataLatencySummaryPart = (part: StreamPart): part is DataLatencySummaryPart =>
  part.type === "data-latency-summary";

export const isFinishPart = (part: StreamPart): part is FinishPart => part.type === "finish";

export const isErrorPart = (part: StreamPart): part is ErrorPart => part.type === "error";

export const isDataFlowUIPart = (part: StreamPart): part is DataFlowUIPart =>
  part.type === "data-flow-ui";

export const isToolProgressPart = (part: StreamPart): part is ToolProgressPart =>
  part.type === "tool-progress";

export const isCustomPart = (part: StreamPart): part is CustomPart => part.type === "custom";

/**
 * Check if part is a terminal event (Finish or Error).
 */
export const isTerminalPart = (part: StreamPart): part is FinishPart | ErrorPart =>
  part.type === "finish" || part.type === "error";

/**
 * Check if part is a tool call (state = "call").
 */
export const isToolCall = (part: StreamPart): part is ToolInvocationPart =>
  isToolInvocationPart(part) && part.state === "call";

/**
 * Check if part is a tool result (state = "result").
 */
export const isToolResult = (part: StreamPart): part is ToolInvocationPart =>
  isToolInvocationPart(part) && part.state === "result";

// ============================================================================
// TokenUsage Monoid
// ============================================================================

/**
 * Zero element for TokenUsage monoid.
 */
export const tokenUsageZero: TokenUsage = {
  promptTokens: 0,
  completionTokens: 0,
  cacheReadInputTokens: 0,
  cacheCreationInputTokens: 0,
  totalTokens: 0,
};

/**
 * Combine two TokenUsage values (monoid operation).
 *
 * Laws:
 * - Identity: combine(zero, a) = a = combine(a, zero)
 * - Associativity: combine(combine(a, b), c) = combine(a, combine(b, c))
 * - Commutativity: combine(a, b) = combine(b, a)
 */
export const combineTokenUsage = (a: TokenUsage, b: TokenUsage): TokenUsage => ({
  promptTokens: a.promptTokens + b.promptTokens,
  completionTokens: a.completionTokens + b.completionTokens,
  cacheReadInputTokens: (a.cacheReadInputTokens ?? 0) + (b.cacheReadInputTokens ?? 0),
  cacheCreationInputTokens: (a.cacheCreationInputTokens ?? 0) + (b.cacheCreationInputTokens ?? 0),
  totalTokens: a.totalTokens + b.totalTokens,
});

/**
 * Cache hit rate for a CostSummary (prompt cache / total prompt tokens).
 * Returns null if no prompt tokens.
 */
export const costCacheHitRatePercent = (cost: CostSummary): number | null => {
  if (cost.totalPromptTokens === 0 || !cost.totalCacheReadInputTokens) return null;
  return (cost.totalCacheReadInputTokens / cost.totalPromptTokens) * 100;
};

/**
 * Combine multiple TokenUsage values.
 */
export const combineAllTokenUsage = (usages: TokenUsage[]): TokenUsage =>
  usages.reduce(combineTokenUsage, tokenUsageZero);

// ============================================================================
// SSE Parsing
// ============================================================================

/**
 * Parse a single SSE line into a StreamPart.
 *
 * Format: "data: {json}\n\n"
 */
export const parseSSELine = (line: string): StreamPart | null => {
  const trimmed = line.trim();
  if (!trimmed.startsWith("data:")) return null;

  const jsonStr = trimmed.slice(5).trim();
  if (!jsonStr || jsonStr === "[DONE]") return null;

  try {
    return JSON.parse(jsonStr) as StreamPart;
  } catch {
    return null;
  }
};
