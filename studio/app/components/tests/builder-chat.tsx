/**
 * Chat-based test case builder with live session state panel.
 *
 * Uses the test case builder agent (role="test_case_builder") to compose
 * trajectories interactively via streaming chat. A right-side
 * panel shows the current session state (trajectory + GT) as
 * it evolves, fetched from the backend after each exchange.
 *
 * Layout: two-column — chat (flex-1) | session panel (w-72)
 *
 * @module components/tests/builder-chat
 */

import {
  CheckCircle2Icon,
  CircleDotIcon,
  CircleIcon,
  ClipboardListIcon,
  ListOrderedIcon,
  RotateCcwIcon,
  SaveIcon,
  TargetIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router";
import { ChatContext } from "~/components/chat/chat-context";
import { MessageInput } from "~/components/chat/message-input";
import { DisplayPartDisplay } from "~/components/chat/message-part";
import { StreamingMessage } from "~/components/chat/streaming-message";
import { categoryColor } from "~/components/shared/category-badge";
import { ErrorBanner } from "~/components/shared/error-banner";
import type { ChatRequestMessage } from "~/lib/api";
import {
  clearTestCaseBuilderSession,
  getTestCaseBuilderSession,
  saveFromBuilder,
  startChatStream,
} from "~/lib/api";
import type { Message } from "~/lib/domain/message";
import { extractTextContent, groupParts, MessageId } from "~/lib/domain/message";
import { isOk } from "~/lib/domain/result";
import type { GroundTruth, TestCaseBuilderSession } from "~/lib/domain/test-case";
import {
  selectAvailableTools,
  selectBuilderError,
  selectBuilderHasMessages,
  selectBuilderIsStreaming,
  selectBuilderLiveParts,
  selectBuilderMessages,
  selectBuilderSession,
  selectBuilderSessionId,
  useBuilderChatActions,
  useTestSuite,
  useTestSuiteActions,
} from "~/lib/stores";
import { useWorkspace } from "~/lib/stores/workspace-store";
import { cn } from "~/lib/utils";

// ============================================================================
// Workflow Stepper
// ============================================================================

type WorkflowStep = "describe" | "compose" | "groundTruth" | "review";

interface StepDef {
  readonly key: WorkflowStep;
  readonly label: string;
  readonly description: string;
}

const WORKFLOW_STEPS: StepDef[] = [
  { key: "describe", label: "Describe", description: "Describe the scenario" },
  { key: "compose", label: "Trajectory", description: "Tool call sequence" },
  { key: "groundTruth", label: "Ground Truth", description: "Expected outcome" },
  { key: "review", label: "Save", description: "Review & save" },
];

/** Derive the current workflow step from session state. */
function deriveStep(hasMessages: boolean, session: TestCaseBuilderSession | null): WorkflowStep {
  if (!hasMessages) return "describe";
  if (!session || session.composedTrajectory.length === 0) return "compose";
  if (!session.structuredGroundTruth && !session.groundTruth) return "groundTruth";
  return "review";
}

function isStepComplete(step: WorkflowStep, currentStep: WorkflowStep): boolean {
  const order: WorkflowStep[] = ["describe", "compose", "groundTruth", "review"];
  return order.indexOf(step) < order.indexOf(currentStep);
}

function WorkflowStepper({ currentStep }: { readonly currentStep: WorkflowStep }) {
  return (
    <div className="flex items-center gap-1">
      {WORKFLOW_STEPS.map((step, i) => {
        const isActive = step.key === currentStep;
        const isComplete = isStepComplete(step.key, currentStep);

        return (
          <div key={step.key} className="flex items-center gap-1">
            {i > 0 && (
              <div
                className={cn("w-4 h-px", isComplete ? "bg-[var(--dot-emerald)]" : "bg-border")}
              />
            )}
            <div
              className={cn(
                "flex items-center gap-1.5 px-2 py-1 rounded-md text-[10px] font-medium transition-colors",
                isComplete && "text-[var(--dot-emerald)]",
                isActive && "text-primary bg-primary/10",
                !isComplete && !isActive && "text-muted-foreground/50"
              )}
              title={step.description}
            >
              {isComplete ? (
                <CheckCircle2Icon className="size-3 text-[var(--dot-emerald)]" />
              ) : isActive ? (
                <CircleDotIcon className="size-3" />
              ) : (
                <CircleIcon className="size-3" />
              )}
              <span className="hidden sm:inline">{step.label}</span>
            </div>
          </div>
        );
      })}
    </div>
  );
}

// ============================================================================
// Ground Truth Summary
// ============================================================================

function gtSummary(gt: GroundTruth): string {
  switch (gt.kind) {
    case "text":
      return `Text: "${gt.text.slice(0, 60)}${gt.text.length > 60 ? "..." : ""}"`;
    case "structured":
      return "Structured ground truth";
    case "textOnly":
      return `Text: "${gt.text.slice(0, 60)}${gt.text.length > 60 ? "..." : ""}"`;
    case "flat": {
      const n = gt.expectedActions.length;
      return `${n} action${n !== 1 ? "s" : ""} (structured)`;
    }
    case "multiGroup": {
      const g = gt.groups.length;
      const a = gt.groups.reduce((sum, group) => sum + group.actions.length, 0);
      return `${g} group${g !== 1 ? "s" : ""}, ${a} action${a !== 1 ? "s" : ""}`;
    }
  }
}

// ============================================================================
// Session State Panel
// ============================================================================

function SessionStatePanel({
  session,
  currentStep,
  availableTools,
}: {
  readonly session: TestCaseBuilderSession | null;
  readonly currentStep: WorkflowStep;
  readonly availableTools: ReadonlyArray<{ name: string; category: string }>;
}) {
  const toolCategoryMap = useMemo(() => {
    const m = new Map<string, string>();
    for (const t of availableTools) m.set(t.name, t.category);
    return m;
  }, [availableTools]);

  const trajectory = session?.composedTrajectory ?? [];
  const gt = session?.structuredGroundTruth ?? null;
  const legacyGt = session?.groundTruth;
  const mode = session?.trajectoryMode ?? "unordered";

  return (
    <div className="w-72 shrink-0 border-l flex flex-col min-h-0">
      {/* Header */}
      <div className="px-4 py-3 border-b">
        <div className="section-label">Session State</div>
        <WorkflowStepper currentStep={currentStep} />
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto scroll-container p-4 space-y-4">
        {/* Trajectory Section */}
        <div className="space-y-2">
          <div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
            <ListOrderedIcon className="size-3" />
            Trajectory
            {trajectory.length > 0 && (
              <span className="ml-auto text-[10px] font-mono tabular-nums">
                {trajectory.length} step{trajectory.length !== 1 ? "s" : ""}
              </span>
            )}
          </div>

          {trajectory.length === 0 ? (
            <div className="text-[10px] text-muted-foreground/50 pl-0.5">Not yet composed</div>
          ) : (
            <div className="space-y-1">
              {trajectory.map((step, i) => {
                const cat = toolCategoryMap.get(step.toolName);
                const color = categoryColor(cat);
                return (
                  <div
                    key={`${i}-${step.toolName}`}
                    className={cn(
                      "flex items-center gap-1.5 px-2 py-1 rounded text-[11px] font-mono border",
                      color.bg,
                      color.text,
                      color.border
                    )}
                  >
                    <span className="text-[9px] opacity-50 tabular-nums w-3 text-center shrink-0">
                      {i + 1}
                    </span>
                    <span className="truncate">{step.toolName}</span>
                  </div>
                );
              })}
              <div className="text-[10px] text-muted-foreground/50 pl-0.5">Mode: {mode}</div>
            </div>
          )}
        </div>

        {/* Ground Truth Section */}
        <div className="space-y-2">
          <div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
            <TargetIcon className="size-3" />
            Ground Truth
          </div>

          {gt ? (
            <div className="rounded-md border bg-[var(--accent-emerald)] border-[var(--dot-emerald)]/20 px-2.5 py-2 text-[11px] text-[var(--dot-emerald)]">
              <div className="font-medium capitalize">{gt.kind}</div>
              <div className="text-[var(--dot-emerald)]/70 mt-0.5">{gtSummary(gt)}</div>
            </div>
          ) : legacyGt ? (
            <div className="rounded-md border bg-muted/30 px-2.5 py-2 text-[11px] text-muted-foreground">
              <div className="font-medium">Text</div>
              <div className="truncate mt-0.5">{legacyGt}</div>
            </div>
          ) : (
            <div className="text-[10px] text-muted-foreground/50 pl-0.5">Not yet defined</div>
          )}
        </div>

        {/* Completeness Check */}
        {currentStep === "review" && (
          <div className="rounded-md border border-[var(--dot-emerald)]/30 bg-[var(--accent-emerald)] px-3 py-2.5 text-[11px]">
            <div className="flex items-center gap-1.5 text-[var(--dot-emerald)] font-medium">
              <CheckCircle2Icon className="size-3.5" />
              Ready to save
            </div>
            <div className="text-[var(--dot-emerald)]/60 mt-1">
              Trajectory and ground truth are set. Click "Save as Test Case" to persist.
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

// ============================================================================
// Message List
// ============================================================================

function TestCaseBuilderMessageList({ messages }: { messages: Message[] }) {
  return (
    <>
      {messages.map((msg) => (
        <div
          key={msg.id}
          className={msg.role === "user" ? "flex justify-end" : "flex justify-start"}
        >
          <div
            className={
              msg.role === "user"
                ? "max-w-[80%] rounded-2xl px-4 py-2 bg-primary text-primary-foreground"
                : "max-w-[80%] rounded-2xl px-4 py-2"
            }
          >
            <div className="space-y-2">
              {(msg.role === "user" ? msg.parts : groupParts(msg.parts)).map((part, i) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: parts lack unique IDs
                <DisplayPartDisplay key={i} part={part} isUserMessage={msg.role === "user"} />
              ))}
            </div>
          </div>
        </div>
      ))}
    </>
  );
}

// ============================================================================
// Save Bar
// ============================================================================

function SaveBar({
  sessionId,
  session,
}: {
  readonly sessionId: string;
  readonly session: TestCaseBuilderSession | null;
}) {
  const navigate = useNavigate();
  const { addTestCase } = useTestSuiteActions();
  const messages = useTestSuite(selectBuilderMessages);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const trajectory = session?.composedTrajectory ?? [];
  const gt = session?.structuredGroundTruth ?? null;
  const legacyGt = session?.groundTruth;

  const handleSave = useCallback(async () => {
    setSaving(true);
    setError(null);

    const firstUserMsg = messages.find((m) => m.role === "user");
    const userPrompt = firstUserMsg
      ? extractTextContent(firstUserMsg.parts) || undefined
      : undefined;

    const result = await saveFromBuilder(
      sessionId,
      "draft",
      userPrompt,
      gt ?? undefined,
      legacyGt ?? undefined
    );
    setSaving(false);

    if (isOk(result)) {
      addTestCase(result.value);
      navigate(`/tests/${result.value.id}`);
    } else {
      setError(result.error.message);
    }
  }, [sessionId, messages, addTestCase, navigate, gt, legacyGt]);

  return (
    <div className="border-t px-4 py-3 space-y-2">
      {/* Preview */}
      <div className="flex items-center gap-3 text-[10px] text-muted-foreground">
        <span className="flex items-center gap-1">
          <ClipboardListIcon className="size-3" />
          {trajectory.length} trajectory step{trajectory.length !== 1 ? "s" : ""}
        </span>
        <span className="text-muted-foreground/30">|</span>
        <span className="flex items-center gap-1">
          <TargetIcon className="size-3" />
          {gt ? gtSummary(gt) : legacyGt ? "Text GT" : "No ground truth"}
        </span>
      </div>
      {/* Save button */}
      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={handleSave}
          disabled={saving}
          className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90 transition-colors disabled:opacity-50 text-sm font-medium flex items-center gap-2 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
        >
          <SaveIcon className="size-4" />
          {saving ? "Saving..." : "Save as Test Case"}
        </button>
        {!gt && !legacyGt && (
          <span className="text-[10px] text-[var(--dot-amber)] leading-relaxed">
            Tip: ask the agent to set ground truth, e.g. &quot;Set expected actions: UPDATE for all
            matching entities&quot;
          </span>
        )}
        {error && <span className="text-sm text-destructive">{error}</span>}
      </div>
    </div>
  );
}

// ============================================================================
// Main Component
// ============================================================================

export function TestCaseBuilderChat({
  prefill,
  builderSessionId,
  workspaceId,
}: {
  readonly prefill?: string;
  readonly builderSessionId?: string;
  readonly workspaceId?: string;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const abortRef = useRef<(() => void) | null>(null);

  // Store selectors (state)
  const sessionId = useTestSuite(selectBuilderSessionId);
  const messages = useTestSuite(selectBuilderMessages);
  const streamingParts = useTestSuite(selectBuilderLiveParts);
  const isStreaming = useTestSuite(selectBuilderIsStreaming);
  const hasMessages = useTestSuite(selectBuilderHasMessages);
  const builderError = useTestSuite(selectBuilderError);
  const session = useTestSuite(selectBuilderSession);
  const availableTools = useTestSuite(selectAvailableTools);

  // Action bundles
  const { setBuilderSession } = useTestSuiteActions();
  const {
    startSession: startBuilderSession,
    adoptSession: adoptBuilderSession,
    sendMessage: addBuilderUserMessageRaw,
    startStream: startBuilderStreamingRaw,
    interpretEvent: handleBuilderStreamPartRaw,
    finishStream: completeBuilderStreamingRaw,
    abortStream: cancelBuilderStreamingRaw,
    setError: setBuilderErrorRaw,
    resetBuilder: resetBuilderRaw,
  } = useBuilderChatActions();

  // Session-scoped wrappers (bind sessionId for all actions)
  const addBuilderUserMessage = useCallback(
    (content: string) => {
      if (sessionId) {
        addBuilderUserMessageRaw(
          sessionId,
          content,
          MessageId(crypto.randomUUID()),
          new Date().toISOString()
        );
      }
    },
    [addBuilderUserMessageRaw, sessionId]
  );
  const startBuilderStreaming = useCallback(() => {
    if (sessionId) startBuilderStreamingRaw(sessionId, MessageId(crypto.randomUUID()));
  }, [startBuilderStreamingRaw, sessionId]);
  const handleBuilderStreamPart = useCallback(
    (part: import("~/lib/domain/stream-part").StreamPart) => {
      if (sessionId) handleBuilderStreamPartRaw(sessionId, part);
    },
    [handleBuilderStreamPartRaw, sessionId]
  );
  const completeBuilderStreaming = useCallback(() => {
    if (sessionId) completeBuilderStreamingRaw(sessionId, new Date().toISOString());
  }, [completeBuilderStreamingRaw, sessionId]);
  const cancelBuilderStreaming = useCallback(() => {
    if (sessionId) cancelBuilderStreamingRaw(sessionId, new Date().toISOString());
  }, [cancelBuilderStreamingRaw, sessionId]);
  const setBuilderError = useCallback(
    (msg: string | null) => {
      if (sessionId) setBuilderErrorRaw(sessionId, msg);
    },
    [setBuilderErrorRaw, sessionId]
  );
  const resetBuilder = useCallback(() => {
    if (sessionId) resetBuilderRaw(sessionId);
  }, [resetBuilderRaw, sessionId]);

  useEffect(() => {
    if (!builderSessionId) {
      return;
    }
    if (workspaceId) {
      useWorkspace.getState().setActiveWorkspace(workspaceId);
    }
    adoptBuilderSession(builderSessionId, workspaceId ?? "default");
  }, [adoptBuilderSession, builderSessionId, workspaceId]);

  // Auto-start session on mount (or restore focused session)
  useEffect(() => {
    if (!sessionId && !builderSessionId) {
      startBuilderSession();
    }
  }, [builderSessionId, sessionId, startBuilderSession]);

  // Fetch session state after streaming completes
  const prevStreamingRef = useRef(isStreaming);
  useEffect(() => {
    if (prevStreamingRef.current && !isStreaming && sessionId) {
      getTestCaseBuilderSession(sessionId).then((result) => {
        if (isOk(result)) {
          setBuilderSession(result.value);
        }
      });
    }
    prevStreamingRef.current = isStreaming;
  }, [isStreaming, sessionId, setBuilderSession]);

  // Auto-scroll on new content
  const messageCount = messages.length;
  const partCount = streamingParts.length;
  useEffect(() => {
    void messageCount;
    void partCount;
    scrollRef.current?.scrollTo({
      top: scrollRef.current.scrollHeight,
      behavior: "smooth",
    });
  }, [messageCount, partCount]);

  // Build chat history for the request
  const buildHistory = useCallback((): ChatRequestMessage[] => {
    const history: ChatRequestMessage[] = [];
    for (const msg of messages) {
      if (msg.role !== "user" && msg.role !== "assistant") continue;
      const text = extractTextContent(msg.parts);
      if (text) {
        history.push({ role: msg.role, content: text });
      }
    }
    return history;
  }, [messages]);

  // Send message
  const handleSend = useCallback(
    async (content: string) => {
      if (!sessionId) return;

      addBuilderUserMessage(content);
      startBuilderStreaming();

      const history = buildHistory();

      const result = await startChatStream(
        {
          threadId: `builder-${sessionId}`,
          messages: [...history, { role: "user", content }],
          role: "test_case_builder",
          sessionId,
        },
        {
          onPart: handleBuilderStreamPart,
          onComplete: completeBuilderStreaming,
          onError: (err) => {
            setBuilderError(err.message);
            completeBuilderStreaming();
          },
        }
      );

      if (isOk(result)) {
        abortRef.current = result.value.abort;
      } else {
        setBuilderError(result.error.message);
        completeBuilderStreaming();
      }
    },
    [
      sessionId,
      addBuilderUserMessage,
      startBuilderStreaming,
      buildHistory,
      handleBuilderStreamPart,
      completeBuilderStreaming,
      setBuilderError,
    ]
  );

  // Cancel streaming
  const handleCancel = useCallback(() => {
    abortRef.current?.();
    abortRef.current = null;
    cancelBuilderStreaming();
  }, [cancelBuilderStreaming]);

  // Reset builder
  const handleReset = useCallback(async () => {
    if (!window.confirm("Clear this conversation and start over?")) return;
    handleCancel();
    if (sessionId) {
      try {
        await clearTestCaseBuilderSession(sessionId);
      } catch {
        // Best-effort cleanup
      }
    }
    setBuilderSession(null);
    resetBuilder();
  }, [sessionId, handleCancel, resetBuilder, setBuilderSession]);

  // Derive workflow step
  const currentStep = deriveStep(hasMessages, session);

  // Chat context for MessagePartDisplay
  const chatContextValue = useMemo(() => ({ isStreaming }), [isStreaming]);

  return (
    <ChatContext.Provider value={chatContextValue}>
      <div className="flex-1 flex min-h-0">
        {/* Chat Column */}
        <div className="flex-1 flex flex-col min-h-0 min-w-0">
          {/* Message area */}
          <div ref={scrollRef} className="flex-1 overflow-y-auto scroll-container p-4 space-y-4">
            {!hasMessages && !isStreaming && (
              <div className="text-center mt-12 max-w-md mx-auto">
                <div className="inline-flex items-center justify-center w-12 h-12 rounded-full bg-primary/10 mb-4">
                  <ClipboardListIcon className="size-6 text-primary" />
                </div>
                <h2 className="text-sm font-semibold mb-1">Build a Test Case</h2>
                <p className="text-xs text-muted-foreground leading-relaxed">
                  Describe a scenario and the builder agent will compose the expected tool
                  trajectory and ground truth. It can also explore your data to find real entity IDs
                  and column values.
                </p>
                <div className="mt-4 flex flex-wrap justify-center gap-2">
                  {[
                    "Find all records matching status = active and update them",
                    "Search for entities in the US region and summarize results",
                    "Run a batch update on all items created this quarter",
                  ].map((example) => (
                    <button
                      key={example}
                      type="button"
                      onClick={() => handleSend(example)}
                      className="px-3 py-1.5 text-[11px] rounded-full border border-dashed border-muted-foreground/30 text-muted-foreground hover:border-primary/50 hover:text-primary transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                    >
                      {example}
                    </button>
                  ))}
                </div>
              </div>
            )}

            <TestCaseBuilderMessageList messages={messages} />

            {isStreaming && streamingParts.length > 0 && (
              <StreamingMessage parts={streamingParts} />
            )}

            {builderError && (
              <ErrorBanner message={builderError} onDismiss={() => setBuilderError(null)} />
            )}
          </div>

          {/* Save bar (after streaming completes and there are messages) */}
          {!isStreaming && hasMessages && sessionId && (
            <SaveBar sessionId={sessionId} session={session} />
          )}

          {/* Input + Reset */}
          <div className="border-t p-4">
            <div className="flex items-end gap-2">
              <div className="flex-1">
                <MessageInput
                  onSend={handleSend}
                  onCancel={handleCancel}
                  isStreaming={isStreaming}
                  placeholder="Describe the scenario for your test case..."
                  pendingInput={prefill}
                />
              </div>
              {hasMessages && !isStreaming && (
                <button
                  type="button"
                  onClick={handleReset}
                  className="p-2 rounded-md border hover:bg-muted transition-colors text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                  title="Reset session"
                >
                  <RotateCcwIcon className="size-4" />
                </button>
              )}
            </div>
          </div>
        </div>

        {/* Session State Panel (right side) */}
        <SessionStatePanel
          session={session}
          currentStep={currentStep}
          availableTools={availableTools}
        />
      </div>
    </ChatContext.Provider>
  );
}
