import type { ApiError, ApiResult, StreamError } from "@studio/core/domain/errors";
import { makeApiError, makeStreamError } from "@studio/core/domain/errors";
import { err, ok, type Result } from "@studio/core/domain/result";
import { type AppScope, getWorkspaceKey } from "@studio/core/domain/scope";
import type {
  AgentSummary,
  AgentsResponse,
  ApprovalResponseInput,
  ApprovalSummary,
  CapabilitiesResponse,
  CatalogSearchResult,
  ChatStreamHandle,
  DataEventStreamHandle,
  DataEventStreamHandlers,
  DataSourceSummary,
  DocumentSummary,
  FlowAIHarnessRuntimeAdapter,
  HarnessChatStreamHandlers,
  HarnessChatStreamInput,
  KnowledgeItem,
  MetricSummary,
  ProfileEstimateResult,
  RuntimeSummary,
  SampleRowsResult,
  SearchToolResult,
  StudioEvent,
  StudioStatus,
  TableDetail,
  TableSummary,
  ThreadDetail,
  ThreadMessage,
  ThreadSummary,
  ToolExecutionResult,
  ToolSummary,
  WorkspaceSummary,
  WorkspacesResponse,
} from "@studio/core/runtime";
import { getApiConfig } from "~/lib/api/client";

const STUDIO_API_VERSION = "harness-studio/v1";

type Decoder<T> = (input: unknown) => T;

