/**
 * Chat replay view — renders a persisted conversation from a thread.
 *
 * @module components/eval/chat-replay-view
 */

import { MessageSquareIcon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { DisplayPartDisplay } from "~/components/chat/message-part";
import { EmptyState } from "~/components/shared/empty-state";
import { ErrorBanner } from "~/components/shared/error-banner";
import { Markdown } from "~/components/shared/markdown";
import { fetchMessagesWithFormat } from "~/lib/api";
import type { Message, MessagePart } from "~/lib/domain/message";
import { groupParts, parsePersistedMessages } from "~/lib/domain/message";
import { isOk } from "~/lib/domain/result";
import { cn } from "~/lib/utils";

/** Total function: extract plain text content from a parsed message. */
function extractContent(message: Message): string {
  return message.parts
    .filter((part): part is Extract<MessagePart, { type: "text" }> => part.type === "text")
    .map((part) => part.text)
    .join("");
}

/** Check if a message has structured parts that should be rendered richly. */
function hasStructuredParts(message: Message): boolean {
  return message.parts.some((part) => part.type !== "text");
}

/** Renders message parts with tool-call grouping for assistant messages. */
function GroupedParts({
  msgId,
  parts,
  isUser,
}: {
  msgId: string;
  parts: readonly MessagePart[];
  isUser: boolean;
}) {
  const grouped = useMemo(() => (isUser ? parts : groupParts(parts)), [parts, isUser]);
  return (
    <div className="space-y-2">
      {grouped.map((part, idx) => (
        <DisplayPartDisplay key={`${msgId}-part-${idx}`} part={part} isUserMessage={isUser} />
      ))}
    </div>
  );
}

interface ChatReplayViewProps {
  readonly threadId: string;
}

export function ChatReplayView({ threadId }: ChatReplayViewProps) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    fetchMessagesWithFormat(threadId).then((result) => {
      if (cancelled) return;
      setLoading(false);
      if (isOk(result)) {
        setMessages(parsePersistedMessages(result.value.messages, result.value.format));
      } else {
        setError(result.error.message);
      }
    });

    return () => {
      cancelled = true;
    };
  }, [threadId]);

  if (loading) {
    return (
      <div className="space-y-3 animate-in fade-in duration-200">
        {[1, 0.7, 0.85].map((width, i) => (
          <div
            key={`skel-${i}`}
            className={i % 2 === 0 ? "flex justify-end" : "flex justify-start"}
          >
            <div
              className="h-10 bg-muted rounded-2xl animate-shimmer"
              style={{ width: `${width * 16}rem` }}
            />
          </div>
        ))}
      </div>
    );
  }

  if (error) {
    return (
      <ErrorBanner
        message={`Failed to load conversation: ${error}`}
        onDismiss={() => setError(null)}
      />
    );
  }

  if (messages.length === 0) {
    return (
      <EmptyState
        icon={MessageSquareIcon}
        title="No messages"
        description="This thread has no messages yet."
      />
    );
  }

  return (
    <div className="space-y-3">
      {messages.map((msg, idx) => {
        const isUser = msg.role === "user";
        const content = extractContent(msg);

        return (
          <div key={msg.id}>
            <div
              className={cn(
                "group relative",
                isUser ? "flex justify-end" : "flex w-full justify-start"
              )}
            >
              <div
                className={cn(
                  "text-sm",
                  isUser
                    ? "max-w-[80%] rounded-2xl bg-primary px-4 py-2 text-primary-foreground"
                    : "w-full max-w-none text-foreground"
                )}
              >
                {hasStructuredParts(msg) ? (
                  <GroupedParts msgId={msg.id} parts={msg.parts} isUser={isUser} />
                ) : (
                  <Markdown
                    text={content}
                    className={
                      isUser ? "text-primary-foreground markdown-content-user" : "text-foreground"
                    }
                  />
                )}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}
