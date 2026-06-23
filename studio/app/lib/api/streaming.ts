/**
 * SSE streaming client for chat endpoint.
 *
 * Handles Server-Sent Events from the Rust backend with:
 * - AbortController for cancellation
 * - Automatic reconnection (optional)
 * - Type-safe stream part parsing
 *
 * @module api/streaming
 */

import type { ModelSettings, ReasoningEffort } from "~/lib/domain/model-settings";
import { modelSettingsToChatFields } from "~/lib/domain/model-settings";
import type { Result } from "~/lib/domain/result";
import { err, ok } from "~/lib/domain/result";
import type { StreamPart } from "~/lib/domain/stream-part";
import { parseSSELine } from "~/lib/domain/stream-part";
import type { ApiError } from "./client";
import { getApiConfig, getApiRequestHeaders } from "./client";

// ============================================================================
// Types
// ============================================================================

/**
 * Chat request payload.
 */
export interface ChatRequest {
  readonly threadId: string;
  readonly messages: ChatRequestMessage[];
  readonly agentModels?: Record<string, string>;
  readonly agentEndpoints?: Record<string, AgentEndpointOverride>;
  readonly agentId?: string;
  readonly role?: string;
  readonly sessionId?: string;
  readonly maxTokens?: number;
  readonly thinkingBudgetTokens?: number;
  readonly reasoningEffort?: ReasoningEffort;
  readonly cacheControl?: boolean;
}

export interface AgentEndpointOverride {
  readonly transport: string;
  readonly settings: Record<string, string>;
  readonly targetModel?: string;
}

/**
 * Individual message in chat request.
 */
export interface ChatRequestMessage {
  readonly role: "user" | "assistant" | "system";
  readonly content: string;
}

/**
 * Stream event handler callbacks.
 */
export interface StreamHandlers {
  /** Called for each stream part */
  onPart: (part: StreamPart) => void;
  /** Called when stream completes normally */
  onComplete: () => void;
  /** Called on error */
  onError: (error: ApiError) => void;
  /** Called when first chunk arrives (optional) */
  onFirstChunk?: () => void;
}

// ============================================================================
// SSE Stream Reader
// ============================================================================

/**
 * Read SSE stream and emit parts.
 *
 * This is the core streaming function that handles:
 * - Line buffering for SSE format
 * - Part parsing and validation
 * - Error handling
 * - Cancellation via AbortSignal
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: stream event dispatcher
async function readSSEStream(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  handlers: StreamHandlers,
  signal?: AbortSignal
): Promise<void> {
  const decoder = new TextDecoder();
  let buffer = "";
  let firstChunkReceived = false;

  try {
    while (true) {
      // Check for cancellation
      if (signal?.aborted) {
        break;
      }

      const { done, value } = await reader.read();

      if (done) {
        break;
      }

      // Decode chunk and add to buffer
      buffer += decoder.decode(value, { stream: true });

      // Emit first chunk callback
      if (!firstChunkReceived) {
        firstChunkReceived = true;
        handlers.onFirstChunk?.();
      }

      // Process complete lines (SSE format: "data: {...}\n\n")
      const lines = buffer.split("\n\n");
      buffer = lines.pop() ?? ""; // Keep incomplete line in buffer

      for (const line of lines) {
        if (!line.trim()) continue;

        // Parse each "data:" line
        const dataLines = line.split("\n");
        for (const dataLine of dataLines) {
          const part = parseSSELine(dataLine);
          if (part) {
            handlers.onPart(part);

            // Check for terminal events
            if (part.type === "finish" || part.type === "error") {
              if (part.type === "error") {
                handlers.onError({
                  code: "SERVER_ERROR",
                  message: part.error.message,
                });
                return;
              }
            }
          }
        }
      }
    }

    // Process any remaining buffer
    if (buffer.trim()) {
      const part = parseSSELine(buffer);
      if (part) {
        handlers.onPart(part);
      }
    }

    handlers.onComplete();
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") {
      // Cancellation is not an error
      handlers.onComplete();
    } else {
      handlers.onError({
        code: "NETWORK_ERROR",
        message: error instanceof Error ? error.message : "Stream error",
      });
    }
  }
}

// ============================================================================
// Public API
// ============================================================================

/**
 * Start a streaming chat request.
 *
 * Returns an object with the abort function for cancellation.
 *
 * @example
 * ```ts
 * const { abort } = await startChatStream(
 *   { threadId: "123", messages: [...] },
 *   {
 *     onPart: (part) => console.log(part),
 *     onComplete: () => console.log("done"),
 *     onError: (err) => console.error(err),
 *   }
 * );
 *
 * // To cancel:
 * abort();
 * ```
 */
export async function startChatStream(
  request: ChatRequest,
  handlers: StreamHandlers,
  signal?: AbortSignal
): Promise<Result<{ abort: () => void }, ApiError>> {
  const config = getApiConfig();
  const url = `${config.baseUrl}/chat-with-abort`;

  // Create internal abort controller that can be triggered externally
  const controller = new AbortController();

  // Link external signal if provided (once: auto-removes after firing)
  if (signal) {
    signal.addEventListener("abort", () => controller.abort(), { once: true });
  }

  try {
    const response = await fetch(url, {
      method: "POST",
      headers: getApiRequestHeaders({ Accept: "text/event-stream" }),
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

    // Start reading stream in background
    readSSEStream(reader, handlers, controller.signal);

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

/**
 * Create a chat request from user input.
 */
export function createChatRequest(
  threadId: string,
  userMessage: string,
  history: ChatRequestMessage[] = [],
  options?: {
    agentId?: string;
    role?: string;
    sessionId?: string;
    agentModels?: Record<string, string>;
    agentEndpoints?: Record<string, AgentEndpointOverride>;
    modelSettings?: ModelSettings;
    maxTokens?: number;
    thinkingBudgetTokens?: number;
    reasoningEffort?: ReasoningEffort;
    cacheControl?: boolean;
  }
): ChatRequest {
  const modelSettings = options?.modelSettings
    ? modelSettingsToChatFields(options.modelSettings)
    : {
        maxTokens: options?.maxTokens,
        thinkingBudgetTokens: options?.thinkingBudgetTokens,
        reasoningEffort: options?.reasoningEffort,
        cacheControl: options?.cacheControl,
      };

  return {
    threadId,
    messages: [...history, { role: "user", content: userMessage }],
    agentModels: options?.agentModels,
    agentEndpoints: options?.agentEndpoints,
    agentId: options?.agentId,
    role: options?.role,
    sessionId: options?.sessionId,
    ...modelSettings,
  };
}
