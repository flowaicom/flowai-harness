/**
 * Chat effects hook - separates side effects from pure store.
 *
 * Following "Effects at the Edges" pattern:
 * - Pure store: data + transitions (no effects)
 * - Effects hook: subscribes to external data, dispatches to store
 *
 * Performance optimizations:
 * - requestIdleCallback for non-critical work
 * - Async worker-based message transformation
 * - Abort signal support for cleanup
 *
 * @module hooks/use-chat-effects
 */

import { useCallback, useEffect, useRef } from "react";
import { parsePersistedMessages } from "~/lib/domain/message";
import { isOk } from "~/lib/domain/result";
import { rafThrottle, scheduleIdle } from "~/lib/perf/scheduler";
import { threadMessageToUiMessage, useHarnessRuntime } from "~/lib/runtime";
import { selectConversationLoading, useConversation, useConversationActions } from "~/lib/stores";

// ============================================================================
// Types
// ============================================================================

interface UseChatEffectsOptions {
  threadId: string;
  /** Ref to messages end element for scrolling */
  messagesEndRef?: React.RefObject<HTMLElement | null>;
  /** Callback when messages are loaded */
  onMessagesLoaded?: () => void;
  /** Callback when error occurs */
  onError?: (error: string) => void;
}

interface UseChatEffectsReturn {
  /** Whether messages are being loaded */
  isLoading: boolean;
  /** Reload messages from API */
  reloadMessages: () => Promise<void>;
}

// ============================================================================
// Hook Implementation
// ============================================================================

/**
 * Effects hook for chat thread management.
 *
 * Handles:
 * - Loading messages when threadId changes
 * - Scrolling to bottom after load
 * - Error handling
 * - Cleanup on unmount
 */
export function useChatEffects({
  threadId,
  messagesEndRef,
  onMessagesLoaded,
  onError,
}: UseChatEffectsOptions): UseChatEffectsReturn {
  const abortControllerRef = useRef<AbortController | null>(null);
  const onMessagesLoadedRef = useRef(onMessagesLoaded);
  const onErrorRef = useRef(onError);
  const { adapter, scope } = useHarnessRuntime();

  // Use action bundle hook for stable references
  const { replaceHistory, setLoadPhase } = useConversationActions();
  const isLoading = useConversation(selectConversationLoading) as boolean;

  useEffect(() => {
    onMessagesLoadedRef.current = onMessagesLoaded;
  }, [onMessagesLoaded]);

  useEffect(() => {
    onErrorRef.current = onError;
  }, [onError]);

  // Load messages function (memoized for external use)
  const loadMessages = useCallback(async () => {
    // Abort any in-flight request
    if (abortControllerRef.current) {
      abortControllerRef.current.abort();
    }

    abortControllerRef.current = new AbortController();
    setLoadPhase({ phase: "loading" });

    const result = await adapter.listThreadMessages(scope, threadId);

    // Check if we were aborted
    if (abortControllerRef.current?.signal.aborted) {
      return;
    }

    if (isOk(result)) {
      const transformed = parsePersistedMessages(result.value.map(threadMessageToUiMessage), "ui");

      replaceHistory(transformed);

      // Scroll to bottom after messages load (RAF-throttled)
      if (messagesEndRef?.current) {
        rafThrottle(`scroll-load-${threadId}`, () => {
          messagesEndRef.current?.scrollIntoView({ behavior: "instant" });
        });
      }

      // Defer non-critical work to idle time
      scheduleIdle(
        `messages-loaded-${threadId}`,
        () => {
          onMessagesLoadedRef.current?.();
        },
        "low"
      );
    } else {
      if (result.error.code === "NOT_FOUND") {
        replaceHistory([]);
        setLoadPhase({ phase: "ready" });
        abortControllerRef.current = null;
        return;
      }
      setLoadPhase({ phase: "failed", reason: result.error.message });
      onErrorRef.current?.(result.error.message);
    }

    abortControllerRef.current = null;
  }, [threadId, adapter, scope, replaceHistory, setLoadPhase, messagesEndRef]);

  // Effect: Load messages when threadId changes
  useEffect(() => {
    loadMessages();

    return () => {
      // Cleanup: abort any in-flight request
      if (abortControllerRef.current) {
        abortControllerRef.current.abort();
        abortControllerRef.current = null;
      }
    };
  }, [loadMessages]);

  return {
    isLoading,
    reloadMessages: loadMessages,
  };
}

