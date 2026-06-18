import { describe, expect, test } from "bun:test";

import {
  accumulatePart,
  createAccumulator,
  finalizeAccumulator,
  groupParts,
  MessageId,
} from "./message";

describe("groupParts", () => {
  test("separates sub-agent delegation from ordinary tool groups", () => {
    const grouped = groupParts([
      {
        type: "tool-invocation",
        toolCallId: "query-1",
        toolName: "execute_query",
        args: {},
        state: "result",
        result: { rows: [] },
      },
      {
        type: "tool-agent",
        toolCallId: "agent-1",
        agentName: "data_analyst",
        state: "result",
      },
      {
        type: "tool-invocation",
        toolCallId: "call-agent-1",
        toolName: "call_agent",
        args: { agent: "data_analyst", prompt: "List products" },
        state: "result",
        result: { response: "Done" },
      },
      {
        type: "tool-invocation",
        toolCallId: "catalog-1",
        toolName: "search_catalog",
        args: {},
        state: "result",
        result: { results: [] },
      },
    ]);

    expect(grouped).toHaveLength(2);
    expect(grouped[0]).toMatchObject({
      type: "sub-agent-invocation",
      agentName: "data_analyst",
      state: "result",
    });
    expect(grouped[1]).toMatchObject({
      type: "tool-group",
      parts: [
        { type: "tool-invocation", toolName: "execute_query" },
        { type: "tool-invocation", toolName: "search_catalog" },
      ],
    });
  });

  test("uses call_agent arguments as a fallback sub-agent title", () => {
    const grouped = groupParts([
      {
        type: "tool-invocation",
        toolCallId: "call-agent-1",
        toolName: "call_agent",
        args: { agent: "planner", prompt: "Build a plan" },
        state: "result",
        result: { response: "Plan stored" },
      },
    ]);

    expect(grouped).toEqual([
      {
        type: "sub-agent-invocation",
        toolCallId: "call-agent-1",
        agentName: "planner",
        state: "result",
        parts: [
          {
            type: "tool-invocation",
            toolCallId: "call-agent-1",
            toolName: "call_agent",
            args: { agent: "planner", prompt: "Build a plan" },
            state: "result",
            result: { response: "Plan stored" },
          },
        ],
      },
    ]);
  });
});

describe("finalizeAccumulator", () => {
  test("marks pending sub-agent calls as cancelled when requested", () => {
    const acc = accumulatePart(createAccumulator(MessageId("msg-1")), {
      type: "tool-agent",
      toolInvocationId: "agent-call-1",
      agentName: "data_analyst",
      state: "call",
    });

    const message = finalizeAccumulator(acc, "2026-01-01T00:00:00Z", {
      pendingState: "cancelled",
    });

    expect(message.parts).toContainEqual({
      type: "tool-agent",
      toolCallId: "agent-call-1",
      agentName: "data_analyst",
      state: "cancelled",
    });
  });
});
