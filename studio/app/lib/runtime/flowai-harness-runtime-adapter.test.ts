import { afterEach, describe, expect, mock, test } from "bun:test";
import { createWorkspaceScope } from "@studio/core/domain/scope";
import type { StudioEvent } from "@studio/core/runtime";
import { getApiConfig, setApiConfig } from "~/lib/api/client";
import { createFlowAIHarnessRuntimeAdapter } from "./flowai-harness-runtime-adapter";

const originalFetch = globalThis.fetch;
const originalConfig = getApiConfig();

afterEach(() => {
  mock.restore();
  globalThis.fetch = originalFetch;
  setApiConfig(originalConfig);
});

function studioEvent(seq: number, kind: string, payload: unknown = {}): StudioEvent {
  return {
    schemaVersion: "harness-studio/v1",
    workspaceKey: "default",
    runId: "run-1",
    threadId: "thread-1",
    agentId: "agent-1",
    seq,
    kind,
    payload,
  };
}

function sseBlock(event: StudioEvent, eventId = String(event.seq)): string {
  return `id: ${eventId}\nevent: ${event.kind}\ndata: ${JSON.stringify(event)}\n\n`;
}

function responseStream(blocks: readonly string[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  return new ReadableStream<Uint8Array>({
    start(controller) {
      for (const block of blocks) {
        controller.enqueue(encoder.encode(block));
      }
      controller.close();
    },
  });
}

function delayedSecondBlockStream(
  first: string,
  second: string,
  delayMs: number
): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  return new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(encoder.encode(first));
      setTimeout(() => {
        try {
          controller.enqueue(encoder.encode(second));
          controller.close();
        } catch {
          // The adapter may cancel before the delayed block is written.
        }
      }, delayMs);
    },
  });
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

