import { getApiConfig } from "../api/client";
import type { ApiError } from "../domain/errors";
import { makeApiError } from "../domain/errors";
import { err, ok, type Result } from "../domain/result";

export interface JsonSSEHandlers<E> {
  onEvent: (event: E) => void;
  onComplete: () => void;
  onError: (error: ApiError) => void;
  onEventId?: (eventId: string) => void;
}

export interface JsonSSERequestOptions {
  readonly lastEventId?: string;
  readonly headers?: HeadersInit;
}

export type TerminalCheck<E> = (event: E) => { error?: string } | null;
export type EventDecoder<E> = (parsed: unknown) => E | null;

function makeNetworkError(message: string): ApiError {
  return makeApiError({
    code: "NETWORK_ERROR",
    message,
  });
}

function parseEventPayload<E>(
  jsonStr: string,
  handlers: JsonSSEHandlers<E>,
  isTerminal: TerminalCheck<E>,
  decodeEvent?: EventDecoder<E>
): { terminal: boolean } {
  try {
    const parsed = JSON.parse(jsonStr) as unknown;
    if (typeof parsed !== "object" || parsed === null) {
      return { terminal: false };
    }

    const maybeTyped = parsed as { type?: unknown; message?: unknown };
    if (typeof maybeTyped.type !== "string") {
      return { terminal: false };
    }

    if (maybeTyped.type === "resync") {
      const message =
        typeof maybeTyped.message === "string" && maybeTyped.message.trim().length > 0
          ? maybeTyped.message
          : "Stream lagged, please reconnect";
      handlers.onError(makeNetworkError(message));
      return { terminal: true };
    }

    const event = decodeEvent ? decodeEvent(parsed) : (parsed as E);
    if (event === null) {
      return { terminal: false };
    }

    handlers.onEvent(event);

    const terminal = isTerminal(event);
    if (terminal) {
      if (terminal.error) {
        handlers.onError(
          makeApiError({
            code: "SERVER_ERROR",
            message: terminal.error,
          })
        );
      } else {
        handlers.onComplete();
      }
      return { terminal: true };
    }
  } catch (error) {
    console.debug(
      "[sse] Malformed JSON event:",
      jsonStr,
      error instanceof Error ? error.message : error
    );
  }

  return { terminal: false };
}

function processEventBlock<E>(
  block: string,
  handlers: JsonSSEHandlers<E>,
  isTerminal: TerminalCheck<E>,
  decodeEvent?: EventDecoder<E>
): { terminal: boolean } {
  if (!block.trim()) {
    return { terminal: false };
  }

  let eventId: string | undefined;
  const dataLines: string[] = [];

  for (const line of block.split("\n")) {
    if (!line.trim() || line.startsWith(":")) {
      continue;
    }

    if (line.startsWith("id:")) {
      eventId = line.slice(3).trim();
      continue;
    }

    if (line.startsWith("data:")) {
      dataLines.push(line.slice(5).trimStart());
    }
  }

  if (eventId) {
    handlers.onEventId?.(eventId);
  }

  if (dataLines.length === 0) {
    return { terminal: false };
  }

  if (dataLines.length === 1) {
    return parseEventPayload(dataLines[0], handlers, isTerminal, decodeEvent);
  }

  const joinedPayload = dataLines.join("\n");
  const joinedResult = parseEventPayload(joinedPayload, handlers, isTerminal, decodeEvent);
  if (joinedResult.terminal) {
    return joinedResult;
  }

  // Studio historically emitted multiple standalone JSON payloads in one block.
  // Fall back to per-line parsing so that legacy streams still render.
  for (const dataLine of dataLines) {
    const lineResult = parseEventPayload(dataLine, handlers, isTerminal, decodeEvent);
    if (lineResult.terminal) {
      return lineResult;
    }
  }

  return { terminal: false };
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: SSE parser state machine
export async function readJsonSSEStream<E>(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  handlers: JsonSSEHandlers<E>,
  isTerminal: TerminalCheck<E>,
  signal?: AbortSignal,
  decodeEvent?: EventDecoder<E>
): Promise<void> {
  const decoder = new TextDecoder();
  let buffer = "";
  let settled = false;

  const settle = (error?: ApiError) => {
    if (settled) {
      return;
    }
    settled = true;
    if (error) {
      handlers.onError(error);
    } else {
      handlers.onComplete();
    }
  };

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

      const blocks = buffer.split("\n\n");
      buffer = blocks.pop() ?? "";

      for (const block of blocks) {
        const result = processEventBlock(block, handlers, isTerminal, decodeEvent);
        if (result.terminal) {
          return;
        }
      }
    }

    if (buffer.trim()) {
      const result = processEventBlock(buffer, handlers, isTerminal, decodeEvent);
      if (result.terminal) {
        return;
      }
    }

    if (signal?.aborted) {
      settle();
    } else {
      settle(makeNetworkError("Stream closed without completion event"));
    }
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") {
      settle();
    } else {
      settle(makeNetworkError(error instanceof Error ? error.message : "Stream error"));
    }
  } finally {
    try {
      await reader.cancel();
    } catch {
      // Ignore: cancellation is best-effort cleanup only.
    }
  }
}

export async function startJsonSSEStream<E>(
  method: "GET" | "POST",
  path: string,
  body: unknown | undefined,
  handlers: JsonSSEHandlers<E>,
  isTerminal: TerminalCheck<E>,
  options: JsonSSERequestOptions | string = {},
  decodeEvent?: EventDecoder<E>
): Promise<Result<{ abort: () => void }, ApiError>> {
  const apiConfig = getApiConfig();
  const url = `${apiConfig.baseUrl}${path}`;
  const controller = new AbortController();
  const lastEventId = typeof options === "string" ? options.trim() : options.lastEventId?.trim();
  const extraHeaders = typeof options === "string" ? undefined : options.headers;

  try {
    const response = await fetch(url, {
      method,
      headers: {
        ...apiConfig.headers,
        Accept: "text/event-stream",
        ...(lastEventId ? { "Last-Event-ID": lastEventId } : {}),
        ...extraHeaders,
      },
      ...(body !== undefined ? { body: JSON.stringify(body) } : {}),
      signal: controller.signal,
    });

    if (!response.ok) {
      const errorText = await response.text().catch(() => "");
      return err(
        makeApiError({
          code: response.status === 404 ? "NOT_FOUND" : "SERVER_ERROR",
          message: errorText || response.statusText,
          status: response.status,
        })
      );
    }

    const reader = response.body?.getReader();
    if (!reader) {
      return err(
        makeApiError({
          code: "SERVER_ERROR",
          message: "No response body",
        })
      );
    }

    void readJsonSSEStream(reader, handlers, isTerminal, controller.signal, decodeEvent);

    return ok({ abort: () => controller.abort() });
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") {
      return ok({ abort: () => {} });
    }
    return err(makeNetworkError(error instanceof Error ? error.message : "Failed to connect"));
  }
}
