/**
 * Chat area component.
 *
 * Performance optimizations:
 * - RAF-throttled auto-scroll
 * - requestIdleCallback for non-critical work
 * - Split stable/streaming messages
 * - Stable callback references
 * - Virtualized message list
 * - Optimized scroll detection
 *
 * @module components/chat/chat-area
 */

import { AlertCircleIcon, ClipboardCheckIcon, DownloadIcon } from "lucide-react";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router";
import { createFromChat } from "~/lib/api";
import { MessageId } from "~/lib/domain/message";
import { isOk } from "~/lib/domain/result";
import { generateTitle } from "~/lib/domain/thread";
import { createUserActionMessage, UserActionType } from "~/lib/domain/user-action";
import { useAutoScroll, useChatEffects } from "~/lib/hooks/use-chat-effects";
import { useLatencyTracker } from "~/lib/hooks/use-latency-tracker";
import { useOptimizedScroll } from "~/lib/hooks/use-optimized-scroll";
import { usePendingAutoSend } from "~/lib/hooks/use-pending-auto-send";
import { useTraceCollection } from "~/lib/hooks/use-trace-collection";
import { scheduleIdle } from "~/lib/perf/scheduler";
import {
  createLocalThread,
  pickChatAgentId,
  studioEventToStreamPart,
  useHarnessRuntime,
} from "~/lib/runtime";
import {
  selectActiveWorkspaceId,
  selectAgentCustomEndpoints,
  selectAgentModels,
  selectAgentSelectedModels,
  selectAvailableModels,
  selectConversationIsStreaming,
  selectConversationLoading,
  selectFinishReason,
  selectHasServerMetrics,
  selectHistory,
  selectLiveParts,
  selectModelSettings,
  selectServerMetrics,
  selectShowLatencyPanel,
  selectTokenCost,
  useAgentConfig,
  useConversation,
  useConversationActions,
  useSessionRegistry,
  useStreamMetrics,
  useThreadActions,
  useWorkspace,
} from "~/lib/stores";
import { getFlowAIStudioConfig } from "~/lib/studio-config/flowai-config";
import { cn } from "~/lib/utils";
import { ChatContext } from "./chat-context";
import { type ClientLatencyMetrics, LatencyPanel } from "./latency-panel";
import { LatencySummaryDisplay } from "./latency-summary-display";
import { MessageInput } from "./message-input";
import { StreamingMessage } from "./streaming-message";
import { type ChatPreset, VirtualizedMessageList } from "./virtualized-message-list";

// ============================================================================
// Props
// ============================================================================

interface ChatAreaProps {
  threadId: string;
}

interface ResponseContractInfo {
  readonly role: string;
  readonly modelName: string;
  readonly modelRef: string;
}

interface RoleTopologyInfo {
  readonly role: string;
  readonly delegatesTo: readonly string[];
  readonly tools: readonly string[];
}

const EMPTY_CHAT_PRESETS: ChatPreset[] = [];
const EMPTY_RESPONSE_CONTRACTS: readonly ResponseContractInfo[] = [];
const EMPTY_ROLE_TOPOLOGY: readonly RoleTopologyInfo[] = [];

// ============================================================================
// Loading Skeleton (memoized)
// ============================================================================

const LoadingSkeleton = memo(function LoadingSkeleton() {
  return (
    <div className="flex-1 flex flex-col justify-end p-6 space-y-4 max-w-3xl mx-auto w-full">
      {/* Skeleton message bubbles */}
      {Array.from({ length: 3 }, (_, i) => (
        <div
          key={`msg-skel-${i}`}
          className={cn("flex gap-3", i % 2 === 0 ? "justify-end" : "justify-start")}
          style={{ animationDelay: `${i * 100}ms` }}
        >
          <div className={cn("space-y-1.5 max-w-[70%]", i % 2 === 0 ? "items-end" : "items-start")}>
            <div
              className="h-3.5 bg-muted rounded animate-pulse"
              style={{ width: `${180 - i * 30}px` }}
            />
            {i !== 2 && (
              <div
                className="h-3.5 bg-muted/60 rounded animate-pulse"
                style={{ width: `${120 - i * 20}px` }}
              />
            )}
          </div>
        </div>
      ))}
    </div>
  );
});