export function createFlowAIHarnessRuntimeAdapter(): FlowAIHarnessRuntimeAdapter {
  return {
    getStatus: () => requestJson("/status", decodeStatus),
    listWorkspaces: () => requestJson("/workspaces", decodeWorkspacesResponse),
    getWorkspace: (scope) => requestJson(workspacePath(scope), decodeWorkspaceSummary),
    getRuntime: (scope) => requestJson(workspacePath(scope, "runtime"), decodeRuntimeSummary),
    getCapabilities: (scope) =>
      requestJson(workspacePath(scope, "capabilities"), decodeCapabilitiesResponse),
    listAgents: (scope) => requestJson(workspacePath(scope, "agents"), decodeAgentsResponse),
    listThreads: (scope) => requestJson(workspacePath(scope, "threads"), decodeThreadsResponse),
    getThread: (scope, threadId) =>
      requestJson(workspacePath(scope, "threads", threadId), decodeThreadResponse),
    deleteThread: (scope, threadId) =>
      requestJson(workspacePath(scope, "threads", threadId), decodeDeleteThreadResponse, {
        method: "DELETE",
      }),
    listThreadMessages: (scope, threadId) =>
      requestJson(workspacePath(scope, "threads", threadId, "messages"), decodeMessagesResponse),
    startChatStream,
    respondToApproval: (scope, approvalId, input) =>
      requestJson(workspacePath(scope, "approvals", approvalId, "respond"), decodeApprovalSummary, {
        method: "POST",
        body: normalizeApprovalInput(input),
      }),
    listToolkits: () =>
      Promise.resolve(
        ok([
          {
            toolkitId: "catalog",
            name: "Catalog tools",
            enabled: true,
            metadata: { source: "flowai-runtime" },
          },
        ])
      ),
    listTools: (scope) => requestJson(workspacePath(scope, "tools"), decodeToolsResponse),
    listAgentTools: () =>
      notImplemented("tools.inspect", "Agent tool inspection is not available in harness mode."),
    executeTool: (scope, input) =>
      requestJson(
        workspacePath(scope, "tools", input.toolId, "execute"),
        decodeToolExecutionResponse,
        {
          method: "POST",
          body: {
            input: input.input ?? {},
            ...(input.agentId ? { agentId: input.agentId } : {}),
          },
        }
      ),
    listDataSources: (scope) =>
      requestJson(workspacePath(scope, "data", "sources"), decodeDataSourcesResponse),
    listSchemas: (scope, input) =>
      requestJson(
        `${workspacePath(scope, "data", "discovery", "schemas")}${queryString({
          sourceId: input?.sourceId,
        })}`,
        decodeSchemasResponse,
        { signal: input?.signal }
      ),
    listTables: (scope, input) =>
      requestJson(
        `${workspacePath(scope, "data", "discovery", "tables")}${queryString({
          schema: input?.schema,
          sourceId: input?.sourceId,
        })}`,
        decodeTablesResponse,
        { signal: input?.signal }
      ),
    getTableDetail: (scope, input) =>
      requestJson(
        `${workspacePath(scope, "data", "discovery", "tables", input.tableName)}${queryString({
          schema: input.schema,
          sourceId: input.sourceId,
        })}`,
        decodeTableDetailResponse
      ),
    getTableColumns: (scope, input) =>
      requestJson(
        `${workspacePath(scope, "data", "discovery", "tables", input.tableName, "columns")}${queryString(
          {
            schema: input.schema,
            sourceId: input.sourceId,
          }
        )}`,
        decodeTableColumnsResponse,
        { signal: input.signal }
      ),
    sampleRows: (scope, input) =>
      requestJson(
        `${workspacePath(scope, "data", "discovery", "tables", input.tableName, "sample")}${queryString(
          {
            schema: input.schema,
            sourceId: input.sourceId,
            limit: input.limit,
          }
        )}`,
        decodeSampleRowsResponse,
        { signal: input.signal }
      ),
    estimateProfile: (scope, input) =>
      requestJson(
        workspacePath(scope, "data", "profile", "estimate"),
        decodeProfileEstimateResponse,
        {
          method: "POST",
          body: input,
        }
      ),
    startProfileTable: (scope, input) =>
      startDataEventStream(
        scope,
        ["data", "profile", "table"],
        {
          ...input,
          handlers: undefined,
        },
        input.handlers
      ),
    startProfileDatabase: (scope, input) =>
      startDataEventStream(
        scope,
        ["data", "profile", "database"],
        {
          ...input,
          handlers: undefined,
        },
        input.handlers
      ),
    listDocuments: (scope) =>
      requestJson(workspacePath(scope, "data", "knowledge", "documents"), decodeDocumentsResponse),
    browseKnowledge: (scope, input) =>
      requestJson(
        `${workspacePath(scope, "data", "knowledge", "items")}${queryString({
          sourceId: input?.sourceId,
          query: input?.query,
          limit: input?.limit,
        })}`,
        decodeKnowledgeItemsResponse
      ),
    ingestKnowledge: (scope, input) =>
      startDataEventStream(
        scope,
        ["data", "knowledge", "ingest"],
        {
          source: input.source,
          extractKnowledge: input.extractKnowledge,
        },
        input.handlers
      ),
    listMetrics: (scope) =>
      requestJson(workspacePath(scope, "data", "metrics"), decodeMetricsResponse),
    searchCatalog: (scope, input) =>
      requestJson(workspacePath(scope, "data", "search"), decodeCatalogSearchResponse, {
        method: "POST",
        body: input,
      }),
    runSearchTool: (scope, input) =>
      requestJson(
        workspacePath(scope, "tools", searchToolIdForMode(input.mode), "execute"),
        decodeSearchToolResponse,
        {
          method: "POST",
          body: { input: searchToolInputForMode(input) },
        }
      ),
    startImport: () =>
      notImplemented("data.import", "Import is not available in harness mode yet."),
    listTests: () => notImplemented("tests.manage", "Tests are not available in harness mode."),
    getTest: () => notImplemented("tests.manage", "Test detail is not available in harness mode."),
    createTest: () =>
      notImplemented("tests.manage", "Test creation is not available in harness mode."),
    listEvalRuns: () => notImplemented("evals.run", "Eval runs are not available in harness mode."),
    getEvalRun: () => notImplemented("evals.run", "Eval detail is not available in harness mode."),
    startEvalRun: () =>
      notImplemented("evals.run", "Eval execution is not available in harness mode."),
    listRuns: () => notImplemented("runs.list", "Runs are not available in harness mode."),
    getRun: () => notImplemented("runs.list", "Run detail is not available in harness mode."),
    listRunEvents: () =>
      notImplemented("traces.read", "Run events are not available in harness mode."),
  };
}

export const flowAIHarnessRuntimeAdapter = createFlowAIHarnessRuntimeAdapter();

export function workspacePath(scope: AppScope, ...segments: readonly string[]): string {
  const encoded = [getWorkspaceKey(scope), ...segments].map((segment) =>
    encodeURIComponent(segment)
  );
  return `/workspaces/${encoded.join("/")}`;
}

function queryString(params: Record<string, string | number | undefined>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== "") {
      search.set(key, String(value));
    }
  }
  const serialized = search.toString();
  return serialized ? `?${serialized}` : "";
}

