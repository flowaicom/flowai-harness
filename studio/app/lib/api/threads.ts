/**
 * Thread API functions.
 *
 * CRUD operations for conversation threads.
 *
 * @module api/threads
 */

import type { BackendMessage, ResponseValidation } from "~/lib/domain/message";
import type { Result } from "~/lib/domain/result";
import { isErr, ok } from "~/lib/domain/result";
import type {
  CreateThreadRequest,
  Thread,
  ThreadId,
  UpdateThreadRequest,
} from "~/lib/domain/thread";
import { getFlowAIStudioConfig } from "~/lib/studio-config/flowai-config";
import type { ApiError } from "./client";
import { del, get, getApiConfig, post, put } from "./client";

function activeWorkspaceKey(): string {
  const header = getApiConfig().headers["X-Workspace-Id"];
  return header || getFlowAIStudioConfig().defaultWorkspaceKey;
}

function workspacePath(...segments: readonly string[]): string {
  const encoded = [activeWorkspaceKey(), ...segments].map((segment) => encodeURIComponent(segment));
  return `/workspaces/${encoded.join("/")}`;
}

// ============================================================================
// Thread Endpoints
// ============================================================================

/**
 * List all threads for the current user.
 */
export async function listThreads(): Promise<Result<Thread[], ApiError>> {
  return get<{ threads: Thread[] }>(workspacePath("threads")).then((result) =>
    isErr(result) ? result : ok(result.value.threads)
  );
}

/**
 * Get a single thread by ID.
 */
export async function getThread(id: ThreadId): Promise<Result<Thread, ApiError>> {
  return get<{ thread: Thread }>(workspacePath("threads", id)).then((result) =>
    isErr(result) ? result : ok(result.value.thread)
  );
}

/**
 * Create a new thread.
 */
export async function createThread(
  request: CreateThreadRequest = {}
): Promise<Result<Thread, ApiError>> {
  return post<Thread>("/threads", request);
}

/**
 * Update a thread.
 */
export async function updateThread(
  id: ThreadId,
  request: UpdateThreadRequest
): Promise<Result<Thread, ApiError>> {
  return put<Thread>(`/threads/${id}`, request);
}

/**
 * Delete a thread.
 */
export async function deleteThread(id: ThreadId): Promise<Result<void, ApiError>> {
  return del<void>(workspacePath("threads", id));
}

// ============================================================================
// Thread Messages
// ============================================================================

// Re-export BackendMessage from domain layer for downstream consumers.
export type { BackendMessage } from "~/lib/domain/message";

/**
 * Get messages for a thread.
 */
export async function getThreadMessages(
  threadId: ThreadId
): Promise<Result<BackendMessage[], ApiError>> {
  return get<{ messages: BackendMessage[] }>(workspacePath("threads", threadId, "messages")).then(
    (result) => (isErr(result) ? result : ok(result.value.messages))
  );
}

// ============================================================================
// Thread Forking (Phase C)
// ============================================================================

/**
 * Fork a thread at a specific message index.
 *
 * Creates a new thread with messages[0..forkAtMessageIndex] + edited message.
 */
export async function forkThread(
  threadId: ThreadId,
  forkAtMessageIndex: number,
  editedContent: string
): Promise<Result<{ threadId: string }, ApiError>> {
  return post<{ threadId: string }>(`/threads/${threadId}/fork`, {
    forkAtMessageIndex,
    editedContent,
  });
}

// ============================================================================
// Thread Files
// ============================================================================

/**
 * Thread file metadata.
 */
export interface ThreadFile {
  readonly fileId: string;
  readonly filename: string;
  readonly threadId: string;
  readonly createdAt: string;
}

export interface ThreadRunUsage {
  readonly inputTokens: number;
  readonly outputTokens: number;
  readonly cachedTokens: number;
  readonly cacheCreationTokens: number;
  readonly totalTokens: number;
}

export interface ThreadRunPhase {
  readonly index: number;
  readonly label: string;
  readonly milestone?: Record<string, unknown>;
}

export interface ThreadRunToolCall {
  readonly index: number;
  readonly toolCallId: string;
  readonly toolName: string;
  readonly state: string;
  readonly status: string;
  readonly args?: unknown;
  readonly result?: unknown;
  readonly durationMs?: number;
  readonly payloadSize?: number;
  readonly progress?: {
    readonly currentPhaseIndex: number;
    readonly totalPhases: number;
    readonly phases: readonly ThreadRunPhase[];
  };
}

export interface ThreadRunSubAgent {
  readonly index: number;
  readonly invocationId: string;
  readonly agentName: string;
  readonly state: string;
  readonly usage?: {
    readonly promptTokens: number;
    readonly completionTokens: number;
    readonly cacheReadInputTokens?: number;
    readonly cacheCreationInputTokens?: number;
    readonly totalTokens: number;
  };
}

export interface ThreadRunLatency {
  readonly totalDurationMs: number;
  readonly ttftMs?: number;
  readonly firstTextMs?: number;
  readonly retryCount: number;
  readonly hadTimeout: boolean;
  readonly phases: {
    readonly llmTimeMs: number;
    readonly toolTimeMs: number;
    readonly llmCalls: number;
  };
  readonly toolTimings: readonly {
    readonly toolName: string;
    readonly toolCallId: string;
    readonly durationMs: number;
    readonly status: string;
    readonly payloadSize?: number;
  }[];
}

