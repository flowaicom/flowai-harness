/**
 * Shared arbitrary (generator) library for property-based testing.
 *
 * Composable generator DSL organized in layers matching the crate architecture:
 *
 *   Layer 0 — Primitives (dates, branded IDs)
 *   Layer 1 — Stream Parts (token metrics, latency, 14-variant StreamPart)
 *   Layer 2 — Message Parts & Messages (8-variant MessagePart, Message)
 *   Layer 3 — Domain modules (filters, scenario, result)
 *
 * Each layer builds on the previous — complex generators compose from simple
 * ones. This is the generator algebra: pure descriptions of input spaces,
 * separated from the test assertions that interpret them.
 *
 * @module test-utils/arbitraries
 */

import * as fc from "fast-check";
import type {
  EntityFilter,
  FilterSet,
  MeasureAggregate,
  NumericOperator,
} from "~/lib/domain/filter";
import { booleanFilter, matchedFilter, measureFilter, numericFilter } from "~/lib/domain/filter";
import type { Message, MessageId, MessagePart, ToolInvocationState } from "~/lib/domain/message";
import type { Result } from "~/lib/domain/result";
import { err, ok } from "~/lib/domain/result";
import type {
  DistributionEntry,
  EntitySetGlimpse,
  NumericRange,
  PlanStatus,
} from "~/lib/domain/scenario";
// Type imports — domain layer
import type {
  CostSummary,
  FinishReason,
  KVMetrics,
  LatencySummary,
  PhaseBreakdown,
  RetryEvent,
  RetryReason,
  StreamPart,
  TokenMetrics,
  TokenUsage,
  ToolState,
  ToolStatus,
  ToolTiming,
} from "~/lib/domain/stream-part";

// ============================================================================
// Layer 0 — Primitives
// ============================================================================

/** ISO 8601 date string from a safe timestamp range (1970–2099). */
export const arbISODate = fc
  .integer({ min: 0, max: 4102444800000 })
  .map((ts) => new Date(ts).toISOString());

// ============================================================================
// Layer 1 — Stream Part Types
// ============================================================================

// -- Metric types --

/** TokenUsage with all fields present (canonical monoid element). */
export const arbTokenUsage: fc.Arbitrary<TokenUsage> = fc.record({
  promptTokens: fc.nat(),
  completionTokens: fc.nat(),
  cacheReadInputTokens: fc.nat(),
  cacheCreationInputTokens: fc.nat(),
  totalTokens: fc.nat(),
});

/** TokenUsage where optional cache fields may be undefined. */
export const arbTokenUsageWithOptional: fc.Arbitrary<TokenUsage> = fc.record({
  promptTokens: fc.nat(),
  completionTokens: fc.nat(),
  cacheReadInputTokens: fc.option(fc.nat(), { nil: undefined }),
  cacheCreationInputTokens: fc.option(fc.nat(), { nil: undefined }),
  totalTokens: fc.nat(),
});

export const arbKVMetrics: fc.Arbitrary<KVMetrics> = fc.record({
  bytesWritten: fc.nat(),
  bytesRead: fc.nat(),
  kvDurationMs: fc.nat(),
  putCount: fc.nat(),
  getCount: fc.nat(),
});

export const arbTokenMetrics: fc.Arbitrary<TokenMetrics> = fc.record({
  inputTokens: fc.nat(),
  outputTokens: fc.nat(),
  cachedTokens: fc.nat(),
  cacheCreationTokens: fc.nat(),
});

// -- Retry / Latency --

export const arbRetryReason: fc.Arbitrary<RetryReason> = fc.constantFrom(
  "rate_limit" as const,
  "timeout" as const,
  "context_length" as const,
  "server_error" as const,
  "network_error" as const,
  "content_filter" as const,
  "unknown" as const
);

export const arbRetryEvent: fc.Arbitrary<RetryEvent> = fc.record({
  reason: arbRetryReason,
  attempt: fc.nat(),
  durationMs: fc.nat(),
});

export const arbPhaseBreakdown: fc.Arbitrary<PhaseBreakdown> = fc.record({
  llmTimeMs: fc.nat(),
  toolTimeMs: fc.nat(),
  llmCalls: fc.nat(),
});

export const arbToolStatus: fc.Arbitrary<ToolStatus> = fc.constantFrom(
  "completed" as const,
  "error" as const
);