async function requestJson<T>(
  path: string,
  decode: Decoder<T>,
  options: {
    readonly method?: "GET" | "POST" | "DELETE";
    readonly body?: unknown;
    readonly signal?: AbortSignal;
  } = {}
): Promise<ApiResult<T>> {
  const apiConfig = getApiConfig();
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), apiConfig.timeout);
  const signal = options.signal
    ? AbortSignal.any([options.signal, controller.signal])
    : controller.signal;

  try {
    const response = await fetch(`${apiConfig.baseUrl}${path}`, {
      method: options.method ?? "GET",
      headers: apiConfig.headers,
      body: options.body === undefined ? undefined : JSON.stringify(options.body),
      signal,
    });
    clearTimeout(timeoutId);

    if (!response.ok) {
      return err(await errorFromResponse(response));
    }

    const text = await response.text();
    try {
      const parsed = text.length > 0 ? JSON.parse(text) : {};
      return ok(decode(parsed));
    } catch (error) {
      return err(
        makeApiError({
          code: "VALIDATION_ERROR",
          message: error instanceof Error ? error.message : "Invalid response shape.",
        })
      );
    }
  } catch (error) {
    clearTimeout(timeoutId);
    if (error instanceof DOMException && error.name === "AbortError") {
      return err(makeApiError({ code: "TIMEOUT", message: "Request timed out" }));
    }
    return err(
      makeApiError({
        code: "NETWORK_ERROR",
        message: error instanceof Error ? error.message : "Network error",
      })
    );
  }
}

async function startChatStream(
  input: HarnessChatStreamInput,
  options: { readonly signal: AbortSignal; readonly scope: AppScope }
): Promise<Result<ChatStreamHandle, StreamError | ApiError>> {
  const apiConfig = getApiConfig();
  const controller = new AbortController();
  const runId = input.runId ?? createRunId();
  const abort = createChatAbort(options.scope, runId, controller);
  if (options.signal.aborted) {
    abort();
  } else {
    options.signal.addEventListener("abort", abort, { once: true });
  }
  const path = workspacePath(options.scope, "agents", input.agentId, "stream");

  try {
    const response = await fetch(`${apiConfig.baseUrl}${path}`, {
      method: "POST",
      headers: {
        ...apiConfig.headers,
        Accept: "text/event-stream",
      },
      body: JSON.stringify({
        prompt: input.prompt,
        threadId: input.threadId,
        runId,
        ...(input.metadata ? { metadata: input.metadata } : {}),
      }),
      signal: controller.signal,
    });

    if (!response.ok) {
      return err(await errorFromResponse(response));
    }

    const reader = response.body?.getReader();
    if (!reader) {
      return err(makeStreamError({ code: "STREAM_CONNECT_FAILED", message: "No response body" }));
    }

    if (input.handlers) {
      void readStudioEventStream(reader, input.handlers, controller.signal);
    }

    return ok({ abort });
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") {
      return ok({ abort: () => {} });
    }
    return err(
      makeApiError({
        code: "NETWORK_ERROR",
        message: error instanceof Error ? error.message : "Failed to connect",
      })
    );
  }
}

function createRunId(): string {
  const randomUUID = globalThis.crypto?.randomUUID;
  if (typeof randomUUID === "function") {
    return `run_${randomUUID.call(globalThis.crypto).replaceAll("-", "")}`;
  }
  return `run_${Date.now().toString(36)}_${Math.random().toString(36).slice(2)}`;
}

function createChatAbort(scope: AppScope, runId: string, controller: AbortController): () => void {
  let aborted = false;
  return () => {
    if (aborted) return;
    aborted = true;
    postRunCancellation(scope, runId);
    controller.abort();
  };
}

function postRunCancellation(scope: AppScope, runId: string): void {
  const apiConfig = getApiConfig();
  void fetch(`${apiConfig.baseUrl}${workspacePath(scope, "runs", runId, "cancel")}`, {
    method: "POST",
    headers: apiConfig.headers,
  }).catch(() => {
    // Abort is best-effort; the fetch close still stops local streaming.
  });
}

async function startDataEventStream(
  scope: AppScope,
  pathSegments: readonly string[],
  body: Record<string, unknown>,
  handlers?: DataEventStreamHandlers
): Promise<Result<DataEventStreamHandle, StreamError | ApiError>> {
  const apiConfig = getApiConfig();
  const controller = new AbortController();
  const path = workspacePath(scope, ...pathSegments);

  try {
    const response = await fetch(`${apiConfig.baseUrl}${path}`, {
      method: "POST",
      headers: {
        ...apiConfig.headers,
        Accept: "text/event-stream",
      },
      body: JSON.stringify(body),
      signal: controller.signal,
    });

    if (!response.ok) {
      return err(await errorFromResponse(response));
    }

    const reader = response.body?.getReader();
    if (!reader) {
      return err(makeStreamError({ code: "STREAM_CONNECT_FAILED", message: "No response body" }));
    }

    if (handlers) {
      void readJsonEventStream(reader, handlers, controller.signal);
    }

    return ok({ abort: () => controller.abort() });
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") {
      return ok({ abort: () => {} });
    }
    return err(
      makeApiError({
        code: "NETWORK_ERROR",
        message: error instanceof Error ? error.message : "Failed to connect",
      })
    );
  }
}

