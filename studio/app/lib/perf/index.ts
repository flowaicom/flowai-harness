/**
 * Performance utilities module.
 *
 * @module lib/perf
 */

// Debug logging with production optimization
export {
  createDebugLogger,
  createDebugTimer,
  debugLoggers,
  debugOnly,
  devOnly,
  isDebugEnabled,
  isDebugMode,
} from "./debug";
// Bridge backend latency to trace collection
export {
  backendLatencyToTrace,
  type ClientTimingMetrics,
  extractClientMetrics,
} from "./latency-bridge";
// Latency report infrastructure
export {
  aggregateToolBreakdown,
  calculatePercentileStats,
  formatMs,
  formatPercentileStats,
  generateLatencyReport,
  generateMarkdownSummary,
  getTraceCollector,
  type LatencyReport,
  type PercentileStats,
  type PhaseTimings,
  type RequestTrace,
  resetTraceCollector,
  type TokenCounts,
  type ToolBreakdown,
  type ToolRecord,
  TraceCollector,
} from "./latency-report";
export { type CacheStats, getMessageCache, getTransformCache, LRUCache } from "./lru-cache";
// Named memo comparators
export {
  compareById,
  compareByIdAndState,
  compareByIdStateAnd,
  compareByState,
  compareByText,
  compareMessageProps,
  compareMetricCard,
  compareStreamingParts,
  compareSubAgentRow,
  compareToolAgent,
  compareToolInvocation,
  createPropsComparator,
  withStreamingOverride,
} from "./memo-comparators";
// Message transformation with deduplication
export {
  countTotalParts,
  deduplicateMessages,
  deduplicateTextParts,
  deduplicateToolParts,
  detectMessageFormat,
  hasAnyAgentCalls,
  hasAnyToolInvocations,
  sortMessagesByTimestamp,
  type TransformOptions,
  transformMessages,
} from "./message-transformer";
export {
  isWorkerReady,
  terminateWorker,
  transformMessagesAsync,
  type WorkerRequest,
  type WorkerResponse,
} from "./message-worker";
export {
  cancelDebounce,
  cancelFrame,
  cancelIdle,
  cancelIdleTask,
  cancelRafThrottle,
  clearAllDebounce,
  clearIdleTasks,
  debounce,
  rafThrottle,
  requestFrame,
  requestIdle,
  scheduleIdle,
  scheduleMicrotask,
} from "./scheduler";
