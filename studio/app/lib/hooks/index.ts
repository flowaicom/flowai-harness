/**
 * Custom hooks module.
 *
 * @module lib/hooks
 */

// Chat effects hooks ("Effects at the Edges" pattern)
export {
  useAutoScroll,
  useChatEffects,
  useDebouncedEffect,
  useFirstTokenTracking,
  useStatusTransition,
  useThreadTouch,
  useToolTrackingSync,
  useUnmountCleanup,
} from "./use-chat-effects";
// Performance hooks
export {
  type LatencyMetrics,
  type LatencyTracker,
  useLatencyTracker,
} from "./use-latency-tracker";
// Storage hooks (SSR-safe, cross-tab sync)
export {
  useLocalStorage,
  useLocalStorageValue,
  useSessionStorage,
} from "./use-local-storage";
// Media query hooks (SSR-safe, responsive)
export {
  useCanHover,
  useHasCoarsePointer,
  useHasFinePointer,
  useIsDesktop,
  useIsLargeDesktop,
  useIsMobile,
  useIsTablet,
  useMediaQuery,
  usePrefersDarkMode,
  usePrefersHighContrast,
  usePrefersLightMode,
  usePrefersReducedMotion,
} from "./use-media-query";
export { useOptimizedScroll } from "./use-optimized-scroll";
// Pending auto-send (cross-route message hand-off via URL param)
export { usePendingAutoSend } from "./use-pending-auto-send";
export { usePeriodicRefresh } from "./use-periodic-refresh";
// Session heartbeat (keep-alive with sliding window expiry)
export {
  formatRemainingTime,
  getSessionStatusColor,
  type SessionState,
  type SessionStatus,
  type UseSessionHeartbeatOptions,
  useSessionHeartbeat,
} from "./use-session-heartbeat";
// Shallow comparison for Zustand selectors
export {
  createPickSelector,
  createShallowPick,
  shallowEqual,
  useShallow,
} from "./use-shallow";
// Source selection (role-aware default + local override)
export { useSourceId } from "./use-source-id";
// Sub-agent message loading
export {
  countToolsByName,
  extractToolInvocations,
  type SubAgentLoadingState,
  type UseSubAgentMessagesOptions,
  type UseSubAgentMessagesReturn,
  useSubAgentMessages,
} from "./use-sub-agent-messages";
// Trace collection (automatic latency report collection)
export {
  type UseTraceCollectionOptions,
  useResetTraceCollection,
  useTraceCollection,
} from "./use-trace-collection";