export async function readStudioEventStream(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  handlers: HarnessChatStreamHandlers,
  signal?: AbortSignal
): Promise<void> {
  const decoder = new TextDecoder();
  let buffer = "";
  let settled = false;
  let lastSeq = 0;

  const settle = (kind: "complete" | "error", error?: ApiError | StreamError) => {
    if (settled || signal?.aborted) return;
    settled = true;
    if (kind === "error" && error) {
      handlers.onError(error);
      return;
    }
    handlers.onComplete();
  };

  const processBlock = (block: string): boolean => {
    const parsed = parseSseBlock(block);
    if (!parsed) return false;
    if (parsed.eventId) {
      handlers.onEventId?.(parsed.eventId);
    }
    const eventResult = decodeStudioEvent(parsed.data);
    if (eventResult._tag === "Err") {
      settle("error", eventResult.error);
      return true;
    }
    const event = eventResult.value;
    if (event.seq <= lastSeq) {
      settle(
        "error",
        makeStreamError({
          code: "STREAM_PROTOCOL_ERROR",
          message: `Studio stream sequence must increase; received ${event.seq} after ${lastSeq}.`,
          details: { seq: event.seq, previousSeq: lastSeq },
        })
      );
      return true;
    }
    lastSeq = event.seq;
    if (signal?.aborted) return true;
    handlers.onEvent(event);
    if (event.kind === "run.completed") {
      settle("complete");
      return true;
    }
    if (event.kind === "run.failed") {
      settle("error", errorFromFailedEvent(event));
      return true;
    }
    if (event.kind === "run.cancelled" || event.kind === "run.aborted") {
      settle("complete");
      return true;
    }
    return false;
  };

  try {
    while (true) {
      if (signal?.aborted || settled) break;
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const blocks = buffer.split("\n\n");
      buffer = blocks.pop() ?? "";
      for (const block of blocks) {
        if (processBlock(block)) return;
      }
    }
    if (!settled && !signal?.aborted && buffer.trim() && processBlock(buffer)) {
      return;
    }
    if (!settled && !signal?.aborted) {
      settle(
        "error",
        makeStreamError({ code: "STREAM_CLOSED", message: "Stream closed without terminal event." })
      );
    }
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") {
      return;
    }
    settle(
      "error",
      makeStreamError({
        code: "STREAM_CONNECT_FAILED",
        message: error instanceof Error ? error.message : "Stream error",
      })
    );
  } finally {
    try {
      await reader.cancel();
    } catch {
      // Reader cancellation is best-effort cleanup only.
    }
  }
}

async function readJsonEventStream(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  handlers: DataEventStreamHandlers,
  signal?: AbortSignal
): Promise<void> {
  const decoder = new TextDecoder();
  let buffer = "";
  let settled = false;
  const settle = (kind: "complete" | "error", error?: ApiError | StreamError) => {
    if (settled || signal?.aborted) return;
    settled = true;
    if (kind === "error" && error) {
      handlers.onError(error);
      return;
    }
    handlers.onComplete();
  };
  const processBlock = (block: string): boolean => {
    const parsed = parseSseBlock(block);
    if (!parsed) return false;
    try {
      handlers.onEvent(JSON.parse(parsed.data));
      return false;
    } catch (error) {
      settle(
        "error",
        makeStreamError({
          code: "STREAM_DECODE_ERROR",
          message: error instanceof Error ? error.message : "Invalid data event JSON.",
        })
      );
      return true;
    }
  };

  try {
    while (true) {
      if (signal?.aborted || settled) break;
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const blocks = buffer.split("\n\n");
      buffer = blocks.pop() ?? "";
      for (const block of blocks) {
        if (processBlock(block)) return;
      }
    }
    if (!settled && !signal?.aborted && buffer.trim() && processBlock(buffer)) return;
    if (!settled && !signal?.aborted) settle("complete");
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") return;
    settle(
      "error",
      makeStreamError({
        code: "STREAM_CONNECT_FAILED",
        message: error instanceof Error ? error.message : "Stream error",
      })
    );
  } finally {
    try {
      await reader.cancel();
    } catch {
      // Reader cancellation is best-effort cleanup only.
    }
  }
}

