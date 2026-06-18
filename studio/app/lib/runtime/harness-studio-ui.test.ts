import { describe, expect, test } from "bun:test";
import type { AgentSummary, StudioEvent, ThreadMessage } from "@studio/core/runtime";
import {
  createLocalThread,
  pickChatAgentId,
  studioEventToStreamPart,
  threadMessageToUiMessage,
  threadSummaryToThread,
  workspaceSummaryToWorkspace,
} from "./harness-studio-ui";

describe("harness Studio UI adapters", () => {
  test("maps harness workspace summaries to existing workspace store shape", () => {
    const workspace = workspaceSummaryToWorkspace({
      workspaceKey: "acme",
      displayName: "ACME",
      metadata: { createdAt: "2026-01-01T00:00:00Z" },
    });

    expect(workspace.id).toBe("acme");
    expect(workspace.displayName).toBe("ACME");
    expect(workspace.databases).toEqual([]);
    expect(workspace.bundle?.complete).toBe(true);
  });

  test("maps harness thread summaries and messages to existing chat store shape", () => {
    expect(
      threadSummaryToThread(
        { id: "thread-1", title: "Hello", updatedAt: "2026-01-02T00:00:00Z" },
        "workspace-a"
      )
    ).toEqual({
      id: "thread-1",
      title: "Hello",
      resourceId: "workspace-a",
      createdAt: "2026-01-02T00:00:00Z",
      updatedAt: "2026-01-02T00:00:00Z",
    });

    const message: ThreadMessage = {
      messageId: "1",
      threadId: "thread-1",
      role: "assistant",
      content: "Hi",
      metadata: { runId: "run-1" },
      createdAt: "2026-01-02T00:00:01Z",
    };
    expect(threadMessageToUiMessage(message)).toEqual({
      id: "1",
      role: "assistant",
      parts: [{ type: "text", text: "Hi" }],
      createdAt: "2026-01-02T00:00:01Z",
      metadata: { runId: "run-1", threadId: "thread-1" },
    });
  });

  test("prefers structured persisted message parts over flat content", () => {
    const message: ThreadMessage = {
      messageId: "2",
      threadId: "thread-1",
      role: "assistant",
      content: "Flat fallback text",
      metadata: {
        runId: "run-2",
        parts: [
          { type: "text", text: "Structured text" },
          {
            type: "tool-invocation",
            toolCallId: "tool-1",
            toolName: "execute_query",
            args: {},
            state: "result",
            result: { rowCount: 1 },
          },
        ],
      },
      createdAt: "2026-01-02T00:00:02Z",
    };

    expect(threadMessageToUiMessage(message)).toEqual({
      id: "2",
      role: "assistant",
      parts: [
        { type: "text", text: "Structured text" },
        {
          type: "tool-invocation",
          toolCallId: "tool-1",
          toolName: "execute_query",
          args: {},
          state: "result",
          result: { rowCount: 1 },
        },
      ],
      createdAt: "2026-01-02T00:00:02Z",
      metadata: {
        runId: "run-2",
        parts: [
          { type: "text", text: "Structured text" },
          {
            type: "tool-invocation",
            toolCallId: "tool-1",
            toolName: "execute_query",
            args: {},
            state: "result",
            result: { rowCount: 1 },
          },
        ],
        threadId: "thread-1",
      },
    });
  });

  test("deduplicates persisted lifecycle parts by tool call id", () => {
    const message: ThreadMessage = {
      messageId: "2b",
      threadId: "thread-1",
      role: "assistant",
      content: "Flat fallback text",
      metadata: {
        runId: "run-2b",
        parts: [
          { type: "text", text: "Searching" },
          {
            type: "tool-invocation",
            toolCallId: "tool-1",
            toolName: "search_catalog",
            args: { query: "products" },
            state: "result",
            result: { count: 1 },
          },
          {
            type: "tool-invocation",
            toolCallId: "tool-1",
            toolName: "search_catalog",
            args: {},
            state: "result",
            result: { count: 1 },
          },
        ],
      },
      createdAt: "2026-01-02T00:00:02Z",
    };

    expect(threadMessageToUiMessage(message).parts).toEqual([
      { type: "text", text: "Searching" },
      {
        type: "tool-invocation",
        toolCallId: "tool-1",
        toolName: "search_catalog",
        args: { query: "products" },
        state: "result",
        result: { count: 1 },
      },
    ]);
  });

  test("creates local threads without requiring a server create endpoint", () => {
    const thread = createLocalThread("thread-local", "workspace-a");

    expect(thread.id).toBe("thread-local");
    expect(thread.resourceId).toBe("workspace-a");
    expect(thread.title).toBe("New Conversation");
  });

  test("prefers entrypoint coordinator as the Studio chat agent", () => {
    const agents: AgentSummary[] = [
      agent("specialist", { entrypoint: true, role: "specialist" }),
      agent("coordinator", { entrypoint: true, role: "coordinator" }),
    ];

    expect(pickChatAgentId(agents)).toBe("coordinator");
  });

  test("projects harness Studio events into existing stream parts", () => {
    const event = studioEvent("message.delta", { text: "hello" });
    expect(studioEventToStreamPart(event)).toEqual({ type: "text", text: "hello" });

    expect(
      studioEventToStreamPart(
        studioEvent("tool.call.started", {
          toolCallId: "tool-1",
          toolName: "search",
          arguments: { query: "x" },
        })
      )
    ).toEqual({
      type: "tool-invocation",
      toolInvocationId: "tool-1",
      toolName: "search",
      args: { query: "x" },
      state: "call",
    });

    expect(
      studioEventToStreamPart(
        studioEvent("tool.call.completed", {
          toolCallId: "tool-1",
          toolName: "search",
          result: { ok: true },
        })
      )
    ).toEqual({
      type: "tool-invocation",
      toolInvocationId: "tool-1",
      toolName: "search",
      args: undefined,
      state: "result",
      result: { ok: true },
    });

    expect(
      studioEventToStreamPart(
        studioEvent("sub_agent.call.started", {
          toolCallId: "agent-call-1",
          targetAgentId: "insights",
        })
      )
    ).toEqual({
      type: "tool-agent",
      toolInvocationId: "agent-call-1",
      agentName: "insights",
      state: "call",
    });

    expect(
      studioEventToStreamPart(
        studioEvent("sub_agent.call.completed", {
          toolCallId: "agent-call-1",
          targetAgentId: "insights",
        })
      )
    ).toEqual({
      type: "tool-agent",
      toolInvocationId: "agent-call-1",
      agentName: "insights",
      state: "result",
    });

    expect(
      studioEventToStreamPart(
        studioEvent("approval.required", {
          approvalId: "approval-1",
          title: "Execute plan",
          kind: "plan",
          raw: { target: "plan-1" },
        })
      )
    ).toEqual({
      type: "custom",
      name: "approval.required",
      data: {
        approvalId: "approval-1",
        title: "Execute plan",
        kind: "plan",
        raw: { target: "plan-1" },
      },
    });

    expect(
      studioEventToStreamPart(
        studioEvent("approval.decision", {
          approvalId: "approval-1",
          status: "approve",
        })
      )
    ).toEqual({
      type: "custom",
      name: "approval.decision",
      data: {
        approvalId: "approval-1",
        status: "approve",
      },
    });

    expect(studioEventToStreamPart(studioEvent("run.completed", {}))?.type).toBe("finish");
  });
});

function agent(agentId: string, overrides: Partial<AgentSummary>): AgentSummary {
  return {
    agentId,
    name: agentId,
    role: "specialist",
    model: "mock",
    stateful: false,
    entrypoint: false,
    ...overrides,
  };
}

function studioEvent(kind: string, payload: Record<string, unknown>): StudioEvent {
  return {
    schemaVersion: "harness-studio/v1",
    workspaceKey: "default",
    runId: "run-1",
    threadId: "thread-1",
    agentId: "coordinator",
    seq: 1,
    kind,
    payload,
  };
}
