/**
 * Generic JSON SSE stream reader.
 *
 * Shared by eval and data/ingestion streams. Eliminates near-identical
 * readEvalSSEStream / readIngestionSSEStream implementations.
 *
 * Principle: parametric polymorphism over event type E. The only
 * domain-specific knowledge is the terminal predicate.
 *
 * @module api/sse
 */

import type { Result } from "~/lib/domain/result";
import { err, ok } from "~/lib/domain/result";
import type { ApiError } from "./client";
import { getApiConfig } from "./client";

// =============================================================================
// Handler Interface
// =============================================================================

/**
 * Callbacks for a JSON SSE stream.
 *
 * Generic over the event type E — callers provide domain-specific types
 * (EvalEvent, IngestionEvent) while the reader stays generic.
 */
export interface JsonSSEHandlers<E> {
  onEvent: (event: E) => void;
  onComplete: () => void;
  onError: (error: ApiError) => void;
}

/**
 * Predicate to identify terminal events (completed/error).
 *
 * Returns:
 * - `null` for non-terminal events (keep streaming)
 * - `{ error: string }` for error terminal events
 * - `{ error: undefined }` for success terminal events
 */
export type TerminalCheck<E> = (event: E) => { error?: string } | null;

// =============================================================================
// Generic SSE Reader
// =============================================================================

/**
 * Read a JSON SSE stream, parsing each `data: {...}` line as type E.
 *
 * Terminal events (identified by `isTerminal`) trigger onComplete/onError
 * and stop reading.
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: SSE parser state machine
export async function readJsonSSEStream<E>(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  handlers: JsonSSEHandlers<E>,
  isTerminal: TerminalCheck<E>,
  signal?: AbortSignal
): Promise<void> {
  const decoder = new TextDecoder();
  let buffer = "";

  // Exactly-once completion semantics: once settled, no further callbacks fire.
  // Same pattern as Promise resolution — a stream can only reach one terminal state.
  //
  // onError and onComplete are mutually exclusive: an error settlement calls
  // onError only, a clean settlement calls onComplete only.  Calling both would
  // let onComplete overwrite state that onError just set (e.g. a reconnect
  // attempt or degraded-mode transition).
  let settled = false;
  const settle = (error?: ApiError) => {
    if (settled) return;
    settled = true;
    if (error) {
      handlers.onError(error);
    } else {
      handlers.onComplete();
    }
  };

  try {
    while (true) {
      if (signal?.aborted) break;

      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });

      const lines = buffer.split("\n\n");
      buffer = lines.pop() ?? "";

      for (const line of lines) {
        if (!line.trim()) continue;

        const dataLines = line.split("\n");
        for (const dataLine of dataLines) {
          if (!dataLine.startsWith("data: ")) continue;
          const jsonStr = dataLine.slice(6);
          try {
            const event = JSON.parse(jsonStr) as E;
            handlers.onEvent(event);

            const terminal = isTerminal(event);
            if (terminal) {
              settle(
                terminal.error ? { code: "SERVER_ERROR", message: terminal.error } : undefined
              );
              return;
            }
          } catch (e) {
            console.debug("[sse] Malformed JSON event:", jsonStr, e);
          }
        }
      }
    }

    // Stream ended without a terminal event
    if (signal?.aborted) {
      settle();
    } else {
      settle({
        code: "NETWORK_ERROR",
        message: "Stream closed without completion event",
      });
    }
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") {
      settle();
    } else {
      settle({
        code: "NETWORK_ERROR",
        message: error instanceof Error ? error.message : "Stream error",
      });
    }
  } finally {
    // Release the reader lock so the underlying stream can be GC'd promptly.
    // The stream may already be closed (e.g. after abort), so ignore errors.
    try {
      reader.cancel();
    } catch {
      // Ignore — stream may already be closed or errored.
    }
  }
}

// =============================================================================
// Generic SSE Stream Starter
// =============================================================================

/**
 * Start a JSON SSE stream via HTTP request.
 *
 * Shared by startEvalStream, connectEvalStream, rerunEvalCases,
 * startIngestionStream, extractKnowledge — eliminates 5 near-identical
 * fetch+reader+abort patterns.
 */
export async function startJsonSSEStream<E>(
  method: "GET" | "POST",
  path: string,
  body: unknown | undefined,
  handlers: JsonSSEHandlers<E>,
  isTerminal: TerminalCheck<E>,
  lastEventId?: string
): Promise<Result<{ abort: () => void }, ApiError>> {
  const apiConfig = getApiConfig();
  const url = `${apiConfig.baseUrl}${path}`;
  const controller = new AbortController();

  try {
    const response = await fetch(url, {
      method,
      headers: {
        ...apiConfig.headers,
        Accept: "text/event-stream",
        ...(lastEventId ? { "Last-Event-ID": lastEventId } : {}),
      },
      ...(body !== undefined ? { body: JSON.stringify(body) } : {}),
      signal: controller.signal,
    });

    if (!response.ok) {
      const errorText = await response.text().catch(() => "");
      return err({
        code: response.status === 404 ? "NOT_FOUND" : "SERVER_ERROR",
        message: errorText || response.statusText,
        status: response.status,
      });
    }

    const reader = response.body?.getReader();
    if (!reader) {
      return err({ code: "SERVER_ERROR", message: "No response body" });
    }

    // Intentionally NOT awaited. readJsonSSEStream runs in the background,
    // processing events as they arrive via handlers.onEvent(). The caller
    // receives an abort handle immediately. The settled guard inside
    // readJsonSSEStream ensures exactly-once completion semantics regardless
    // of when the promise settles relative to this function's return.
    readJsonSSEStream(reader, handlers, isTerminal, controller.signal);

    return ok({ abort: () => controller.abort() });
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