function parseSseBlock(block: string): { eventId?: string; data: string } | null {
  if (!block.trim()) return null;
  let eventId: string | undefined;
  const dataLines: string[] = [];
  for (const line of block.split("\n")) {
    if (!line.trim() || line.startsWith(":")) continue;
    if (line.startsWith("id:")) {
      eventId = line.slice(3).trim();
      continue;
    }
    if (line.startsWith("data:")) {
      dataLines.push(line.slice(5).trimStart());
    }
  }
  return dataLines.length === 0 ? null : { eventId, data: dataLines.join("\n") };
}

function decodeStudioEvent(input: string): Result<StudioEvent, StreamError> {
  try {
    const event = JSON.parse(input) as unknown;
    if (!isObject(event)) {
      return err(
        makeStreamError({ code: "STREAM_DECODE_ERROR", message: "Studio event must be an object." })
      );
    }
    if (event.schemaVersion !== STUDIO_API_VERSION) {
      return err(
        makeStreamError({
          code: "STREAM_PROTOCOL_ERROR",
          message: "Unsupported Studio event schemaVersion.",
          details: { schemaVersion: event.schemaVersion },
        })
      );
    }
    if (
      typeof event.workspaceKey !== "string" ||
      typeof event.runId !== "string" ||
      typeof event.threadId !== "string" ||
      typeof event.agentId !== "string" ||
      typeof event.seq !== "number" ||
      typeof event.kind !== "string" ||
      !("payload" in event)
    ) {
      return err(
        makeStreamError({
          code: "STREAM_DECODE_ERROR",
          message: "Studio event is missing required envelope fields.",
          details: event,
        })
      );
    }
    return ok({
      schemaVersion: event.schemaVersion,
      workspaceKey: event.workspaceKey,
      runId: event.runId,
      threadId: event.threadId,
      agentId: event.agentId,
      seq: event.seq,
      kind: event.kind,
      payload: event.payload,
    });
  } catch (error) {
    return err(
      makeStreamError({
        code: "STREAM_DECODE_ERROR",
        message: error instanceof Error ? error.message : "Invalid Studio event JSON.",
      })
    );
  }
}

async function errorFromResponse(response: Response): Promise<ApiError> {
  const body = await response.text().catch(() => "");
  if (!body) {
    return makeApiError({
      code: statusToErrorCode(response.status),
      message: response.statusText || "Request failed",
      status: response.status,
    });
  }
  try {
    const parsed = JSON.parse(body) as unknown;
    if (isObject(parsed) && isObject(parsed.error)) {
      const serverCode = typeof parsed.error.code === "string" ? parsed.error.code : undefined;
      const message =
        typeof parsed.error.message === "string" ? parsed.error.message : response.statusText;
      return makeApiError({
        code: statusToErrorCode(response.status),
        message,
        status: response.status,
        details: {
          ...(isObject(parsed.error.details) ? parsed.error.details : {}),
          ...(serverCode ? { serverCode } : {}),
        },
      });
    }
    if (isObject(parsed) && Array.isArray(parsed.detail)) {
      return makeApiError({
        code: statusToErrorCode(response.status),
        message: "Request validation failed.",
        status: response.status,
        details: { detail: parsed.detail },
      });
    }
    return makeApiError({
      code: statusToErrorCode(response.status),
      message: body,
      status: response.status,
      details: parsed,
    });
  } catch {
    return makeApiError({
      code: statusToErrorCode(response.status),
      message: body,
      status: response.status,
    });
  }
}

function errorFromFailedEvent(event: StudioEvent): ApiError {
  const payload = isObject(event.payload) ? event.payload : {};
  const error = isObject(payload.error) ? payload.error : {};
  return makeApiError({
    code: "SERVER_ERROR",
    message: typeof error.message === "string" ? error.message : "Runtime stream failed.",
    details: {
      ...(isObject(error.details) ? error.details : {}),
      runId: event.runId,
      kind: event.kind,
    },
  });
}

function statusToErrorCode(status: number): ApiError["code"] {
  switch (status) {
    case 401:
      return "UNAUTHORIZED";
    case 403:
      return "FORBIDDEN";
    case 404:
      return "NOT_FOUND";
    case 422:
      return "VALIDATION_ERROR";
    case 501:
      return "NOT_IMPLEMENTED";
    default:
      return status >= 500 ? "SERVER_ERROR" : "UNKNOWN";
  }
}

function notImplemented<T>(capability: string, message: string): Promise<ApiResult<T>> {
  return Promise.resolve(
    err(
      makeApiError({
        code: "NOT_IMPLEMENTED",
        message,
        details: { capability },
      })
    )
  );
}

function normalizeApprovalInput(input: ApprovalResponseInput): ApprovalResponseInput {
  return input;
}

