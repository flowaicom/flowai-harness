import { describe, expect, test } from "bun:test";
import type { RunEventEnvelope, RunSummary } from "~/lib/api/runs";
import {
  isEffectivelyTerminalRun,
  isStaleRunningRun,
  latestChatRunForThread,
  projectRunTimeline,
} from "./run-events";

function event(seq: number, kind: string, payload: Record<string, unknown>): RunEventEnvelope {
  return {
    seq,
    kind,
    event: {
      schemaVersion: "harness-studio/v1",
      workspaceKey: "default",
      runId: "run-1",
      threadId: "thread-1",
      agentId: "agent",
      seq,
      kind,
      payload,
    },
    raw: {},
    createdAt: "2026-01-01T00:00:00Z",
  };
}

describe("projectRunTimeline", () => {
  test("pairs tool started/completed by toolCallId", () => {
    const items = projectRunTimeline([
      event(1, "tool.call.started", {
        toolCallId: "tool-1",
        toolName: "execute_query",
        arguments: { query: "select 1" },
      }),
      event(2, "tool.call.completed", {
        toolCallId: "tool-1",
        toolName: "execute_query",
        result: { rows: [[1]] },
      }),
    ]);

    expect(items).toHaveLength(1);
    expect(items[0]?.label).toBe("execute_query");
    expect(items[0]?.status).toBe("completed");
    expect(items[0]?.input).toEqual({ query: "select 1" });
    expect(items[0]?.output).toEqual({ rows: [[1]] });
  });

  test("pairs sub-agent started/completed by toolCallId", () => {
    const items = projectRunTimeline([
      event(3, "sub_agent.call.started", {
        toolCallId: "agent-1",
        targetAgentId: "data_analyst",
        message: "Find revenue",
      }),
      event(4, "sub_agent.call.completed", {
        toolCallId: "agent-1",
        targetAgentId: "data_analyst",
        result: { response: "Sparkling Water" },
      }),
    ]);

    expect(items).toHaveLength(1);
    expect(items[0]?.kind).toBe("subAgent");
    expect(items[0]?.label).toBe("data_analyst");
    expect(items[0]?.input).toBe("Find revenue");
    expect(items[0]?.output).toEqual({ response: "Sparkling Water" });
  });

  test("marks a missing tool completion as pending", () => {
    const items = projectRunTimeline([
      event(1, "tool.call.started", {
        toolCallId: "tool-1",
        toolName: "search_catalog",
        arguments: { query: "products" },
      }),
    ]);

    expect(items).toHaveLength(1);
    expect(items[0]?.status).toBe("pending");
  });

  test("renders unknown event kinds generically", () => {
    const items = projectRunTimeline([event(9, "custom.unmapped", { value: 1 })]);

    expect(items).toHaveLength(1);
    expect(items[0]?.kind).toBe("unknown");
    expect(items[0]?.label).toBe("custom.unmapped");
    expect(items[0]?.rawEvents).toHaveLength(1);
  });
});

describe("latestChatRunForThread", () => {
  test("selects the newest chat run for the active thread", () => {
    const base = {
      agentId: "agent",
      firstSeq: 1,
      lastSeq: 1,
      eventCount: 1,
      createdAt: "2026-01-01T00:00:00Z",
    };
    const runs: RunSummary[] = [
      {
        ...base,
        runId: "old",
        operation: "chat",
        threadId: "thread-1",
        status: "completed",
        updatedAt: "2026-01-01T00:00:00Z",
      },
      {
        ...base,
        runId: "new",
        operation: "chat",
        threadId: "thread-1",
        status: "completed",
        updatedAt: "2026-01-02T00:00:00Z",
      },
      {
        ...base,
        runId: "eval",
        operation: "eval",
        threadId: "thread-1",
        status: "completed",
        updatedAt: "2026-01-03T00:00:00Z",
      },
    ];

    expect(latestChatRunForThread(runs, "thread-1")?.runId).toBe("new");
  });
});

describe("running run display state", () => {
  const base: RunSummary = {
    runId: "run-1",
    operation: "chat",
    threadId: "thread-1",
    agentId: "agent",
    status: "running",
    firstSeq: 1,
    lastSeq: 1,
    eventCount: 1,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
  };

  test("treats old running rows as stale/interrupted", () => {
    const now = Date.parse("2026-01-01T00:10:01Z");

    expect(isStaleRunningRun(base, now)).toBe(true);
    expect(isEffectivelyTerminalRun(base, now)).toBe(true);
  });

  test("does not mark fresh running rows as stale", () => {
    const now = Date.parse("2026-01-01T00:01:00Z");

    expect(isStaleRunningRun(base, now)).toBe(false);
    expect(isEffectivelyTerminalRun(base, now)).toBe(false);
  });
});
