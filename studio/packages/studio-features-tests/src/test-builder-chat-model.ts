import type { Message } from "@studio/core/domain/message";
import { extractTextContent } from "@studio/core/domain/message";

export interface BuilderChatHistoryMessage {
  readonly role: "user" | "assistant";
  readonly content: string;
}

export interface BuilderStreamRequest {
  readonly threadId: string;
  readonly messages: BuilderChatHistoryMessage[];
  readonly role: "test_case_builder";
  readonly sessionId: string;
}

export function buildBuilderChatHistory(messages: readonly Message[]): BuilderChatHistoryMessage[] {
  const history: BuilderChatHistoryMessage[] = [];

  for (const message of messages) {
    if (message.role !== "user" && message.role !== "assistant") continue;
    const content = extractTextContent(message.parts);
    if (!content) continue;
    history.push({ role: message.role, content });
  }

  return history;
}

export function createBuilderStreamRequest(
  sessionId: string,
  history: readonly BuilderChatHistoryMessage[],
  content: string
): BuilderStreamRequest {
  return {
    threadId: `builder-${sessionId}`,
    messages: [...history, { role: "user", content }],
    role: "test_case_builder",
    sessionId,
  };
}

export function getBuilderSessionIdToRefresh(
  previousIsStreaming: boolean,
  isStreaming: boolean,
  sessionId: string | null | undefined
): string | null {
  return previousIsStreaming && !isStreaming && sessionId ? sessionId : null;
}