// ============================================================================
// Auto-Scroll Effect Hook
// ============================================================================

interface UseAutoScrollOptions {
  /** Scroll container ref */
  scrollContainerRef: React.RefObject<HTMLElement | null>;
  /** Messages end ref for scroll anchor */
  messagesEndRef: React.RefObject<HTMLElement | null>;
  /** Number of messages (trigger) */
  messageCount: number;
  /** Number of streaming parts (trigger) */
  streamingPartsCount: number;
  /** Whether streaming is active */
  isStreaming: boolean;
  /** Check if user has manually scrolled */
  isManuallyScrolled: () => boolean;
  /** Smooth scroll to bottom */
  scrollToBottom: () => void;
  /** Instant scroll to bottom */
  scrollToBottomInstant: () => void;
}

/**
 * Auto-scroll effect separated from main component.
 *
 * Handles:
 * - Smooth scroll when new messages arrive
 * - Instant scroll during streaming (no animation lag)
 * - Respects manual scroll (user scrolled up)
 */
export function useAutoScroll({
  messageCount,
  streamingPartsCount,
  isStreaming,
  isManuallyScrolled,
  scrollToBottom,
  scrollToBottomInstant,
}: UseAutoScrollOptions): void {
  // biome-ignore lint/correctness/useExhaustiveDependencies: messageCount and streamingPartsCount are intentional triggers
  useEffect(() => {
    if (isManuallyScrolled()) return;

    if (isStreaming) {
      // Instant scroll during streaming (no animation lag)
      scrollToBottomInstant();
    } else {
      // Smooth scroll for new messages
      scrollToBottom();
    }
  }, [
    messageCount,
    streamingPartsCount,
    isStreaming,
    scrollToBottom,
    scrollToBottomInstant,
    isManuallyScrolled,
  ]);
}

// ============================================================================
// Status Transition Hook (DRY)
// ============================================================================

type StatusState = "idle" | "loading" | "ready" | "error";

interface UseStatusTransitionOptions {
  /** Current status */
  status: StatusState;
  /** Callback when status becomes 'ready' */
  onReady?: () => void;
  /** Callback when status becomes 'error' */
  onError?: () => void;
  /** Callback when status becomes 'loading' */
  onLoading?: () => void;
}

/**
 * Reusable status transition hook.
 *
 * DRY principle - use this instead of duplicating status effect logic.
 */
export function useStatusTransition({
  status,
  onReady,
  onError,
  onLoading,
}: UseStatusTransitionOptions): void {
  const prevStatusRef = useRef<StatusState>(status);

  useEffect(() => {
    const prevStatus = prevStatusRef.current;
    prevStatusRef.current = status;

    // Only fire callbacks on transitions
    if (prevStatus === status) return;

    switch (status) {
      case "ready":
        onReady?.();
        break;
      case "error":
        onError?.();
        break;
      case "loading":
        onLoading?.();
        break;
    }
  }, [status, onReady, onError, onLoading]);
}

// ============================================================================
// First Token Tracking Hook
// ============================================================================

interface UseFirstTokenTrackingOptions {
  /** Whether streaming is active */
  isStreaming: boolean;
  /** Current streaming text (trigger) */
  streamingText: string;
  /** Callback when first token appears */
  onFirstToken?: () => void;
}

/**
 * Track first token appearance during streaming.
 * Defers callback to idle time.
 */
export function useFirstTokenTracking({
  isStreaming,
  streamingText,
  onFirstToken,
}: UseFirstTokenTrackingOptions): void {
  const hasTrackedRef = useRef(false);

  useEffect(() => {
    // Reset on new stream
    if (!isStreaming) {
      hasTrackedRef.current = false;
      return;
    }

    // Track first token
    if (isStreaming && streamingText.length > 0 && !hasTrackedRef.current) {
      hasTrackedRef.current = true;

      // Defer to idle time
      scheduleIdle(
        "first-token-track",
        () => {
          onFirstToken?.();
        },
        "high"
      );
    }
  }, [isStreaming, streamingText, onFirstToken]);
}

