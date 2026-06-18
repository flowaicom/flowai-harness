/**
 * Streaming message preview component.
 *
 * Shows the message being streamed with:
 * - Live text updates
 * - Tool invocation indicators
 * - Typing cursor animation
 *
 * Performance optimizations:
 * - Memoized component with custom comparator
 * - Optimized for frequent re-renders during streaming
 *
 * @module components/chat/streaming-message
 */

import { Loader2Icon } from "lucide-react";
import { memo, useMemo } from "react";
import type { MessagePart } from "~/lib/domain/message";
import { groupParts } from "~/lib/domain/message";
import { DisplayPartDisplay } from "./message-part";

// ============================================================================
// Props
// ============================================================================

interface StreamingMessageProps {
  parts: MessagePart[];
}

// ============================================================================
// Main Component (Memoized)
// ============================================================================

/**
 * Streaming message comparator.
 *
 * For streaming, we re-render when:
 * - Part count changes
 * - Last part type or content changes (most common case)
 *
 * This avoids deep comparison of entire parts array.
 */
const areStreamingPartsEqual = (
  prev: StreamingMessageProps,
  next: StreamingMessageProps
): boolean => {
  // Different lengths always differ
  if (prev.parts.length !== next.parts.length) return false;

  // Empty arrays are equal
  if (prev.parts.length === 0) return true;

  // Compare last part (most likely to change during streaming)
  const prevLast = prev.parts[prev.parts.length - 1];
  const nextLast = next.parts[next.parts.length - 1];

  if (prevLast.type !== nextLast.type) return false;

  // For text/reasoning, compare content length (most common streaming case)
  if (prevLast.type === "text" && nextLast.type === "text") {
    return prevLast.text === nextLast.text;
  }
  if (prevLast.type === "reasoning" && nextLast.type === "reasoning") {
    return prevLast.text.length === nextLast.text.length;
  }

  // For tool invocations, compare state and progress
  if (prevLast.type === "tool-invocation" && nextLast.type === "tool-invocation") {
    return (
      prevLast.toolCallId === nextLast.toolCallId &&
      prevLast.state === nextLast.state &&
      (prevLast.progress?.phases.length ?? 0) === (nextLast.progress?.phases.length ?? 0)
    );
  }

  // Default: reference equality for other types
  return prevLast === nextLast;
};

export const StreamingMessage = memo(function StreamingMessage({ parts }: StreamingMessageProps) {
  const groupedParts = useMemo(() => groupParts(parts), [parts]);

  return (
    <div className="max-w-3xl mx-auto mt-4">
      {groupedParts.length > 0 ? (
        <div className="space-y-2">
          {groupedParts.map((part, index) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: parts lack unique IDs
            <DisplayPartDisplay key={index} part={part} isUserMessage={false} />
          ))}
        </div>
      ) : null}
      <div className="flex items-center gap-2 text-muted-foreground mt-2">
        <Loader2Icon className="size-4 animate-spin" />
        <span className="text-sm">Thinking...</span>
      </div>
    </div>
  );
}, areStreamingPartsEqual);
