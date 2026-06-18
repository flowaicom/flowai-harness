import type { StreamPart } from "@studio/core/domain/stream-part";

export interface BuilderSessionActionAdapter {
  addUserMessage(sessionId: string, content: string): void;
  startStreaming(sessionId: string): void;
  handleStreamPart(sessionId: string, part: StreamPart): void;
  completeStreaming(sessionId: string): void;
  cancelStreaming(sessionId: string): void;
  setError(sessionId: string, message: string | null): void;
  resetSession(sessionId: string): void;
}

export interface BoundBuilderSessionActions {
  readonly hasSession: boolean;
  addUserMessage(content: string): void;
  startStreaming(): void;
  handleStreamPart(part: StreamPart): void;
  completeStreaming(): void;
  cancelStreaming(): void;
  setError(message: string | null): void;
  reset(): void;
}

const NOOP_BOUND_BUILDER_SESSION_ACTIONS: BoundBuilderSessionActions = {
  hasSession: false,
  addUserMessage: () => {},
  startStreaming: () => {},
  handleStreamPart: () => {},
  completeStreaming: () => {},
  cancelStreaming: () => {},
  setError: () => {},
  reset: () => {},
};

export function bindBuilderSessionActions(
  sessionId: string | null | undefined,
  adapter: BuilderSessionActionAdapter
): BoundBuilderSessionActions {
  if (!sessionId) {
    return NOOP_BOUND_BUILDER_SESSION_ACTIONS;
  }

  return {
    hasSession: true,
    addUserMessage: (content) => adapter.addUserMessage(sessionId, content),
    startStreaming: () => adapter.startStreaming(sessionId),
    handleStreamPart: (part) => adapter.handleStreamPart(sessionId, part),
    completeStreaming: () => adapter.completeStreaming(sessionId),
    cancelStreaming: () => adapter.cancelStreaming(sessionId),
    setError: (message) => adapter.setError(sessionId, message),
    reset: () => adapter.resetSession(sessionId),
  };
}