export interface ThreadRunReport {
  readonly runId: string;
  readonly threadId: string;
  readonly role: string;
  readonly model: string;
  readonly startedAt: string;
  readonly finishedAt: string;
  readonly status: "completed" | "error" | "incomplete";
  readonly finishReason?: string | null;
  readonly error?: {
    readonly message: string;
    readonly code?: string | null;
  } | null;
  readonly usage?: ThreadRunUsage | null;
  readonly latency?: ThreadRunLatency | null;
  readonly responseValidation?: ResponseValidation | null;
  readonly toolCalls: readonly ThreadRunToolCall[];
  readonly subAgents: readonly ThreadRunSubAgent[];
  readonly delegationChain: readonly string[];
  readonly outputPreview: string;
  readonly outputTextLength: number;
  readonly userMessageId?: string | null;
  readonly assistantMessageId?: string | null;
  readonly summary?: {
    readonly toolCount: number;
    readonly subAgentCount: number;
    readonly hasResponseContract: boolean;
    readonly responseContractOk?: boolean | null;
  };
}

/**
 * List files for a thread.
 */
export async function listThreadFiles(threadId: ThreadId): Promise<Result<ThreadFile[], ApiError>> {
  return get<ThreadFile[]>(`/threads/${threadId}/files`);
}

/**
 * Get the latest persisted run report for a thread.
 */
export async function getLatestThreadRunReport(
  threadId: ThreadId
): Promise<Result<ThreadRunReport, ApiError>> {
  return get<ThreadRunReport>(`/threads/${threadId}/runs/latest`);
}

/**
 * Get file download URL.
 */
export function getFileDownloadUrl(fileId: string): string {
  return `/api/files/${fileId}/download`;
}

// ============================================================================
// Sub-Agent Messages
// ============================================================================

/**
 * UI message format (already transformed by backend).
 */
export interface UIMessagePart {
  readonly type: string;
  [key: string]: unknown;
}

export interface UIMessage {
  readonly id: string;
  readonly role: "user" | "assistant" | "system";
  readonly parts: UIMessagePart[];
  readonly createdAt?: Date | string;
  readonly metadata?: {
    readonly createdAt?: string | Date;
    readonly threadId?: string;
    readonly resourceId?: string;
    [key: string]: unknown;
  };
}

/**
 * Result from fetchMessages - includes format indicator.
 */
export interface FetchMessagesResult {
  readonly messages: (UIMessage | BackendMessage)[];
  readonly format: "ui" | "backend";
}

/**
 * Parse messages response from API into FetchMessagesResult.
 * Handles multiple response formats.
 */
function parseMessagesResponse(data: unknown): FetchMessagesResult {
  if (!data || typeof data !== "object") {
    return { messages: [], format: "backend" };
  }

  const response = data as Record<string, unknown>;

  // Prefer uiMessages if available (already in AI SDK format)
  if (response.uiMessages && Array.isArray(response.uiMessages)) {
    return { messages: response.uiMessages as UIMessage[], format: "ui" };
  }

  // Handle wrapped response with messages array
  if (response.messages && Array.isArray(response.messages)) {
    return { messages: response.messages as BackendMessage[], format: "backend" };
  }

  // Handle raw array response
  if (Array.isArray(data)) {
    return { messages: data as BackendMessage[], format: "backend" };
  }

  return { messages: [], format: "backend" };
}

/**
 * Fetch messages for a sub-agent's thread.
 *
 * Used to load detailed history of what a sub-agent did (all its tool calls).
 * This is useful for expanding sub-agent execution details in the UI.
 *
 * @param threadId - The sub-agent's thread ID (from subAgentThreadId in tool call output)
 * @param agentId - The sub-agent's agent ID (e.g., 'mySubAgent')
 * @param signal - Optional AbortSignal for cancellation support
 */
export async function fetchSubAgentMessages(
  threadId: string,
  agentId: string,
  signal?: AbortSignal
): Promise<Result<FetchMessagesResult, ApiError>> {
  const result = await get<unknown>(
    `${workspacePath("threads", threadId, "messages")}?agentId=${encodeURIComponent(agentId)}`,
    { signal }
  );

  if (isErr(result)) {
    // Return empty silently for missing threads (expected for new threads)
    if (result.error.code === "NOT_FOUND") {
      return ok({ messages: [], format: "backend" });
    }
    return result;
  }

  return ok(parseMessagesResponse(result.value));
}

/**
 * Fetch messages with format detection.
 *
 * @param threadId - The thread ID to fetch messages for
 * @param signal - Optional AbortSignal for cancellation support
 */
export async function fetchMessagesWithFormat(
  threadId: ThreadId,
  signal?: AbortSignal
): Promise<Result<FetchMessagesResult, ApiError>> {
  const result = await get<unknown>(workspacePath("threads", threadId, "messages"), { signal });

  if (isErr(result)) {
    // Return empty silently for missing threads
    if (result.error.code === "NOT_FOUND") {
      return ok({ messages: [], format: "backend" });
    }
    return result;
  }

  return ok(parseMessagesResponse(result.value));
}

// ============================================================================
// Sub-Agent Responses
// ============================================================================

interface SubAgentResponseBody {
  responseId: string;
  body: string;
}

/** Fetch full sub-agent response text by its KV pointer. */
export const getSubAgentResponse = (
  responseId: string
): Promise<Result<SubAgentResponseBody, ApiError>> =>
  get<SubAgentResponseBody>(`/threads/sub-agent-response/${responseId}`);