function decodeStatus(input: unknown): StudioStatus {
  const value = object(input);
  return {
    studioApiVersion: stringField(value, "studioApiVersion"),
    supportedVersions: stringArray(value.supportedVersions),
    status: stringField(value, "status"),
    implementation: optionalRecord(value.implementation),
  };
}

function decodeWorkspacesResponse(input: unknown): WorkspacesResponse {
  const value = object(input);
  return {
    defaultWorkspaceKey: stringField(value, "defaultWorkspaceKey"),
    workspaces: arrayOf(value.workspaces, decodeWorkspaceSummary),
  };
}

function decodeWorkspaceSummary(input: unknown): WorkspaceSummary {
  const value = object(input);
  return {
    workspaceKey: stringField(value, "workspaceKey"),
    displayName: optionalString(value.displayName),
    default: optionalBoolean(value.default),
    metadata: optionalRecord(value.metadata),
  };
}

function decodeRuntimeSummary(input: unknown): RuntimeSummary {
  const value = object(input);
  return {
    workspaceKey: stringField(value, "workspaceKey"),
    tenant: optionalRecord(value.tenant),
    providers: optionalArray(value.providers),
    agents: value.agents === undefined ? undefined : arrayOf(value.agents, decodeAgentSummary),
    references: optionalArray(value.references),
    runtime: optionalRecord(value.runtime),
  };
}

function decodeCapabilitiesResponse(input: unknown): CapabilitiesResponse {
  const value = object(input);
  return {
    workspaceKey: stringField(value, "workspaceKey"),
    capabilities: arrayOf(value.capabilities, (item) => {
      const capability = object(item);
      return {
        id: stringField(capability, "id"),
        enabled: booleanField(capability, "enabled"),
        scope: stringField(capability, "scope"),
        reason: optionalString(capability.reason),
        requirements:
          capability.requirements === undefined ? undefined : stringArray(capability.requirements),
      };
    }),
  };
}

function decodeAgentsResponse(input: unknown): AgentsResponse {
  const value = object(input);
  return {
    workspaceKey: stringField(value, "workspaceKey"),
    agents: arrayOf(value.agents, decodeAgentSummary),
  };
}

function decodeAgentSummary(input: unknown): AgentSummary {
  const value = object(input);
  return {
    agentId: stringField(value, "agentId"),
    name: stringField(value, "name"),
    role: stringField(value, "role"),
    model: stringField(value, "model"),
    stateful: booleanField(value, "stateful"),
    entrypoint: booleanField(value, "entrypoint"),
    tools: value.tools === undefined ? undefined : stringArray(value.tools),
    toolkits: value.toolkits === undefined ? undefined : stringArray(value.toolkits),
    routes: value.routes === undefined ? undefined : stringArray(value.routes),
  };
}

function decodeThreadsResponse(input: unknown): readonly ThreadSummary[] {
  const value = object(input);
  return arrayOf(value.threads, decodeThreadSummary);
}

function decodeThreadResponse(input: unknown): ThreadDetail {
  const value = object(input);
  return decodeThreadSummary(value.thread);
}

function decodeThreadSummary(input: unknown): ThreadDetail {
  const value = object(input);
  return {
    id: stringField(value, "threadId"),
    title: optionalString(value.title) ?? null,
    updatedAt: stringField(value, "updatedAt"),
  };
}

function decodeMessagesResponse(input: unknown): readonly ThreadMessage[] {
  const value = object(input);
  return arrayOf(value.messages, (item) => {
    const message = object(item);
    return {
      messageId: stringField(message, "messageId"),
      threadId: stringField(message, "threadId"),
      role: stringField(message, "role"),
      content: stringField(message, "content"),
      metadata: optionalRecord(message.metadata) ?? {},
      createdAt: stringField(message, "createdAt"),
    };
  });
}

function decodeDeleteThreadResponse(_input: unknown): void {
  return undefined;
}

function decodeApprovalSummary(input: unknown): ApprovalSummary {
  const value = object(input);
  return {
    approvalId: stringField(value, "approvalId"),
    status: stringField(value, "status"),
    kind: optionalString(value.kind),
    agentId: optionalString(value.agentId),
    runId: optionalString(value.runId),
    title: optionalString(value.title),
    createdAt: optionalString(value.createdAt),
    payload: optionalRecord(value.payload),
  };
}