// ============================================================================
// Error Display (memoized)
// ============================================================================

interface ErrorDisplayProps {
  error: string;
  onDismiss?: () => void;
  onRetry?: () => void;
}

const ErrorDisplay = memo(function ErrorDisplay({ error, onDismiss, onRetry }: ErrorDisplayProps) {
  return (
    <div className="max-w-3xl mx-auto mt-4">
      <div className="bg-[var(--accent-amber)] text-[var(--dot-amber)] rounded-lg p-3 text-sm flex items-start gap-2">
        <AlertCircleIcon className="size-4 flex-shrink-0 mt-0.5" />
        <div className="flex-1">{error}</div>
        <div className="flex gap-1.5 shrink-0">
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="px-2 py-1 text-xs font-medium bg-[var(--dot-amber)]/20 hover:bg-[var(--dot-amber)]/30 rounded transition-colors"
            >
              Retry
            </button>
          )}
          {onDismiss && (
            <button
              type="button"
              onClick={onDismiss}
              className="px-2 py-1 text-xs font-medium text-[var(--dot-amber)]/70 hover:text-[var(--dot-amber)] rounded transition-colors"
            >
              Dismiss
            </button>
          )}
        </div>
      </div>
    </div>
  );
});

// ============================================================================
// Main Component
// ============================================================================

