import {
  buildDisplayPartKeys,
  type DisplayPart,
  groupParts,
  type Message,
  type MessagePart,
  type SubAgentInvocationDisplayPart,
  type ToolGroupDisplayPart,
} from "@studio/core";
import { memo, type ReactNode, useMemo } from "react";
import { SharedSubAgentInvocationPart } from "./message-part-primitives";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

interface TextPartProps {
  readonly text: string;
  readonly isUser: boolean;
  readonly isStreaming: boolean;
}

const MemoizedTextPart = memo(
  function MemoizedTextPart({ text, isUser, isStreaming }: TextPartProps) {
    if (isStreaming) {
      return (
        <div
          className={cx(
            "streaming-text whitespace-pre-wrap font-mono",
            isUser ? "text-primary-foreground" : "text-foreground"
          )}
        >
          {text}
        </div>
      );
    }

    return (
      <div
        className={cx(
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

interface SharedMessageItemRenderContext {
  readonly isUserMessage: boolean;
  readonly isStreaming: boolean;
  readonly disableActions: boolean;
}

interface SharedMessageItemProps {
  readonly message: Message;
  readonly isStreaming?: boolean;
  readonly disableActions?: boolean;
  readonly renderMessagePart: (
    part: Exclude<MessagePart, { type: "text" }>,
    key: string,
    context: SharedMessageItemRenderContext
  ) => ReactNode;
  readonly renderToolGroup: (
    part: ToolGroupDisplayPart,
    key: string,
    context: SharedMessageItemRenderContext
  ) => ReactNode;
  readonly renderSubAgentInvocation?: (
    part: SubAgentInvocationDisplayPart,
    key: string,
    context: SharedMessageItemRenderContext
  ) => ReactNode;
}

export const SharedMessageItem = memo(
  function SharedMessageItem({
    message,
    isStreaming = false,
    disableActions = false,
    renderMessagePart,
    renderToolGroup,
    renderSubAgentInvocation,
  }: SharedMessageItemProps) {
    const isUser = message.role === "user";
    const groupedParts: DisplayPart[] = useMemo(
      () => (isUser ? message.parts : groupParts(message.parts)),
      [message.parts, isUser]
    );
    const partKeys = useMemo(() => buildDisplayPartKeys(groupedParts), [groupedParts]);
    const keyedParts = useMemo(
      () => groupedParts.map((part, index) => ({ part, key: partKeys[index] })),
      [groupedParts, partKeys]
    );

    const renderContext: SharedMessageItemRenderContext = {
      isUserMessage: isUser,
      isStreaming,
      disableActions,
    };

    return (
      <div
        className={cx(
          "mx-auto max-w-3xl pb-4",
          isStreaming ? "message-item-streaming" : "message-item-contained"
        )}
      >
        {isUser ? (
          <div className="flex justify-end">
            <div className="max-w-[80%] rounded-2xl bg-primary px-4 py-2 text-primary-foreground">
              <div className="space-y-2">
                {keyedParts.map(({ part, key }) =>
                  part.type === "text" ? (
                    <MemoizedTextPart
                      key={`${message.id}-${key}`}
                      text={part.text}
                      isUser={true}
                      isStreaming={isStreaming}
                    />
                  ) : part.type === "tool-group" ? (
                    renderToolGroup(part, `${message.id}-${key}`, renderContext)
                  ) : part.type === "sub-agent-invocation" ? (
                    renderSubAgentInvocation ? (
                      renderSubAgentInvocation(part, `${message.id}-${key}`, renderContext)
                    ) : (
                      <SharedSubAgentInvocationPart
                        key={`${message.id}-${key}`}
                        agentName={part.agentName}
                        state={part.state}
                        sourceParts={part.parts}
                      />
                    )
                  ) : (
                    renderMessagePart(part, `${message.id}-${key}`, renderContext)
                  )
                )}
              </div>
            </div>
          </div>
        ) : (
          <div className="space-y-2">
            {keyedParts.map(({ part, key }) => {
              const itemKey = `${message.id}-${key}`;
              if (part.type === "tool-group") {
                return renderToolGroup(part, itemKey, renderContext);
              }
              if (part.type === "sub-agent-invocation") {
                return renderSubAgentInvocation ? (
                  renderSubAgentInvocation(part, itemKey, renderContext)
                ) : (
                  <SharedSubAgentInvocationPart
                    key={itemKey}
                    agentName={part.agentName}
                    state={part.state}
                    sourceParts={part.parts}
                  />
                );
              }
              if (part.type === "text") {
                return (
                  <MemoizedTextPart
                    key={itemKey}
                    text={part.text}
                    isUser={false}
                    isStreaming={isStreaming}
                  />
                );
              }
              return renderMessagePart(part, itemKey, renderContext);
            })}
          </div>
        )}
      </div>
    );
  },
  (prev, next) => {
    if (prev.message.id !== next.message.id) return false;
    if (prev.isStreaming !== next.isStreaming) return false;
    if (prev.disableActions !== next.disableActions) return false;
    if (prev.renderMessagePart !== next.renderMessagePart) return false;
    if (prev.renderToolGroup !== next.renderToolGroup) return false;
    if (prev.renderSubAgentInvocation !== next.renderSubAgentInvocation) return false;
    if (prev.message.parts.length !== next.message.parts.length) return false;
    if (next.isStreaming) return false;
    return prev.message.parts === next.message.parts;
  }
);

export type { SharedMessageItemProps, SharedMessageItemRenderContext };
