import { getApiConfig } from "../api/client";
import type { ApiError } from "../domain/errors";
import { err, ok, type Result } from "../domain/result";
import type { StreamPart } from "../domain/stream-part";
import { isTerminalPart, parseSSELine } from "../domain/stream-part";

export interface ChatRequestMessage {
  readonly role: "user" | "assistant" | "system";
  readonly content: string;
}

export interface ChatStreamHandlers {
  readonly onPart: (part: StreamPart) => void;
  readonly onComplete: () => void;
  readonly onError: (error: ApiError) => void;
  readonly onFirstChunk?: () => void;
}

async function readChatSSEStream(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  handlers: ChatStreamHandlers,
  signal?: AbortSignal
): Promise<void> {
  const decoder = new TextDecoder();
  let buffer = "";
  let firstChunkReceived = false;

  try {
    while (true) {
      if (signal?.aborted) {
        break;
      }

      const { done, value } = await reader.read();
      if (done) {
        break;
      }

      buffer += decoder.decode(value, { stream: true });

      if (!firstChunkReceived) {
        firstChunkReceived = true;
        handlers.onFirstChunk?.();
      }

      const lines = buffer.split("\n\n");
      buffer = lines.pop() ?? "";

      for (const line of lines) {
        if (!line.trim()) continue;

        const dataLines = line.split("\n");
        for (const dataLine of dataLines) {
          const part = parseSSELine(dataLine);
          if (!part) continue;

          handlers.onPart(part);

          if (isTerminalPart(part) && part.type === "error") {
            handlers.onError({
              code: "SERVER_ERROR",
              message: part.error.message,
            });
            return;
          }
        }
      }
    }

    if (buffer.trim()) {
      const part = parseSSELine(buffer);
      if (part) {
        handlers.onPart(part);
      }
    }

    handlers.onComplete();
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") {
      handlers.onComplete();
      return;
    }

    handlers.onError({
      code: "NETWORK_ERROR",
      message: error instanceof Error ? error.message : "Stream error",
    });
  }
}

export async function startChatStream<TRequest extends object>(
  request: TRequest,
  handlers: ChatStreamHandlers,
  signal?: AbortSignal
): Promise<Result<{ readonly abort: () => void }, ApiError>> {
  const config = getApiConfig();
  const url = `${config.baseUrl}/chat-with-abort`;
  const controller = new AbortController();

  if (signal) {
    signal.addEventListener("abort", () => controller.abort(), { once: true });
  }

  try {
    const response = await fetch(url, {
      method: "POST",
      headers: {
        ...config.headers,
        Accept: "text/event-stream",
      },
      body: JSON.stringify(request),
      signal: controller.signal,
    });

    if (!response.ok) {
      const errorText = await response.text().catch(() => "");
      return err({
        code: response.status === 401 ? "UNAUTHORIZED" : "SERVER_ERROR",
        message: errorText || response.statusText,
        status: response.status,
      });
    }

    const reader = response.body?.getReader();
    if (!reader) {
      return err({
        code: "SERVER_ERROR",
        message: "No response body",
      });
    }

    void readChatSSEStream(reader, handlers, controller.signal);

    return ok({
      abort: () => controller.abort(),
    });
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") {
      return ok({ abort: () => {} });
    }

    return err({
      code: "NETWORK_ERROR",
      message: error instanceof Error ? error.message : "Failed to connect",
    });
  }
}

export function createChatRequest<TExtra extends object = Record<string, never>>(
  threadId: string,
  userMessage: string,
  history: readonly ChatRequestMessage[] = [],
  extra?: TExtra
): { readonly threadId: string; readonly messages: ChatRequestMessage[] } & TExtra {
  return {
    threadId,
    messages: [...history, { role: "user", content: userMessage }],
    ...(extra ?? ({} as TExtra)),
  };
}
