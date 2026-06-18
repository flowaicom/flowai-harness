import { useVirtualizer } from "@tanstack/react-virtual";
import { type ReactNode, type RefObject, useMemo } from "react";

const VIRTUALIZATION_THRESHOLD = 15;
const ESTIMATED_MESSAGE_HEIGHT = 120;
const OVERSCAN = 5;

interface SharedVirtualizedMessageRenderContext {
  readonly isStreaming: boolean;
  readonly disableActions: boolean;
}

interface SharedVirtualizedMessageListProps<TMessage extends { readonly id: unknown }> {
  readonly messages: readonly TMessage[];
  readonly streamingMessageId?: unknown;
  readonly scrollContainerRef: RefObject<HTMLDivElement | null>;
  readonly emptyState?: ReactNode;
  readonly disableStableMessageActionsWhileStreaming?: boolean;
  readonly renderMessage: (
    message: TMessage,
    context: SharedVirtualizedMessageRenderContext
  ) => ReactNode;
}

export function SharedVirtualizedMessageList<TMessage extends { readonly id: unknown }>({
  messages,
  streamingMessageId,
  scrollContainerRef,
  emptyState = null,
  disableStableMessageActionsWhileStreaming = false,
  renderMessage,
}: SharedVirtualizedMessageListProps<TMessage>) {
  const { stableMessages, streamingMessage } = useMemo(() => {
    if (streamingMessageId && messages.length > 0) {
      const streaming = messages.find((message) => message.id === streamingMessageId);
      const stable = messages.filter((message) => message.id !== streamingMessageId);
      return { stableMessages: stable, streamingMessage: streaming };
    }
    return { stableMessages: messages, streamingMessage: undefined };
  }, [messages, streamingMessageId]);

  const useVirtualization = stableMessages.length > VIRTUALIZATION_THRESHOLD;
  const disableActions = disableStableMessageActionsWhileStreaming && streamingMessageId != null;

  const virtualizer = useVirtualizer({
    count: useVirtualization ? stableMessages.length : 0,
    getScrollElement: () => scrollContainerRef.current,
    estimateSize: () => ESTIMATED_MESSAGE_HEIGHT,
    overscan: OVERSCAN,
  });

  const virtualItems = useVirtualization ? virtualizer.getVirtualItems() : [];

  if (messages.length === 0) {
    return <>{emptyState}</>;
  }

  return (
    <>
      {useVirtualization ? (
        <div
          style={{
            height: `${virtualizer.getTotalSize()}px`,
            width: "100%",
            position: "relative",
          }}
        >
          {virtualItems.map((virtualRow) => {
            const message = stableMessages[virtualRow.index];
            return (
              <div
                key={virtualRow.key}
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  transform: `translateY(${virtualRow.start}px)`,
                }}
              >
                {renderMessage(message, { isStreaming: false, disableActions })}
              </div>
            );
          })}
        </div>
      ) : (
        <div className="space-y-4">
          {stableMessages.map((message) => (
            <div key={String(message.id)}>
              {renderMessage(message, { isStreaming: false, disableActions })}
            </div>
          ))}
        </div>
      )}

      {streamingMessage ? (
        <div>{renderMessage(streamingMessage, { isStreaming: true, disableActions: true })}</div>
      ) : null}
    </>
  );
}

export type { SharedVirtualizedMessageListProps, SharedVirtualizedMessageRenderContext };