export function ChatArea({ threadId }: ChatAreaProps) {
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [lastUserMessage, setLastUserMessage] = useState<string | null>(null);
  const [pendingInput, setPendingInput] = useState<string | undefined>();
  const abortControllerRef = useRef<AbortController | null>(null);
  const activeSessionIdRef = useRef<string | null>(null);
  const activeRunIdRef = useRef<string | null>(null);
  const activeWorkspaceId = useWorkspace(selectActiveWorkspaceId);
  const { adapter, scope } = useHarnessRuntime();
  const projectName = useMemo(() => getFlowAIStudioConfig().appName, []);

  // Use ref for isStreaming to avoid callback invalidation
  const isStreamingRef = useRef(false);

  // Store selectors (granular selectors for state)
  const messages = useConversation(selectHistory);
  const isStreaming = useConversation(selectConversationIsStreaming);
  const streamingParts = useConversation(selectLiveParts);
  const isLoading = useConversation(selectConversationLoading) as boolean;
  const backendLatency = useConversation(selectServerMetrics);
  const hasBackendLatency = useConversation(selectHasServerMetrics);
  const costSummary = useConversation(selectTokenCost);
  const finishReason = useConversation(selectFinishReason);

  // Feature flags (granular selectors for settings)
  const showLatencyPanel = useAgentConfig(selectShowLatencyPanel);

  // Agent model selections — read reactively so settings changes apply on next message
  const agentModels = useAgentConfig(selectAgentModels);
  const agentSelectedModels = useAgentConfig(selectAgentSelectedModels);
  const agentCustomEndpoints = useAgentConfig(selectAgentCustomEndpoints);
  const availableModels = useAgentConfig(selectAvailableModels);
  const modelSettings = useAgentConfig(selectModelSettings);

  // Keep ref in sync
  isStreamingRef.current = isStreaming;

  // Thread store: for auto-titling
  const { getThread, addThread, updateThread } = useThreadActions();
  const markChatSessionTerminal = useCallback((status: "completed" | "failed" | "cancelled") => {
    const sessionId = activeSessionIdRef.current;
    if (!sessionId) return;
    useSessionRegistry.getState().markTerminal(sessionId, status);
    activeSessionIdRef.current = null;
  }, []);

  // Store actions via action bundle hooks (stable references)
  const {
    sendMessage: addUserMessage,
    startStream: startStreamingRaw,
    interpretEvent: interpretStreamEventRaw,
    finishStream: finishStreamRaw,
    abortStream: abortStreamRaw,
  } = useConversationActions();
  const { recordFirstChunk: recordFirstChunkRaw, recordFirstToken: recordFirstTokenRaw } =
    useStreamMetrics();

  // Bind session-scoped actions to this threadId
  const startStreaming = useCallback(
    (abortController: AbortController, messageId: MessageId, now?: number) =>
      startStreamingRaw(threadId, abortController, messageId, now),
    [startStreamingRaw, threadId]
  );
  const interpretStreamEvent = useCallback(
    (part: import("~/lib/domain").StreamPart) => interpretStreamEventRaw(threadId, part),
    [interpretStreamEventRaw, threadId]
  );
  const finishStream = useCallback(
    (now?: string) => finishStreamRaw(threadId, now, activeRunIdRef.current ?? undefined),
    [finishStreamRaw, threadId]
  );
  const abortStream = useCallback(
    (now?: string) => abortStreamRaw(threadId, now),
    [abortStreamRaw, threadId]
  );
  const recordFirstChunk = useCallback(
    () => recordFirstChunkRaw(threadId),
    [recordFirstChunkRaw, threadId]
  );
  const recordFirstToken = useCallback(
    () => recordFirstTokenRaw(threadId),
    [recordFirstTokenRaw, threadId]
  );

  // Performance hooks
  const latencyTracker = useLatencyTracker();
  const { scrollToBottom, scrollToBottomInstant, isManuallyScrolled } = useOptimizedScroll(
    scrollContainerRef,
    { scrollId: `chat-${threadId}` }
  );
  const handleMessagesLoaded = useCallback(() => {
    setError(null);
  }, []);
  const handleLoadError = useCallback((message: string) => {
    setError(message);
  }, []);

  // Automatic trace collection from backend latency
  useTraceCollection();

  // Compute streaming message ID for virtualization
  const streamingMessageId = useMemo(() => {
    if (isStreaming && messages.length > 0) {
      return messages[messages.length - 1]?.id;
    }
    return undefined;
  }, [isStreaming, messages]);

  // Memoized client latency metrics for LatencyPanel
  // Re-computes when streaming state or parts change (triggers re-render with fresh ref values)
  const clientLatencyMetrics = useMemo((): ClientLatencyMetrics => {
    const metrics = latencyTracker.getMetrics();
    return {
      timeToFirstChunk: metrics.timeToFirstChunk,
      timeToFirstToken: metrics.timeToFirstToken ?? metrics.timeToFirstChunk,
      submitTime: metrics.submitTime,
      chunkCount: metrics.chunkCount,
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [latencyTracker.getMetrics]);

  useChatEffects({
    threadId,
    messagesEndRef,
    onMessagesLoaded: handleMessagesLoaded,
    onError: handleLoadError,
  });

  useAutoScroll({
    scrollContainerRef,
    messagesEndRef,
    messageCount: messages.length,
    streamingPartsCount: streamingParts.length,
    isStreaming,
    isManuallyScrolled,
    scrollToBottom,
    scrollToBottomInstant,
  });

  // Handle sending a message
  const handleSendMessage = useCallback(
    async (content: string) => {
      if (!content.trim() || isStreamingRef.current) return;
      isStreamingRef.current = true;
      let streamStarted = false;

      try {
        // Track latency
        latencyTracker.trackSubmit();

        // Resolve per-agent model and endpoint overrides before mutating UI state.
        // The runtime contract is explicit: a custom endpoint must carry a
        // declared transport from provider metadata.
        const resolvedAgentModels: Record<string, string> = {};
        const resolvedAgentEndpoints: Record<
          string,
          { transport: string; settings: Record<string, string>; targetModel?: string }
        > = {};
        for (const role of Object.keys(agentModels)) {
          const providerKey = agentModels[role];
          const selected = agentSelectedModels[role];
          const customEndpoint = agentCustomEndpoints[role];
          const config = availableModels.find((m) => m.key === providerKey);

          if (selected) {
            resolvedAgentModels[role] = `${providerKey}/${selected}`;
          } else if (config) {
            resolvedAgentModels[role] = config.model;
          }

          const customEndpointSettings = customEndpoint?.settings;
          const hasCustomEndpointSettings =
            !!customEndpointSettings && Object.keys(customEndpointSettings).length > 0;

          if (hasCustomEndpointSettings) {
            if (!config?.endpointTransport) {
              setError(
                `Provider "${config?.displayName || providerKey}" does not declare a custom endpoint transport.`
              );
              isStreamingRef.current = false;
              return;
            }

            resolvedAgentEndpoints[role] = {
              transport: config.endpointTransport,
              settings: customEndpointSettings,
              ...(customEndpoint.targetModel ? { targetModel: customEndpoint.targetModel } : {}),
            };
          }
        }

        // Auto-title: local UI state only. The harness runtime persists the
        // thread on first stream and derives its server-side title from prompt.
        if (messages.length === 0) {
          const thread = getThread(threadId);
          if (!thread || thread.title === "New Conversation" || !thread.title) {
            const title = generateTitle(content);
            if (thread) {
              updateThread(threadId, { title });
            } else {
              addThread(createLocalThread(threadId, activeWorkspaceId, title));
            }
          }
        }

        // Add user message to store
        addUserMessage(content, MessageId(crypto.randomUUID()), new Date().toISOString());
        setLastUserMessage(content);
        setPendingInput(undefined);
        setError(null);

        // Create abort controller for cancellation
        abortControllerRef.current = new AbortController();
        const messageId = MessageId(crypto.randomUUID());

        // Start streaming
        startStreaming(abortControllerRef.current, messageId);
        streamStarted = true;
        activeSessionIdRef.current = `chat-${threadId}`;
        useSessionRegistry.getState().register({
          id: activeSessionIdRef.current,
          kind: "chat-stream",
          label: `Thread ${threadId.slice(0, 8)}`,
          workspaceId: activeWorkspaceId,
          startedAt: Date.now(),
          routeTo: `/playground/${threadId}`,
        });

        const agentsResult = await adapter.listAgents(scope);
        if (!isOk(agentsResult)) {
          throw new Error(agentsResult.error.message);
        }
        const agentId = pickChatAgentId(agentsResult.value.agents);
        if (!agentId) {
          throw new Error("No Studio chat agent is registered for this workspace.");
        }

        // Start stream
        let firstEvent = true;
        activeRunIdRef.current = null;
        const result = await adapter.startChatStream(
          {
            agentId,
            prompt: content,
            threadId,
            metadata:
              Object.keys(resolvedAgentModels).length > 0 ||
              Object.keys(resolvedAgentEndpoints).length > 0 ||
              modelSettings.maxTokens > 0 ||
              modelSettings.thinkingBudgetTokens >= 0
                ? {
                    ...(Object.keys(resolvedAgentModels).length > 0
                      ? { agentModels: resolvedAgentModels }
                      : {}),
                    ...(Object.keys(resolvedAgentEndpoints).length > 0
                      ? { agentEndpoints: resolvedAgentEndpoints }
                      : {}),
                    modelSettings,
                  }
                : undefined,
            handlers: {
              onEvent: (event) => {
                activeRunIdRef.current = event.runId;
                if (firstEvent) {
                  firstEvent = false;
                  recordFirstChunk();
                  latencyTracker.trackFirstChunk();
                }
                const part = studioEventToStreamPart(event);
                if (!part) return;
                interpretStreamEvent(part);

                // Track chunks
                latencyTracker.trackChunk();

                // Track first text token (distinct from first chunk which may be a tool invocation)
                if (part.type === "text") {
                  recordFirstToken();
                  latencyTracker.trackFirstToken();
                }
              },
              onComplete: () => {
                finishStream();
                latencyTracker.trackComplete();
                abortControllerRef.current = null;
                activeRunIdRef.current = null;
                isStreamingRef.current = false;
                markChatSessionTerminal("completed");

                // Defer non-critical work to idle time
                scheduleIdle(
                  `complete-${threadId}`,
                  () => {
                    // Any post-completion cleanup
                  },
                  "low"
                );
              },
              onError: (err) => {
                setError(err.message);
                finishStream();
                abortControllerRef.current = null;
                activeRunIdRef.current = null;
                isStreamingRef.current = false;
                markChatSessionTerminal("failed");
              },
            },
          },
          {
            signal: abortControllerRef.current.signal,
            scope,
          }
        );

        if (!isOk(result)) {
          setError(result.error.message);
          finishStream();
          abortControllerRef.current = null;
          activeRunIdRef.current = null;
          isStreamingRef.current = false;
          markChatSessionTerminal("failed");
        }
      } catch (err) {
        setError(err instanceof Error ? err.message : "Failed to send message");
        if (streamStarted) {
          finishStream();
        }
        abortControllerRef.current = null;
        activeRunIdRef.current = null;
        isStreamingRef.current = false;
        if (streamStarted) {
          markChatSessionTerminal("failed");
        }
      }
    },
    [
      threadId,
      messages,
      addUserMessage,
      startStreaming,
      interpretStreamEvent,
      finishStream,
      recordFirstChunk,
      recordFirstToken,
      latencyTracker,
      getThread,
      updateThread,
      addThread,
      agentModels,
      agentSelectedModels,
      agentCustomEndpoints,
      availableModels,
      modelSettings,
      markChatSessionTerminal,
      activeWorkspaceId,
      adapter,
      scope,
    ]
  );

  // Cross-tab autoSend: fire once after messages load, then clear URL param.
  // Avoids race condition from fire-and-forget streams (Test -> Chat, Eval -> Chat, Data -> Chat).
  usePendingAutoSend({ isLoading, isStreaming, onSend: handleSendMessage });

  // Handle cancel
  const handleCancel = useCallback(() => {
    if (abortControllerRef.current) {
      abortControllerRef.current.abort();
      abortControllerRef.current = null;
    }
    isStreamingRef.current = false;
    abortStream(new Date().toISOString());
    markChatSessionTerminal("cancelled");
  }, [abortStream, markChatSessionTerminal]);

  // Dismiss error
  const handleDismissError = useCallback(() => {
    setError(null);
  }, []);

  // Retry last message on error
  const handleRetry = useCallback(() => {
    if (!lastUserMessage) return;
    setError(null);
    handleSendMessage(lastUserMessage);
  }, [lastUserMessage, handleSendMessage]);

  // Handle preset button click (populate input)
  const handlePresetClick = useCallback((text: string) => {
    setPendingInput(text);
  }, []);

  // Handle CommandCard action button clicks (e.g., proceed_plan, cancel_plan)
  const actionSentRef = useRef(false);
  const handleCommandCardAction = useCallback(
    (actionId: string, metadata?: { planId?: string }) => {
      // Guard: streaming or already sent an action this turn
      if (isStreamingRef.current || actionSentRef.current) return;

      // Validate action type
      const validActions = Object.values(UserActionType) as string[];
      if (!validActions.includes(actionId)) return;

      actionSentRef.current = true;
      const message = createUserActionMessage(actionId as UserActionType, {
        planId: metadata?.planId,
      });
      handleSendMessage(message);
    },
    [handleSendMessage]
  );

  // Reset action-sent guard when streaming completes
  useEffect(() => {
    if (!isStreaming) {
      actionSentRef.current = false;
    }
  }, [isStreaming]);

  // Export chat as markdown
  const handleExportMarkdown = useCallback(() => {
    const lines: string[] = [];
    for (const msg of messages) {
      const role = msg.role === "user" ? "User" : "Assistant";
      lines.push(`## ${role}\n`);
      for (const part of msg.parts) {
        if (part.type === "text") {
          lines.push((part as { type: "text"; text: string }).text);
        } else if (part.type === "tool-invocation") {
          const tp = part as { type: "tool-invocation"; toolName: string; result?: string };
          lines.push(`> **Tool:** ${tp.toolName}`);
          if (tp.result) lines.push(`> ${tp.result}`);
        }
      }
      lines.push(""); // blank line between messages
    }
    const md = lines.join("\n");
    const blob = new Blob([md], { type: "text/markdown" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `chat-${threadId}-${Date.now()}.md`;
    a.click();
    URL.revokeObjectURL(url);
  }, [messages, threadId]);

  // Save the current conversation as a draft test case (harness from-chat).
  const navigate = useNavigate();
  const [savingTest, setSavingTest] = useState(false);
  const handleSaveAsTest = useCallback(async () => {
    setSavingTest(true);
    const result = await createFromChat(threadId);
    setSavingTest(false);
    if (isOk(result)) {
      navigate(`/tests/${result.value.id}`);
    } else {
      setError(`Failed to save as test case: ${result.error.message}`);
    }
  }, [threadId, navigate]);

  // Chat context value (memoized to prevent unnecessary re-renders)
  const chatContextValue = useMemo(
    () => ({
      onCommandCardAction: handleCommandCardAction,
      isStreaming,
    }),
    [handleCommandCardAction, isStreaming]
  );

  return (
    <ChatContext.Provider value={chatContextValue}>
      <div className="flex min-h-0 flex-1 flex-col min-w-0 overflow-hidden">
        {/* Action Bar */}
        {messages.length > 0 && !isLoading && (
          <div className="flex items-center justify-end gap-2 px-4 py-1.5 border-b">
            <button
              type="button"
              onClick={handleSaveAsTest}
              disabled={isStreaming || savingTest}
              className="flex items-center gap-1.5 px-2.5 py-1 text-xs rounded-md text-muted-foreground hover:bg-muted hover:text-foreground transition-colors disabled:opacity-50"
              title="Save this conversation as a draft test case"
            >
              <ClipboardCheckIcon className="size-3.5" />
              {savingTest ? "Saving…" : "Save as test case"}
            </button>
            <button
              type="button"
              onClick={handleExportMarkdown}
              disabled={isStreaming}
              className="flex items-center gap-1.5 px-2.5 py-1 text-xs rounded-md text-muted-foreground hover:bg-muted hover:text-foreground transition-colors disabled:opacity-50"
              title="Export chat as markdown"
            >
              <DownloadIcon className="size-3.5" />
              Export
            </button>
          </div>
        )}

        {/* Messages Area */}
        <div
          ref={scrollContainerRef}
          className="min-h-0 flex-1 overflow-y-auto scroll-container p-4"
        >
          {isLoading ? (
            <LoadingSkeleton />
          ) : (
            <>
              <VirtualizedMessageList
                messages={messages}
                streamingMessageId={streamingMessageId}
                scrollContainerRef={scrollContainerRef}
                onPresetClick={handlePresetClick}
                presets={EMPTY_CHAT_PRESETS}
                projectName={projectName}
                responseContracts={EMPTY_RESPONSE_CONTRACTS}
                roleTopology={EMPTY_ROLE_TOPOLOGY}
              />

              {/* Streaming Preview (rendered outside virtualization) */}
              {isStreaming && <StreamingMessage parts={streamingParts} />}

              {/* Error Display */}
              {error && (
                <ErrorDisplay
                  error={error}
                  onDismiss={handleDismissError}
                  onRetry={lastUserMessage ? handleRetry : undefined}
                />
              )}

              {/* Latency Summary (pure view of backend metrics) */}
              {hasBackendLatency && backendLatency && !isStreaming && (
                <div className="mt-4">
                  <LatencySummaryDisplay summary={backendLatency} />
                </div>
              )}

              {/* Scroll anchor */}
              <div ref={messagesEndRef} className="h-px" />
            </>
          )}
        </div>

        {/* Input Area */}
        <div className="border-t p-4">
          <div className="max-w-3xl mx-auto">
            <MessageInput
              onSend={handleSendMessage}
              onCancel={handleCancel}
              isStreaming={isStreaming}
              disabled={isLoading}
              pendingInput={pendingInput}
            />
          </div>
        </div>

        {/* Latency Panel (feature flag controlled) */}
        {showLatencyPanel && (
          <LatencyPanel
            clientMetrics={clientLatencyMetrics}
            backendLatency={backendLatency}
            costSummary={costSummary}
            finishReason={finishReason}
            isStreaming={isStreaming}
            position="bottom-right"
          />
        )}
      </div>
    </ChatContext.Provider>
  );
}
