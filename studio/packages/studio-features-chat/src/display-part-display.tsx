import type {
  CommandCardPart,
  DisplayPart,
  MessagePart,
  ToolInvocationMessagePart,
} from "@studio/core";
import { memo, type ReactNode } from "react";
import {
  type MarkdownRenderer,
  SharedFilePart,
  SharedReasoningPart,
  SharedSubAgentInvocationPart,
  SharedTextPart,
  SharedToolAgentPart,
  SharedToolGroupDisplay,
  SharedToolProgressDisplay,
} from "./message-part-primitives";

interface SharedDisplayRenderContext {
  readonly isStreaming?: boolean;
}

export interface SharedDisplayPartDisplayProps {
  readonly part: DisplayPart;
  readonly isUserMessage: boolean;
  readonly isStreaming?: boolean;
  readonly renderMarkdown: MarkdownRenderer;
  readonly renderToolInvocation: (
    part: ToolInvocationMessagePart,
    key: string,
    context: SharedDisplayRenderContext
  ) => ReactNode;
  readonly renderCommandCard: (
    part: CommandCardPart,
    key: string,
    context: SharedDisplayRenderContext
  ) => ReactNode;
}

const areDisplayPartsEqual = (
  prev: SharedDisplayPartDisplayProps,
  next: SharedDisplayPartDisplayProps
): boolean => {
  if (prev.part.type !== next.part.type) return false;
  if (prev.isUserMessage !== next.isUserMessage) return false;
  if (prev.isStreaming !== next.isStreaming) return false;
  if (prev.renderMarkdown !== next.renderMarkdown) return false;
  if (prev.renderToolInvocation !== next.renderToolInvocation) return false;
  if (prev.renderCommandCard !== next.renderCommandCard) return false;

  switch (prev.part.type) {
    case "text":
      return prev.part.text === (next.part as typeof prev.part).text;
    case "reasoning":
      return prev.part.text === (next.part as typeof prev.part).text;
    case "tool-invocation": {
      const nextPart = next.part as typeof prev.part;
      return (
        prev.part.toolCallId === nextPart.toolCallId &&
        prev.part.state === nextPart.state &&
        prev.part.args === nextPart.args &&
        prev.part.result === nextPart.result &&
        prev.part.progress?.phases.length === nextPart.progress?.phases.length
      );
    }
    case "tool-agent": {
      const nextPart = next.part as typeof prev.part;
      return prev.part.toolCallId === nextPart.toolCallId && prev.part.state === nextPart.state;
    }
    case "sub-agent-invocation": {
      const nextPart = next.part as typeof prev.part;
      return (
        prev.part.toolCallId === nextPart.toolCallId &&
        prev.part.agentName === nextPart.agentName &&
        prev.part.state === nextPart.state &&
        prev.part.parts.length === nextPart.parts.length
      );
    }
    case "file":
      return prev.part.fileId === (next.part as typeof prev.part).fileId;
    case "flow-ui":
      return prev.part.dsl === (next.part as typeof prev.part).dsl;
    case "tool-progress": {
      const nextPart = next.part as typeof prev.part;
      return (
        prev.part.toolName === nextPart.toolName &&
        prev.part.currentPhaseIndex === nextPart.currentPhaseIndex &&
        prev.part.phases.length === nextPart.phases.length
      );
    }
    case "tool-group": {
      const nextPart = next.part as typeof prev.part;
      if (prev.part.parts.length !== nextPart.parts.length) return false;
      return prev.part.parts.every(
        (part, index) =>
          part.toolCallId === nextPart.parts[index].toolCallId &&
          part.state === nextPart.parts[index].state &&
          part.args === nextPart.parts[index].args &&
          part.result === nextPart.parts[index].result
      );
    }
    default:
      return prev.part === next.part;
  }
};

export const SharedDisplayPartDisplay = memo(function SharedDisplayPartDisplay({
  part,
  isUserMessage,
  isStreaming,
  renderMarkdown,
  renderToolInvocation,
  renderCommandCard,
}: SharedDisplayPartDisplayProps) {
  switch (part.type) {
    case "text":
      return (
        <SharedTextPart text={part.text} isUser={isUserMessage} renderMarkdown={renderMarkdown} />
      );
    case "reasoning":
      return <SharedReasoningPart text={part.text} renderMarkdown={renderMarkdown} />;
    case "tool-invocation":
      return renderToolInvocation(part, part.toolCallId, { isStreaming });
    case "tool-agent":
      return <SharedToolAgentPart agentName={part.agentName} state={part.state} />;
    case "sub-agent-invocation":
      return (
        <SharedSubAgentInvocationPart
          agentName={part.agentName}
          state={part.state}
          sourceParts={part.parts}
        />
      );
    case "file":
      return <SharedFilePart fileId={part.fileId} filename={part.filename} />;
    case "flow-ui":
      return renderCommandCard(part, `flow-ui:${part.dsl.length}`, { isStreaming });
    case "tool-progress":
      return (
        <SharedToolProgressDisplay
          toolName={part.toolName}
          agentName={part.agentName}
          phases={part.phases}
          currentPhaseIndex={part.currentPhaseIndex}
          totalPhases={part.totalPhases}
        />
      );
    case "tool-group":
      return (
        <SharedToolGroupDisplay
          group={part}
          isStreaming={isStreaming}
          renderToolInvocation={renderToolInvocation}
        />
      );
    default:
      return null;
  }
}, areDisplayPartsEqual);

export interface SharedMessagePartDisplayProps extends Omit<SharedDisplayPartDisplayProps, "part"> {
  readonly part: MessagePart;
}

export function SharedMessagePartDisplay({ part, ...props }: SharedMessagePartDisplayProps) {
  return <SharedDisplayPartDisplay part={part} {...props} />;
}