export const arbToolTiming: fc.Arbitrary<ToolTiming> = fc.record({
  toolName: fc.string(),
  toolCallId: fc.string(),
  durationMs: fc.nat(),
  status: arbToolStatus,
  payloadSize: fc.option(fc.nat(), { nil: undefined }),
});

export const arbLatencySummary: fc.Arbitrary<LatencySummary> = fc.record({
  totalDurationMs: fc.nat(),
  phases: arbPhaseBreakdown,
  toolTimings: fc.array(arbToolTiming),
  kvMetrics: fc.option(arbKVMetrics, { nil: undefined }),
  tokenMetrics: fc.option(arbTokenMetrics, { nil: undefined }),
  ttftMs: fc.option(fc.nat(), { nil: undefined }),
  firstTextMs: fc.option(fc.nat(), { nil: undefined }),
  productSetSize: fc.option(fc.nat(), { nil: undefined }),
  planPayloadBytes: fc.option(fc.nat(), { nil: undefined }),
  retryCount: fc.nat(),
  retryEvents: fc.option(fc.array(arbRetryEvent), { nil: undefined }),
  hadTimeout: fc.boolean(),
});

export const arbCostSummary: fc.Arbitrary<CostSummary> = fc.record({
  agents: fc.array(
    fc.record({
      agentName: fc.string(),
      model: fc.string(),
      usage: arbTokenUsage,
    })
  ),
  totalPromptTokens: fc.nat(),
  totalCompletionTokens: fc.nat(),
  totalCacheReadInputTokens: fc.option(fc.nat(), { nil: undefined }),
  totalCacheCreationInputTokens: fc.option(fc.nat(), { nil: undefined }),
  totalTokens: fc.nat(),
});

// -- StreamPart enum values --

export const arbToolState: fc.Arbitrary<ToolState> = fc.constantFrom(
  "call" as const,
  "result" as const
);

export const arbFinishReason: fc.Arbitrary<FinishReason> = fc.constantFrom(
  "stop" as const,
  "tool-calls" as const,
  "length" as const,
  "content-filter" as const
);

// -- Individual StreamPart variants --

export const arbTextStreamPart: fc.Arbitrary<StreamPart> = fc.record({
  type: fc.constant("text" as const),
  text: fc.string(),
});

export const arbReasoningStreamPart: fc.Arbitrary<StreamPart> = fc.record({
  type: fc.constant("reasoning" as const),
  text: fc.string(),
});

export const arbErrorStreamPart: fc.Arbitrary<StreamPart> = fc.record({
  type: fc.constant("error" as const),
  error: fc.record({
    message: fc.string(),
    code: fc.option(fc.string(), { nil: undefined }),
  }),
});

/** StreamParts that don't change accumulator content. */
export const arbNoOpStreamPart: fc.Arbitrary<StreamPart> = fc.oneof(
  fc.constant({ type: "step-start" as const }),
  fc.record({
    type: fc.constant("data-cost-summary" as const),
    data: fc.constant({
      agents: [],
      totalPromptTokens: 0,
      totalCompletionTokens: 0,
      totalTokens: 0,
    }),
  }),
  fc.record({
    type: fc.constant("finish" as const),
    finishReason: fc.constant("stop" as const),
    usage: fc.constant({
      promptTokens: 0,
      completionTokens: 0,
      totalTokens: 0,
    }),
  }),
  fc.record({
    type: fc.constant("custom" as const),
    name: fc
      .string()
      .filter((name) => name !== "approval.required" && name !== "approval.decision"),
    data: fc.anything(),
  })
) as fc.Arbitrary<StreamPart>;

/** Content-bearing StreamParts (text, reasoning, error). */
export const arbContentStreamPart: fc.Arbitrary<StreamPart> = fc.oneof(
  arbTextStreamPart,
  arbReasoningStreamPart,
  arbErrorStreamPart
) as fc.Arbitrary<StreamPart>;