// ============================================================================
// Tool Tracking Sync Hook
// ============================================================================

interface UseToolTrackingSyncOptions {
  /** Streaming parts to sync from */
  streamingParts: Array<{ type: string; toolCallId?: string; toolName?: string; state?: string }>;
  /** Callback when tool starts */
  onToolStart?: (toolCallId: string, toolName: string) => void;
  /** Callback when tool completes */
  onToolComplete?: (toolCallId: string) => void;
}

/**
 * Sync tool tracking from streaming parts.
 * Uses two-pass approach: detect agents first, then track tools.
 */
export function useToolTrackingSync({
  streamingParts,
  onToolStart,
  onToolComplete,
}: UseToolTrackingSyncOptions): void {
  const trackedToolsRef = useRef<Set<string>>(new Set());
  const completedToolsRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    if (streamingParts.length === 0) return;

    // Defer to idle time for non-critical tracking
    scheduleIdle(
      "tool-tracking-sync",
      () => {
        for (const part of streamingParts) {
          if (part.type === "tool-invocation" && part.toolCallId && part.toolName) {
            // Track start
            if (!trackedToolsRef.current.has(part.toolCallId)) {
              trackedToolsRef.current.add(part.toolCallId);
              onToolStart?.(part.toolCallId, part.toolName);
            }

            // Track completion
            if (part.state === "result" && !completedToolsRef.current.has(part.toolCallId)) {
              completedToolsRef.current.add(part.toolCallId);
              onToolComplete?.(part.toolCallId);
            }
          }
        }
      },
      "normal"
    );
  }, [streamingParts, onToolStart, onToolComplete]);

  // Reset on unmount or new stream
  useEffect(() => {
    return () => {
      trackedToolsRef.current.clear();
      completedToolsRef.current.clear();
    };
  }, []);
}

// ============================================================================
// Thread Touch Hook
// ============================================================================

interface UseThreadTouchOptions {
  /** Thread ID */
  threadId: string;
  /** Whether streaming is active */
  isStreaming: boolean;
  /** Callback to update thread timestamp */
  onThreadTouch?: (threadId: string) => void;
}

/**
 * Update thread timestamp when streaming completes.
 * Defers the update to avoid blocking UI.
 */
export function useThreadTouch({
  threadId,
  isStreaming,
  onThreadTouch,
}: UseThreadTouchOptions): void {
  const wasStreamingRef = useRef(false);

  useEffect(() => {
    const wasStreaming = wasStreamingRef.current;
    wasStreamingRef.current = isStreaming;

    // Detect transition from streaming to not streaming (completion)
    if (wasStreaming && !isStreaming && onThreadTouch) {
      scheduleIdle(
        `thread-touch-${threadId}`,
        () => {
          onThreadTouch(threadId);
        },
        "low"
      );
    }
  }, [threadId, isStreaming, onThreadTouch]);
}

// ============================================================================
// Debounced Effect Hook
// ============================================================================

/**
 * Run an effect with debouncing.
 * Useful for expensive operations that shouldn't run on every render.
 */
export function useDebouncedEffect(
  effect: () => undefined | (() => void),
  deps: React.DependencyList,
  delay: number
): void {
  // biome-ignore lint/correctness/useExhaustiveDependencies: effect and deps are intentional
  useEffect(() => {
    const timer = setTimeout(effect, delay);
    return () => clearTimeout(timer);
  }, [...deps, delay]);
}

// ============================================================================
// Cleanup on Unmount Hook
// ============================================================================

/**
 * Run cleanup function on unmount.
 * Useful for cancelling subscriptions, aborting requests, etc.
 */
export function useUnmountCleanup(cleanup: () => void): void {
  const cleanupRef = useRef(cleanup);
  cleanupRef.current = cleanup;

  useEffect(() => {
    return () => {
      cleanupRef.current();
    };
  }, []);
}
