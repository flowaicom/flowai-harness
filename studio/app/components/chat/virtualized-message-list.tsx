/**
 * Virtualized message list component.
 *
 * Performance optimizations:
 * - Virtualization for large message lists (>15 messages)
 * - React.memo with custom comparators
 * - CSS containment for off-screen content
 * - Lazy markdown parsing during streaming
 *
 * @module components/chat/virtualized-message-list
 */

import { SharedVirtualizedMessageList } from "@studio/features-chat";
import { ExternalLinkIcon } from "lucide-react";
import { memo, useMemo } from "react";
import { Link } from "react-router";
import type { DisplayPart, Message, MessagePart, ResponseValidation } from "~/lib/domain/message";
import { groupParts } from "~/lib/domain/message";
import { cn } from "~/lib/utils";
import { DisplayPartDisplay, MessagePartDisplay } from "./message-part";

// ============================================================================
// Memoized Text Part (lazy markdown)
// ============================================================================

interface TextPartProps {
  text: string;
  isUser: boolean;
  isStreaming: boolean;
}

/**
 * During streaming, show plain text (no parsing overhead).
 * For completed messages, render normally.
 */
const MemoizedTextPart = memo(
  function MemoizedTextPart({ text, isUser, isStreaming }: TextPartProps) {
    if (isStreaming) {
      // Plain text during streaming - faster rendering
      return (
        <div
          className={cn(
            "streaming-text whitespace-pre-wrap font-mono",
            isUser ? "text-primary-foreground" : "text-foreground"
          )}
        >
          {text}
        </div>
      );
    }

    // Normal text for completed messages
    return (
      <div
        className={cn(
          "whitespace-pre-wrap",
          isUser ? "text-primary-foreground" : "text-foreground"
        )}
      >
        {text}
      </div>
    );
  },
  (prev, next) =>
    prev.text === next.text && prev.isUser === next.isUser && prev.isStreaming === next.isStreaming
);

// ============================================================================
// Message Part Renderer
// ============================================================================

interface PartRendererProps {
  part: MessagePart;
  isUserMessage: boolean;
  isStreaming: boolean;
}

const PartRenderer = memo(
  function PartRenderer({ part, isUserMessage, isStreaming }: PartRendererProps) {
    // During streaming, show plain text for speed (no markdown parsing).
    if (part.type === "text" && isStreaming) {
      return <MemoizedTextPart text={part.text} isUser={isUserMessage} isStreaming />;
    }

    // Completed text (and all other parts) render via the canonical path,
    // which renders text as markdown.
    return <MessagePartDisplay part={part} isUserMessage={isUserMessage} />;
  },
  (prev, next) => {
    // Custom comparator for performance
    if (prev.part.type !== next.part.type) return false;
    if (prev.isUserMessage !== next.isUserMessage) return false;
    if (prev.isStreaming !== next.isStreaming) return false;

    // Type-specific comparisons
    if (prev.part.type === "text" && next.part.type === "text") {
      return prev.part.text === next.part.text;
    }
    if (prev.part.type === "tool-invocation" && next.part.type === "tool-invocation") {
      return prev.part.state === next.part.state && prev.part.toolName === next.part.toolName;
    }

    return prev.part === next.part;
  }
);

// ============================================================================
// Message Item (memoized)
// ============================================================================

interface MessageItemProps {
  message: Message;
  isStreaming?: boolean;
}

function ResponseValidationBadge({ validation }: { validation: ResponseValidation }) {
  const summary = validation.ok
    ? `Validated against ${validation.contract.modelName}`
    : `Invalid ${validation.contract.modelName} output`;
  const detail = validation.ok ? validation.contract.modelRef : validation.errors?.[0];

  return (
    <div
      className={cn(
        "mt-2 rounded-md border px-3 py-2 text-xs",
        validation.ok
          ? "border-[var(--dot-emerald)]/30 bg-[var(--accent-emerald)] text-[var(--dot-emerald)]"
          : "border-destructive/30 bg-[var(--accent-red)] text-[var(--dot-red)]"
      )}
    >
      <div className="font-medium">{summary}</div>
      {detail && <div className="mt-1 font-mono break-words opacity-80">{detail}</div>}
    </div>
  );
}

/**
 * Memoized message item prevents re-render when message unchanged.
 * CSS containment for off-screen optimization.
 */