function decodeDataSourcesResponse(input: unknown): readonly DataSourceSummary[] {
  const value = object(input);
  return arrayOf(value.sources, (item) => {
    const source = object(item);
    return {
      sourceId: optionalString(source.sourceId) ?? stringField(source, "id"),
      name: stringField(source, "name"),
      kind: optionalString(source.kind),
      status: optionalString(source.status),
      metadata: {
        ...(optionalRecord(source.metadata) ?? {}),
        databaseType: optionalString(source.databaseType),
        host: optionalString(source.host),
        port: optionalNumber(source.port),
        databaseName: optionalString(source.databaseName),
        schemaName: optionalString(source.schemaName),
        readOnly: true,
      },
    };
  });
}

function decodeTablesResponse(input: unknown): readonly TableSummary[] {
  const value = object(input);
  return arrayOf(value.tables, decodeTableSummary);
}

function decodeSchemasResponse(input: unknown): readonly string[] {
  const value = object(input);
  return stringArray(value.schemas);
}

function decodeTableSummary(input: unknown): TableSummary {
  const table = object(input);
  return {
    tableName: stringField(table, "tableName"),
    schemaName: optionalString(table.schemaName),
    columnCount: optionalNumber(table.columnCount),
    metadata: {
      tableType: optionalString(table.tableType),
      rowCount: optionalNumber(table.rowCount),
      description: table.description ?? null,
    },
  };
}

function decodeTableDetailResponse(input: unknown): TableDetail {
  const value = object(input);
  const table = object(value.table);
  return {
    ...decodeTableSummary(table),
    columns: Array.isArray(table.columns) ? table.columns : [],
    sampleRows: Array.isArray(table.sampleRows)
      ? (table.sampleRows.filter(isObject) as Record<string, unknown>[])
      : undefined,
  };
}

function decodeTableColumnsResponse(input: unknown): readonly unknown[] {
  const value = object(input);
  return Array.isArray(value.columns) ? value.columns : [];
}

function decodeSampleRowsResponse(input: unknown): SampleRowsResult {
  const value = object(input);
  return {
    rows: Array.isArray(value.rows)
      ? (value.rows.filter(isObject) as Record<string, unknown>[])
      : [],
    metadata: optionalRecord(value.metadata),
  };
}

function decodeDocumentsResponse(input: unknown): readonly DocumentSummary[] {
  const value = object(input);
  return arrayOf(value.documents, (item) => {
    const document = object(item);
    return {
      documentId: stringField(document, "id"),
      title: stringField(document, "name"),
      sourceId: optionalString(document.targetDatabaseId),
      metadata: {
        extractionStatus: optionalString(document.extractionStatus),
        extractedKnowledgeIds: Array.isArray(document.extractedKnowledgeIds)
          ? document.extractedKnowledgeIds
          : [],
        createdAt: optionalString(document.createdAt),
      },
    };
  });
}

function decodeKnowledgeItemsResponse(input: unknown): readonly KnowledgeItem[] {
  const value = object(input);
  return arrayOf(value.items, (item) => {
    const knowledge = object(item);
    return {
      itemId: stringField(knowledge, "id"),
      title: optionalString(knowledge.name),
      content: optionalString(knowledge.description),
      metadata: {
        knowledgeType: optionalString(knowledge.knowledgeType),
        scopeTables: Array.isArray(knowledge.scopeTables) ? knowledge.scopeTables : [],
        scopeColumns: Array.isArray(knowledge.scopeColumns) ? knowledge.scopeColumns : [],
        sqlExpression: knowledge.sqlExpression ?? null,
        synonyms: Array.isArray(knowledge.synonyms) ? knowledge.synonyms : [],
        sourceDocumentId: knowledge.sourceDocumentId ?? null,
      },
    };
  });
}

function decodeProfileEstimateResponse(input: unknown): ProfileEstimateResult {
  const value = object(input);
  return {
    estimate: value.estimate,
    metadata: optionalRecord(value.metadata),
  };
}

function decodeToolsResponse(input: unknown): readonly ToolSummary[] {
  const value = object(input);
  return arrayOf(value.tools, (item) => {
    const tool = object(item);
    const inputSchema = optionalRecord(tool.inputSchema) ?? optionalRecord(tool.parameters);
    return {
      toolId: optionalString(tool.toolId) ?? stringField(tool, "id"),
      name: stringField(tool, "name"),
      description: normalizeToolDescription(optionalString(tool.description)),
      origin: "toolkit",
      toolkit: optionalString(tool.toolkit) ?? "catalog",
      inputSchema,
    };
  });
}

