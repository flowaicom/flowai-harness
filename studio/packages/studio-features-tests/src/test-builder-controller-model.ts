import type { Result } from "@studio/core/domain/result";
import {
  type BuilderChatHistoryMessage,
  type BuilderStreamRequest,
  createBuilderStreamRequest,
} from "./test-builder-chat-model";

export interface BuilderStreamAbortHandle {
  readonly abort: () => void;
}

export interface BuilderStreamErrorLike {
  readonly message: string;
}

export interface SendBuilderMessageDeps<TError extends BuilderStreamErrorLike> {
  readonly sessionId: string | null | undefined;
  readonly content: string;
  readonly history: readonly BuilderChatHistoryMessage[];
  readonly addUserMessage: (content: string) => void;
  readonly startStreaming: () => void;
  readonly startStream: (input: {
    readonly request: BuilderStreamRequest;
    readonly signal: AbortSignal;
  }) => Promise<Result<BuilderStreamAbortHandle, TError>>;
  readonly setError: (message: string | null) => void;
  readonly completeStreaming: () => void;
}

export async function sendBuilderMessage<TError extends BuilderStreamErrorLike>(
  deps: SendBuilderMessageDeps<TError>
): Promise<(() => void) | null> {
  if (!deps.sessionId) return null;

  deps.addUserMessage(deps.content);
  deps.startStreaming();

  const abortController = new AbortController();
  const result = await deps.startStream({
    request: createBuilderStreamRequest(deps.sessionId, deps.history, deps.content),
    signal: abortController.signal,
  });

  if (result._tag === "Ok") {
    return () => {
      abortController.abort();
      result.value.abort();
    };
  }

  deps.setError(result.error.message);
  deps.completeStreaming();
  return null;
}

export function cancelBuilderStream(
  abortCurrent: (() => void) | null,
  cancelStreaming: () => void
): null {
  abortCurrent?.();
  cancelStreaming();
  return null;
}

export interface ResetBuilderSessionDeps {
  readonly confirmReset: () => boolean;
  readonly cancelStreaming: () => void;
  readonly sessionId: string | null | undefined;
  readonly clearSession: (sessionId: string) => Promise<unknown>;
  readonly clearLocalSession: () => void;
  readonly resetBuilder: () => void;
}

export async function resetBuilderSession(deps: ResetBuilderSessionDeps): Promise<boolean> {
  if (!deps.confirmReset()) return false;

  deps.cancelStreaming();

  if (deps.sessionId) {
    try {
      await deps.clearSession(deps.sessionId);
    } catch {
      // Best-effort cleanup
    }
  }

  deps.clearLocalSession();
  deps.resetBuilder();
  return true;
}