describe("FlowAIHarnessRuntimeAdapter", () => {
  test("builds encoded workspace-scoped paths from AppScope", async () => {
    const adapter = createFlowAIHarnessRuntimeAdapter();
    const calls: string[] = [];
    globalThis.fetch = mock(async (input: RequestInfo | URL) => {
      calls.push(String(input));
      return new Response(
        JSON.stringify({
          workspaceKey: "customer one",
          agents: [],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } }
      );
    }) as unknown as typeof fetch;

    const result = await adapter.listAgents(createWorkspaceScope("customer one"));

    expect(result._tag).toBe("Ok");
    expect(calls).toEqual(["/api/workspaces/customer%20one/agents"]);
  });

  test("normalizes Studio error bodies into typed ApiError details", async () => {
    const adapter = createFlowAIHarnessRuntimeAdapter();
    globalThis.fetch = mock(async () => {
      return new Response(
        JSON.stringify({
          error: {
            code: "workspace.not_found",
            message: "Workspace `missing` was not found.",
            retryable: false,
            details: { workspaceKey: "missing" },
          },
        }),
        { status: 404, headers: { "Content-Type": "application/json" } }
      );
    }) as unknown as typeof fetch;

    const result = await adapter.getWorkspace(createWorkspaceScope("missing"));

    expect(result._tag).toBe("Err");
    if (result._tag === "Err") {
      expect(result.error.code).toBe("NOT_FOUND");
      expect(result.error.message).toBe("Workspace `missing` was not found.");
      expect(result.error.details).toEqual({
        workspaceKey: "missing",
        serverCode: "workspace.not_found",
      });
    }
  });

  test("wires catalog tools through workspace-scoped tools paths", async () => {
    const adapter = createFlowAIHarnessRuntimeAdapter();
    const calls: Array<{ url: string; body?: unknown }> = [];
    globalThis.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      calls.push({
        url: String(input),
        body: init?.body ? JSON.parse(String(init.body)) : undefined,
      });
      if (String(input).endsWith("/tools")) {
        return new Response(
          JSON.stringify({
            tools: [
              {
                id: "search_catalog",
                toolId: "search_catalog",
                name: "search catalog",
                description: "/** * Find tables. * Use this when: * - You need catalog context. */",
                inputSchema: { type: "object" },
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } }
        );
      }
      return new Response(
        JSON.stringify({
          result: {
            toolId: "search_catalog",
            success: true,
            data: { results: [] },
          },
        }),
        { status: 200, headers: { "Content-Type": "application/json" } }
      );
    }) as unknown as typeof fetch;

    const scope = createWorkspaceScope("default");
    const tools = await adapter.listTools(scope);
    const execution = await adapter.executeTool(scope, {
      toolId: "search_catalog",
      input: { query: "orders" },
    });

    expect(tools._tag).toBe("Ok");
    if (tools._tag === "Ok") {
      expect(tools.value[0]?.toolkit).toBe("catalog");
      expect(tools.value[0]?.description).toBe(
        "Find tables.\nUse this when:\n- You need catalog context."
      );
    }
    expect(execution._tag).toBe("Ok");
    expect(calls).toEqual([
      { url: "/api/workspaces/default/tools", body: undefined },
      {
        url: "/api/workspaces/default/tools/search_catalog/execute",
        body: { input: { query: "orders" } },
      },
    ]);
  });

  test("starts chat streams with prompt and threadId, not message history", async () => {
    const adapter = createFlowAIHarnessRuntimeAdapter();
    const calls: Array<{ url: string; body: unknown }> = [];
    const events: StudioEvent[] = [];
    const eventIds: string[] = [];
    const completed = new Promise<void>((resolve, reject) => {
      globalThis.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({
          url: String(input),
          body: JSON.parse(String(init?.body)),
        });
        return new Response(
          responseStream([
            sseBlock(studioEvent(1, "message.delta", { text: "hi" }), "evt-1"),
            sseBlock(studioEvent(2, "run.completed"), "evt-2"),
          ]),
          { status: 200, headers: { "Content-Type": "text/event-stream" } }
        );
      }) as unknown as typeof fetch;

      void adapter.startChatStream(
        {
          agentId: "agent/1",
          prompt: "hello",
          threadId: "thread-1",
          runId: "run-client",
          handlers: {
            onEvent: (event) => events.push(event),
            onEventId: (eventId) => eventIds.push(eventId),
            onComplete: resolve,
            onError: reject,
          },
        },
        { scope: createWorkspaceScope("default"), signal: new AbortController().signal }
      );
    });

    await completed;

    expect(calls).toEqual([
      {
        url: "/api/workspaces/default/agents/agent%2F1/stream",
        body: { prompt: "hello", threadId: "thread-1", runId: "run-client" },
      },
    ]);
    expect("messages" in (calls[0]?.body as Record<string, unknown>)).toBe(false);
    expect(events.map((event) => event.kind)).toEqual(["message.delta", "run.completed"]);
    expect(eventIds).toEqual(["evt-1", "evt-2"]);
  });

  test("uses workspace-scoped thread and message paths instead of legacy thread endpoints", async () => {
    const adapter = createFlowAIHarnessRuntimeAdapter();
    const calls: string[] = [];
    globalThis.fetch = mock(async (input: RequestInfo | URL) => {
      calls.push(String(input));
      if (String(input).endsWith("/messages")) {
        return new Response(
          JSON.stringify({
            workspaceKey: "customer-a",
            threadId: "thread-1",
            messages: [
              {
                messageId: "1",
                threadId: "thread-1",
                role: "user",
                content: "hello",
                metadata: {},
                createdAt: "2026-01-01T00:00:00Z",
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } }
        );
      }
      return new Response(
        JSON.stringify({
          workspaceKey: "customer-a",
          threads: [
            {
              threadId: "thread-1",
              title: "Thread",
              updatedAt: "2026-01-01T00:00:00Z",
            },
          ],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } }
      );
    }) as unknown as typeof fetch;

    const scope = createWorkspaceScope("customer-a");
    const threads = await adapter.listThreads(scope);
    const messages = await adapter.listThreadMessages(scope, "thread-1");

    expect(threads._tag).toBe("Ok");
    expect(messages._tag).toBe("Ok");
    expect(calls).toEqual([
      "/api/workspaces/customer-a/threads",
      "/api/workspaces/customer-a/threads/thread-1/messages",
    ]);
    expect(calls.some((url) => url === "/api/threads" || url.includes("/api/threads/"))).toBe(
      false
    );
  });

  test("deletes chat threads through workspace-scoped thread paths", async () => {
    const adapter = createFlowAIHarnessRuntimeAdapter();
    const calls: Array<{ method: string; url: string }> = [];
    globalThis.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      calls.push({ method: init?.method ?? "GET", url: String(input) });
      return new Response("", { status: 204 });
    }) as unknown as typeof fetch;

    const result = await adapter.deleteThread(createWorkspaceScope("customer-a"), "thread-1");

    expect(result._tag).toBe("Ok");
    expect(calls).toEqual([
      {
        method: "DELETE",
        url: "/api/workspaces/customer-a/threads/thread-1",
      },
    ]);
  });

  test("wires Connect discovery, knowledge, search, and metrics through workspace-scoped data paths", async () => {
    const adapter = createFlowAIHarnessRuntimeAdapter();
    const calls: string[] = [];
    globalThis.fetch = mock(async (input: RequestInfo | URL) => {
      const url = String(input);
      calls.push(url);
      if (url.endsWith("/data/sources")) {
        return new Response(
          JSON.stringify({
            sources: [
              {
                id: "workspace-runtime",
                sourceId: "workspace-runtime",
                name: "Workspace runtime",
                kind: "workspace-runtime",
                status: "ready",
                metadata: { readOnly: true },
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } }
        );
      }
      if (url.includes("/data/discovery/tables/orders")) {
        return new Response(
          JSON.stringify({
            table: {
              schemaName: "main",
              tableName: "orders",
              tableType: "base_table",
              rowCount: 5,
              columnCount: 2,
              description: null,
              columns: [{ columnName: "id" }],
            },
          }),
          { status: 200, headers: { "Content-Type": "application/json" } }
        );
      }
      if (url.includes("/data/discovery/tables")) {
        return new Response(
          JSON.stringify({
            tables: [
              {
                schemaName: "main",
                tableName: "orders",
                tableType: "base_table",
                rowCount: 5,
                columnCount: 2,
                description: null,
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } }
        );
      }
      if (url.endsWith("/data/knowledge/documents")) {
        return new Response(
          JSON.stringify({
            documents: [
              {
                id: "doc-1",
                name: "Rules",
                targetDatabaseId: "workspace-runtime",
                extractionStatus: "processed",
                extractedKnowledgeIds: [],
                createdAt: "2026-01-01T00:00:00Z",
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } }
        );
      }
      if (url.endsWith("/data/search")) {
        return new Response(
          JSON.stringify({
            search: {
              items: [],
              totalCount: 0,
              queryTimeMs: 1,
            },
          }),
          { status: 200, headers: { "Content-Type": "application/json" } }
        );
      }
      if (url.endsWith("/data/metrics")) {
        return new Response(
          JSON.stringify({
            metrics: [
              {
                id: "metric-1",
                name: "Revenue",
                description: "Revenue",
                metricType: "sum",
                tags: [],
                metadata: {},
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } }
        );
      }
      return new Response(
        JSON.stringify({
          items: [
            {
              id: "knowledge-1",
              name: "Revenue",
              description: "Revenue rule",
              knowledgeType: "business_rule",
              scopeTables: ["orders"],
              scopeColumns: [],
              sqlExpression: null,
              synonyms: [],
              sourceDocumentId: "doc-1",
            },
          ],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } }
      );
    }) as unknown as typeof fetch;

    const scope = createWorkspaceScope("customer-a");
    const sources = await adapter.listDataSources(scope);
    const tables = await adapter.listTables(scope, { schema: "main" });
    const table = await adapter.getTableDetail(scope, { tableName: "orders", schema: "main" });
    const documents = await adapter.listDocuments(scope);
    const knowledge = await adapter.browseKnowledge(scope);
    const search = await adapter.searchCatalog(scope, { query: "orders" });
    const metrics = await adapter.listMetrics(scope);

    expect(sources._tag).toBe("Ok");
    expect(tables._tag).toBe("Ok");
    expect(table._tag).toBe("Ok");
    expect(documents._tag).toBe("Ok");
    expect(knowledge._tag).toBe("Ok");
    expect(search._tag).toBe("Ok");
    expect(metrics._tag).toBe("Ok");
    expect(calls).toEqual([
      "/api/workspaces/customer-a/data/sources",
      "/api/workspaces/customer-a/data/discovery/tables?schema=main",
      "/api/workspaces/customer-a/data/discovery/tables/orders?schema=main",
      "/api/workspaces/customer-a/data/knowledge/documents",
      "/api/workspaces/customer-a/data/knowledge/items",
      "/api/workspaces/customer-a/data/search",
      "/api/workspaces/customer-a/data/metrics",
    ]);
  });

  test("surfaces non-increasing StudioEvent seq as a stream protocol error", async () => {
    const adapter = createFlowAIHarnessRuntimeAdapter();
    const errors: string[] = [];
    const failed = new Promise<void>((resolve) => {
      globalThis.fetch = mock(async () => {
        return new Response(
          responseStream([
            sseBlock(studioEvent(1, "message.delta")),
            sseBlock(studioEvent(1, "message.delta")),
          ]),
          { status: 200, headers: { "Content-Type": "text/event-stream" } }
        );
      }) as unknown as typeof fetch;

      void adapter.startChatStream(
        {
          agentId: "agent-1",
          prompt: "hello",
          threadId: "thread-1",
          handlers: {
            onEvent: () => {},
            onComplete: () => {},
            onError: (error) => {
              errors.push(error.code);
              resolve();
            },
          },
        },
        { scope: createWorkspaceScope("default"), signal: new AbortController().signal }
      );
    });

    await failed;

    expect(errors).toEqual(["STREAM_PROTOCOL_ERROR"]);
  });

  test("abort prevents post-abort Studio events from being delivered", async () => {
    const adapter = createFlowAIHarnessRuntimeAdapter();
    const abortController = new AbortController();
    const events: string[] = [];
    globalThis.fetch = mock(async () => {
      return new Response(
        delayedSecondBlockStream(
          sseBlock(studioEvent(1, "message.delta", { text: "first" })),
          sseBlock(studioEvent(2, "message.delta", { text: "second" })),
          25
        ),
        { status: 200, headers: { "Content-Type": "text/event-stream" } }
      );
    }) as unknown as typeof fetch;

    const result = await adapter.startChatStream(
      {
        agentId: "agent-1",
        prompt: "hello",
        threadId: "thread-1",
        handlers: {
          onEvent: (event) => {
            events.push(String((event.payload as { text?: string }).text));
            abortController.abort();
          },
          onComplete: () => {},
          onError: () => {},
        },
      },
      { scope: createWorkspaceScope("default"), signal: abortController.signal }
    );

    expect(result._tag).toBe("Ok");
    await sleep(60);
    expect(events).toEqual(["first"]);
  });

  test("abort posts run cancellation before aborting the chat stream", async () => {
    const adapter = createFlowAIHarnessRuntimeAdapter();
    const calls: Array<{ url: string; method: string; body: unknown }> = [];
    globalThis.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      calls.push({
        url: String(input),
        method: String(init?.method ?? "GET"),
        body: init?.body ? JSON.parse(String(init.body)) : null,
      });
      if (String(input).endsWith("/stream")) {
        return new Response(new ReadableStream<Uint8Array>(), {
          status: 200,
          headers: { "Content-Type": "text/event-stream" },
        });
      }
      return new Response(
        JSON.stringify({
          workspaceKey: "default",
          runId: "run-client",
          status: "cancelled",
          cancelled: true,
        }),
        { status: 200, headers: { "Content-Type": "application/json" } }
      );
    }) as unknown as typeof fetch;

    const result = await adapter.startChatStream(
      {
        agentId: "agent-1",
        prompt: "hello",
        threadId: "thread-1",
        runId: "run-client",
      },
      { scope: createWorkspaceScope("default"), signal: new AbortController().signal }
    );

    expect(result._tag).toBe("Ok");
    if (result._tag === "Ok") {
      result.value.abort();
    }
    await sleep(0);

    expect(calls).toEqual([
      {
        url: "/api/workspaces/default/agents/agent-1/stream",
        method: "POST",
        body: { prompt: "hello", threadId: "thread-1", runId: "run-client" },
      },
      {
        url: "/api/workspaces/default/runs/run-client/cancel",
        method: "POST",
        body: null,
      },
    ]);
  });
});