/** Full 14-variant StreamPart. */
export const arbStreamPart: fc.Arbitrary<StreamPart> = fc.oneof(
  fc.record({ type: fc.constant("text" as const), text: fc.string() }),
  fc.record({ type: fc.constant("reasoning" as const), text: fc.string() }),
  fc.constant({ type: "step-start" as const }),
  fc.record({
    type: fc.constant("tool-invocation" as const),
    toolInvocationId: fc.string(),
    toolName: fc.string(),
    args: fc.anything(),
    state: arbToolState,
    result: fc.option(fc.anything(), { nil: undefined }),
  }),
  fc.record({
    type: fc.constant("tool-agent" as const),
    agentName: fc.string(),
    toolInvocationId: fc.string(),
    state: arbToolState,
  }),
  fc.record({
    type: fc.constant("tool-progress" as const),
    toolName: fc.string(),
    toolCallId: fc.option(fc.string(), { nil: undefined }),
    label: fc.string(),
    phaseIndex: fc.nat(),
    totalPhases: fc.nat(),
    milestone: fc.option(fc.dictionary(fc.string(), fc.anything()), {
      nil: undefined,
    }),
  }),
  fc.record({
    type: fc.constant("data-tool-agent" as const),
    data: fc.record({
      agentName: fc.string(),
      model: fc.string(),
      usage: arbTokenUsage,
    }),
  }),
  fc.record({
    type: fc.constant("data-file-registered" as const),
    data: fc.record({
      fileId: fc.string(),
      filename: fc.string(),
      threadId: fc.string(),
      timestamp: fc.string(),
    }),
  }),
  fc.record({
    type: fc.constant("data-cost-summary" as const),
    data: arbCostSummary,
  }),
  fc.record({
    type: fc.constant("data-flow-ui" as const),
    data: fc.record({ dsl: fc.string() }),
  }),
  fc.record({
    type: fc.constant("data-latency-summary" as const),
    data: arbLatencySummary,
  }),
  fc.record({
    type: fc.constant("finish" as const),
    finishReason: arbFinishReason,
    usage: arbTokenUsage,
  }),
  fc.record({
    type: fc.constant("error" as const),
    error: fc.record({
      message: fc.string(),
      code: fc.option(fc.string(), { nil: undefined }),
    }),
  }),
  fc.record({
    type: fc.constant("custom" as const),
    name: fc.string(),
    data: fc.anything(),
  })
) as fc.Arbitrary<StreamPart>;

// ============================================================================
// Layer 2 — Message Parts & Messages
// ============================================================================

export const arbToolInvocationState: fc.Arbitrary<ToolInvocationState> = fc.constantFrom(
  "partial-call" as const,
  "call" as const,
  "result" as const,
  "cancelled" as const
);

export const arbTextMsgPart: fc.Arbitrary<MessagePart> = fc.record({
  type: fc.constant("text" as const),
  text: fc.string(),
});

export const arbReasoningMsgPart: fc.Arbitrary<MessagePart> = fc.record({
  type: fc.constant("reasoning" as const),
  text: fc.string(),
});

export const arbToolInvMsgPart: fc.Arbitrary<MessagePart> = fc.record({
  type: fc.constant("tool-invocation" as const),
  toolCallId: fc.string({ minLength: 1 }),
  toolName: fc.string({ minLength: 1 }),
  args: fc.constant({}),
  state: arbToolInvocationState,
  result: fc.option(fc.constant("ok"), { nil: undefined }),
});

export const arbToolAgentMsgPart: fc.Arbitrary<MessagePart> = fc.record({
  type: fc.constant("tool-agent" as const),
  toolCallId: fc.string({ minLength: 1 }),
  agentName: fc.string({ minLength: 1 }),
  state: arbToolInvocationState,
});

export const arbFileMsgPart: fc.Arbitrary<MessagePart> = fc.record({
  type: fc.constant("file" as const),
  fileId: fc.string({ minLength: 1 }),
  filename: fc.string({ minLength: 1 }),
});

export const arbFlowUiMsgPart: fc.Arbitrary<MessagePart> = fc.record({
  type: fc.constant("flow-ui" as const),
  dsl: fc.string(),
});

export const arbApprovalRequiredMsgPart: fc.Arbitrary<MessagePart> = fc.record({
  type: fc.constant("approval-required" as const),
  approvalId: fc.string({ minLength: 1 }),
  title: fc.string({ minLength: 1 }),
  kind: fc.string({ minLength: 1 }),
  status: fc.constantFrom(
    "pending" as const,
    "approve" as const,
    "reject" as const,
    "revise" as const
  ),
  payload: fc.dictionary(fc.string({ minLength: 1 }), fc.anything()),
});

/** All 7 display MessagePart variants (excluding tool-progress which is accumulator-internal). */
export const arbMessagePart: fc.Arbitrary<MessagePart> = fc.oneof(
  arbTextMsgPart,
  arbReasoningMsgPart,
  arbToolInvMsgPart,
  arbToolAgentMsgPart,
  arbFileMsgPart,
  arbFlowUiMsgPart,
  arbApprovalRequiredMsgPart
) as fc.Arbitrary<MessagePart>;

