/**
 * Hook for loading sub-agent message history.
 *
 * Sub-agent message expansion pattern.
 *
 * Use this hook to lazily load detailed tool call history
 * when the user expands a sub-agent execution in the UI.
 *
 * @module hooks/use-sub-agent-messages
 */

import { useCallback, useRef, useState } from "react";
import { fetchSubAgentMessages } from "~/lib/api";
import type { Message } from "~/lib/domain/message";
import { parsePersistedMessages } from "~/lib/domain/message";
import { isOk } from "~/lib/domain/result";
import { scheduleIdle } from "~/lib/perf/scheduler";

// ============================================================================
// Types
// ============================================================================

/**
 * Loading state for sub-agent messages.
 */
export type SubAgentLoadingState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "loaded"; messages: Message[]; format: "ui" | "backend" }
  | { status: "error"; error: string };

/**
 * Cache entry for sub-agent messages.
 */
interface CacheEntry {
  readonly messages: Message[];
  readonly format: "ui" | "backend";
  readonly loadedAt: number;
}

/**
 * Hook options.
 */
export interface UseSubAgentMessagesOptions {
  /** Cache TTL in milliseconds (default: 5 minutes) */
  cacheTtl?: number;
  /** Callback when messages are loaded */
  onLoad?: (messages: Message[]) => void;
  /** Callback when error occurs */
  onError?: (error: string) => void;
}

/**
 * Hook return type.
 */
export interface UseSubAgentMessagesReturn {
  /** Load messages for a sub-agent */
  loadMessages: (threadId: string, agentId: string) => Promise<void>;
  /** Current loading state */
  state: SubAgentLoadingState;
  /** Clear cached messages */
  clearCache: () => void;
  /** Check if messages are cached */
  isCached: (threadId: string, agentId: string) => boolean;
}

// ============================================================================
// Hook Implementation
// ============================================================================

/**
 * Hook for lazily loading sub-agent message history.
 *
 * Features:
 * - In-memory caching with TTL
 * - Abort signal support for cleanup
 * - Idle callback for non-critical updates
 *
 * @example
 * ```tsx
 * const { loadMessages, state } = useSubAgentMessages();
 *
 * const handleExpand = async (threadId: string, agentId: string) => {
 *   await loadMessages(threadId, agentId);
 * };
 *
 * // Render based on state
 * if (state.status === 'loading') return <Spinner />;
 * if (state.status === 'loaded') return <MessageList messages={state.messages} />;
 * ```
 */
export function useSubAgentMessages(
  options: UseSubAgentMessagesOptions = {}
): UseSubAgentMessagesReturn {
  const { cacheTtl = 5 * 60 * 1000, onLoad, onError } = options;

  const [state, setState] = useState<SubAgentLoadingState>({ status: "idle" });
  const cacheRef = useRef<Map<string, CacheEntry>>(new Map());
  const abortControllerRef = useRef<AbortController | null>(null);

  /**
   * Generate cache key from threadId and agentId.
   */
  const getCacheKey = useCallback((threadId: string, agentId: string): string => {
    return `${threadId}:${agentId}`;
  }, []);

  /**
   * Check if messages are cached and not expired.
   */
  const isCached = useCallback(
    (threadId: string, agentId: string): boolean => {
      const key = getCacheKey(threadId, agentId);
      const entry = cacheRef.current.get(key);
      if (!entry) return false;
      return Date.now() - entry.loadedAt < cacheTtl;
    },
    [getCacheKey, cacheTtl]
  );

  /**
   * Load messages for a sub-agent.
   */
  const loadMessages = useCallback(
    async (threadId: string, agentId: string): Promise<void> => {
      const key = getCacheKey(threadId, agentId);

      // Check cache first
      const cached = cacheRef.current.get(key);
      if (cached && Date.now() - cached.loadedAt < cacheTtl) {
        setState({
          status: "loaded",
          messages: cached.messages,
          format: cached.format,
        });
        return;
      }

      // Abort any in-flight request
      if (abortControllerRef.current) {
        abortControllerRef.current.abort();
      }

      abortControllerRef.current = new AbortController();
      setState({ status: "loading" });

      const result = await fetchSubAgentMessages(
        threadId,
        agentId,
        abortControllerRef.current.signal
      );

      // Check if we were aborted
      if (abortControllerRef.current?.signal.aborted) {
        return;
      }

      if (isOk(result)) {
        const { format } = result.value;
        const messages = parsePersistedMessages(result.value.messages, format);

        // Cache the result
        cacheRef.current.set(key, {
          messages,
          format,
          loadedAt: Date.now(),
        });

        setState({ status: "loaded", messages, format });

        // Defer callback to idle time
        if (onLoad) {
          scheduleIdle(
            `sub-agent-loaded-${key}`,
            () => {
              onLoad(messages);
            },
            "low"
          );
        }
      } else {
        const errorMessage = result.error.message;
        setState({ status: "error", error: errorMessage });
        onError?.(errorMessage);
      }

      abortControllerRef.current = null;
    },
    [getCacheKey, cacheTtl, onLoad, onError]
  );

  /**
   * Clear the message cache.
   */
  const clearCache = useCallback(() => {
    cacheRef.current.clear();
    setState({ status: "idle" });
  }, []);

  return {
    loadMessages,
    state,
    clearCache,
    isCached,
  };
}

// ============================================================================
// Utility Functions
// ============================================================================

/**
 * Extract tool invocations from sub-agent messages.
 */
export function extractToolInvocations(
  messages: readonly Message[]
): Array<{ toolName: string; toolCallId: string; args: unknown; result?: unknown }> {
  const tools: Array<{ toolName: string; toolCallId: string; args: unknown; result?: unknown }> =
    [];

  for (const msg of messages) {
    for (const part of msg.parts) {
      if (part.type === "tool-invocation") {
        tools.push({
          toolName: part.toolName,
          toolCallId: part.toolCallId,
          args: part.args,
          result: part.result,
        });
      }
    }
  }

  return tools;
}

/**
 * Count tool invocations by tool name.
 */
export function countToolsByName(messages: readonly Message[]): Map<string, number> {
  const counts = new Map<string, number>();
  const tools = extractToolInvocations(messages);

  for (const tool of tools) {
    const current = counts.get(tool.toolName) ?? 0;
    counts.set(tool.toolName, current + 1);
  }

  return counts;
}