const MessageItem = memo(
  function MessageItem({ message, isStreaming = false }: MessageItemProps) {
    const isUser = message.role === "user";

    // Group consecutive tool calls for assistant messages (pure, memoized)
    const groupedParts: DisplayPart[] = useMemo(
      () => (isUser ? message.parts : groupParts(message.parts)),
      [message.parts, isUser]
    );

    return (
      <div
        className={cn(
          "max-w-3xl mx-auto pb-4",
          isStreaming ? "message-item-streaming" : "message-item-contained"
        )}
      >
        {isUser ? (
          <div className="flex justify-end">
            <div className="bg-primary text-primary-foreground rounded-2xl px-4 py-2 max-w-[80%]">
              <div className="space-y-2">
                {groupedParts.map((part, index) => (
                  <PartRenderer
                    key={`${message.id}-${index}-${part.type}`}
                    part={part as MessagePart}
                    isUserMessage={true}
                    isStreaming={isStreaming}
                  />
                ))}
              </div>
            </div>
          </div>
        ) : (
          <div className="space-y-2">
            {groupedParts.map((part, index) =>
              part.type === "tool-group" || part.type === "sub-agent-invocation" ? (
                <DisplayPartDisplay
                  key={`${message.id}-${index}-${part.type}`}
                  part={part}
                  isUserMessage={false}
                />
              ) : (
                <PartRenderer
                  key={`${message.id}-${index}-${part.type}`}
                  part={part as MessagePart}
                  isUserMessage={false}
                  isStreaming={isStreaming}
                />
              )
            )}
            {message.responseValidation && (
              <ResponseValidationBadge validation={message.responseValidation} />
            )}
            {message.runId && !isStreaming && (
              <div className="flex justify-end">
                <Link
                  to={`/runs/${message.runId}`}
                  className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-xs font-medium text-muted-foreground/55 transition-colors hover:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  aria-label={`Go to run ${message.runId}`}
                >
                  Go to run
                  <ExternalLinkIcon className="size-3" />
                </Link>
              </div>
            )}
          </div>
        )}
      </div>
    );
  },
  (prev, next) => {
    // Custom comparator
    if (prev.message.id !== next.message.id) return false;
    if (prev.isStreaming !== next.isStreaming) return false;
    if (prev.message.parts.length !== next.message.parts.length) return false;
    if (prev.message.responseValidation !== next.message.responseValidation) return false;
    if (prev.message.runId !== next.message.runId) return false;

    // For streaming messages, always re-render
    if (next.isStreaming) return false;

    // For stable messages, compare parts
    return prev.message.parts === next.message.parts;
  }
);

// ============================================================================
// Props
// ============================================================================

export interface ChatPreset {
  readonly label: string;
  readonly prompt: string;
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

interface VirtualizedMessageListProps {
  messages: Message[];
  streamingMessageId?: string;
  scrollContainerRef: React.RefObject<HTMLDivElement | null>;
  onPresetClick?: (text: string) => void;
  presets?: ChatPreset[];
  projectName?: string;
  responseContracts?: readonly ResponseContractInfo[];
  roleTopology?: readonly RoleTopologyInfo[];
}

// ============================================================================
// Main Component
// ============================================================================

export function VirtualizedMessageList({
  messages,
  streamingMessageId,
  scrollContainerRef,
  onPresetClick,
  presets,
  projectName,
  responseContracts = [],
  roleTopology = [],
}: VirtualizedMessageListProps) {
  const emptyState = useMemo(() => {
    const title = projectName || "Agent Studio";
    const hasPresets = presets && presets.length > 0;

    return (
      <div className="flex flex-col items-center justify-center h-full text-center p-8">
        <h1 className="text-xl font-semibold mb-2">{title}</h1>
        <p className="text-muted-foreground text-sm mb-6">
          {hasPresets ? "Try one of these prompts to get started" : "Send a message to get started"}
        </p>
        {hasPresets && (
          <div className="flex flex-wrap gap-3 justify-center">
            {presets.map((preset) => (
              <button
                key={preset.label}
                type="button"
                onClick={() => onPresetClick?.(preset.prompt)}
                className="px-4 py-2 border rounded-lg text-sm hover:bg-muted transition-colors"
              >
                {preset.label}
              </button>
            ))}
          </div>
        )}
        {responseContracts.length > 0 && (
          <div className="mt-8 w-full max-w-2xl">
            <div className="text-[11px] font-medium uppercase tracking-[0.12em] text-muted-foreground">
              Response Contracts
            </div>
            <div className="mt-3 grid gap-2 sm:grid-cols-2">
              {responseContracts.map((contract) => (
                <div
                  key={`${contract.role}-${contract.modelRef}`}
                  className="rounded-lg border bg-card px-3 py-2 text-left"
                >
                  <div className="text-xs font-medium">{contract.role}</div>
                  <div className="mt-1 text-sm">{contract.modelName}</div>
                  <div className="mt-1 font-mono text-[11px] text-muted-foreground break-words">
                    {contract.modelRef}
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}
        {roleTopology.length > 0 && (
          <div className="mt-8 w-full max-w-2xl">
            <div className="text-[11px] font-medium uppercase tracking-[0.12em] text-muted-foreground">
              Role Topology
            </div>
            <div className="mt-3 grid gap-2 sm:grid-cols-2">
              {roleTopology.map((role) => (
                <div
                  key={`${role.role}-${role.delegatesTo.join(",")}`}
                  className="rounded-lg border bg-card px-3 py-2 text-left"
                >
                  <div className="text-xs font-medium">{role.role}</div>
                  <div className="mt-1 text-sm text-muted-foreground">
                    {role.delegatesTo.length > 0
                      ? `Delegates to ${role.delegatesTo.join(", ")}`
                      : "No explicit delegation edges"}
                  </div>
                  <div className="mt-2 text-[11px] text-muted-foreground">
                    Tools: {role.tools.length > 0 ? role.tools.join(", ") : "none"}
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>
    );
  }, [onPresetClick, presets, projectName, responseContracts, roleTopology]);

  return (
    <SharedVirtualizedMessageList
      messages={messages}
      streamingMessageId={streamingMessageId}
      scrollContainerRef={scrollContainerRef}
      emptyState={emptyState}
      renderMessage={(message, context) => (
        <MessageItem message={message} isStreaming={context.isStreaming} />
      )}
    />
  );
}

// ============================================================================
// Exports
// ============================================================================

export { MessageItem };