/** Message with small ID space (forces duplicates for dedup testing). */
export const arbMessage: fc.Arbitrary<Message> = fc.record({
  id: fc.constantFrom("m1", "m2", "m3", "m4", "m5").map((s) => s as MessageId),
  role: fc.constantFrom("user" as const, "assistant" as const, "system" as const),
  parts: fc.array(arbMessagePart, { maxLength: 10 }),
  createdAt: arbISODate,
  isStreaming: fc.option(fc.boolean(), { nil: undefined }),
});

/** Message with unique ID (no duplicates). */
export const arbUniqueMessage: fc.Arbitrary<Message> = fc.record({
  id: fc.uuid().map((s) => s as MessageId),
  role: fc.constantFrom("user" as const, "assistant" as const, "system" as const),
  parts: fc.array(arbMessagePart, { maxLength: 5 }),
  createdAt: arbISODate,
});

// ============================================================================
// Layer 3 — Domain Modules
// ============================================================================

// -- Filters --

export const arbNumericOp: fc.Arbitrary<NumericOperator> = fc.constantFrom(
  "=" as const,
  ">" as const,
  "<" as const,
  ">=" as const,
  "<=" as const,
  "!=" as const,
  "BETWEEN" as const
);

export const arbMeasureAgg: fc.Arbitrary<MeasureAggregate> = fc.constantFrom(
  "avg" as const,
  "any" as const,
  "min" as const,
  "max" as const
);

export const arbMatchedFilter = fc
  .tuple(fc.string(), fc.array(fc.string()))
  .map(([field, values]) => matchedFilter(field, values));

export const arbNumericFilter = fc
  .tuple(fc.string(), arbNumericOp, fc.integer(), fc.option(fc.integer(), { nil: undefined }))
  .map(([field, op, v, v2]) => numericFilter(field, op, v, v2));

export const arbBooleanFilter = fc
  .tuple(fc.string(), fc.boolean())
  .map(([field, value]) => booleanFilter(field, value));

export const arbMeasureFilter = fc
  .tuple(
    fc.string(),
    arbMeasureAgg,
    arbNumericOp,
    fc.integer(),
    fc.option(fc.integer(), { nil: undefined })
  )
  .map(([metric, agg, op, v, v2]) => measureFilter(metric, agg, op, v, v2));

export const arbEntityFilter: fc.Arbitrary<EntityFilter> = fc.oneof(
  arbMatchedFilter,
  arbNumericFilter,
  arbBooleanFilter,
  arbMeasureFilter
) as fc.Arbitrary<EntityFilter>;

export const arbFilterSet: fc.Arbitrary<FilterSet> = fc.record({
  matched: fc.array(arbMatchedFilter),
  numeric: fc.array(arbNumericFilter),
  boolean: fc.array(arbBooleanFilter),
  measure: fc.array(arbMeasureFilter),
});

// -- Scenario --

export const arbPlanStatus: fc.Arbitrary<PlanStatus> = fc.constantFrom(
  "pending" as const,
  "approved" as const,
  "executing" as const,
  "executed" as const,
  "failed" as const
);

export const arbDistEntry: fc.Arbitrary<DistributionEntry> = fc.record({
  value: fc.string(),
  count: fc.nat(),
  percentage: fc.double({ min: 0, max: 100, noNaN: true }),
});

export const arbNumericRange: fc.Arbitrary<NumericRange> = fc
  .tuple(
    fc.double({ noNaN: true, noDefaultInfinity: true }),
    fc.double({ noNaN: true, noDefaultInfinity: true })
  )
  .map(([a, b]) => {
    const min = Math.min(a, b);
    const max = Math.max(a, b);
    return { min, max, mean: (min + max) / 2 };
  });

export const arbGlimpse: fc.Arbitrary<EntitySetGlimpse> = fc.record({
  entityCount: fc.nat(),
  distributions: fc.dictionary(fc.string({ minLength: 1 }), fc.array(arbDistEntry)),
  numericRanges: fc.dictionary(fc.string({ minLength: 1 }), arbNumericRange),
});

// -- Result --

export const arbOk = fc.integer().map(ok);
export const arbErr = fc.string().map(err);
export const arbResult: fc.Arbitrary<Result<number, string>> = fc.oneof(arbOk, arbErr);
export const arbKleisli = fc.func(arbResult);