function normalizeToolDescription(description: string | undefined): string | undefined {
  if (description === undefined) return undefined;

  const raw = description.trim();
  if (raw.length === 0) return "";

  const hasBlockCommentMarkers = /^\s*\/\*+/.test(raw) || /\*\/\s*$/.test(raw);
  let cleaned = raw.replace(/^\s*\/\*+\s*/, "").replace(/\s*\*\/\s*$/, "");

  if (hasBlockCommentMarkers && !cleaned.includes("\n")) {
    cleaned = cleaned.replace(/\s+\*\s+/g, "\n");
  }

  return cleaned
    .split(/\r?\n/)
    .map((line) => line.replace(/^\s*(?:\*|\/\/\/)\s?/, "").trimEnd())
    .join("\n")
    .trim();
}

function decodeToolExecutionResponse(input: unknown): ToolExecutionResult {
  const value = object(input);
  const result = object(value.result);
  const success = result.success === undefined ? true : booleanField(result, "success");
  return {
    toolId: stringField(result, "toolId"),
    status: success ? "success" : "error",
    output: result.data ?? result,
  };
}

function decodeMetricsResponse(input: unknown): readonly MetricSummary[] {
  const value = object(input);
  return arrayOf(value.metrics, (item) => {
    const metric = object(item);
    return {
      metricId: optionalString(metric.metricId) ?? stringField(metric, "id"),
      name: stringField(metric, "name"),
      description: optionalString(metric.description),
      metricType: optionalString(metric.metricType),
      tags: Array.isArray(metric.tags) ? stringArray(metric.tags) : undefined,
      metadata: optionalRecord(metric.metadata),
    };
  });
}

function decodeCatalogSearchResponse(input: unknown): CatalogSearchResult {
  const value = object(input);
  const search = isObject(value.search) ? value.search : value;
  return {
    items: Array.isArray(search.items) ? search.items : [],
    metadata: {
      totalCount: optionalNumber(search.totalCount),
      queryTimeMs: optionalNumber(search.queryTimeMs),
      mode: optionalString(search.mode),
    },
  };
}

function decodeSearchToolResponse(input: unknown): SearchToolResult {
  const value = object(input);
  const result = isObject(value.result) ? value.result : value;
  const data = isObject(result.data) ? result.data : {};
  const rows = Array.isArray(data.results)
    ? data.results
    : Array.isArray(data.rows)
      ? data.rows
      : Array.isArray(data.matches)
        ? data.matches
        : [];
  return {
    rows,
    metadata: optionalRecord(result),
  };
}

function searchToolIdForMode(mode: string): string {
  void mode;
  return "search_catalog";
}

function searchToolInputForMode(input: {
  mode: string;
  query: string;
  limit?: number;
  filters?: Record<string, unknown>;
}) {
  const kinds = searchKindsForMode(input.mode);
  return {
    query: input.query,
    ...(input.limit !== undefined ? { limit: input.limit } : {}),
    ...(input.filters ? { filters: input.filters } : {}),
    ...(kinds.length > 0 ? { kinds } : {}),
  };
}

function searchKindsForMode(mode: string): string[] {
  switch (mode) {
    case "columns":
    case "column":
      return ["column"];
    case "metrics":
    case "metric":
      return ["metric"];
    case "enums":
    case "enum":
      return ["enum_value"];
    case "tables":
    case "table":
      return ["table"];
    default:
      return [];
  }
}

function object(input: unknown): Record<string, unknown> {
  if (!isObject(input)) {
    throw new Error("Expected object response.");
  }
  return input;
}

function isObject(input: unknown): input is Record<string, unknown> {
  return typeof input === "object" && input !== null && !Array.isArray(input);
}

function stringField(input: Record<string, unknown>, field: string): string {
  const value = input[field];
  if (typeof value !== "string") {
    throw new Error(`Expected string field ${field}.`);
  }
  return value;
}

function booleanField(input: Record<string, unknown>, field: string): boolean {
  const value = input[field];
  if (typeof value !== "boolean") {
    throw new Error(`Expected boolean field ${field}.`);
  }
  return value;
}

function optionalString(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function optionalBoolean(value: unknown): boolean | undefined {
  return typeof value === "boolean" ? value : undefined;
}

function optionalNumber(value: unknown): number | undefined {
  return typeof value === "number" ? value : undefined;
}

function optionalRecord(value: unknown): Record<string, unknown> | undefined {
  return isObject(value) ? value : undefined;
}

function optionalArray(value: unknown): readonly unknown[] | undefined {
  return Array.isArray(value) ? value : undefined;
}

function stringArray(value: unknown): readonly string[] {
  if (!Array.isArray(value) || value.some((item) => typeof item !== "string")) {
    throw new Error("Expected string array.");
  }
  return value;
}

function arrayOf<T>(value: unknown, decode: Decoder<T>): readonly T[] {
  if (!Array.isArray(value)) {
    throw new Error("Expected array.");
  }
  return value.map(decode);
}
