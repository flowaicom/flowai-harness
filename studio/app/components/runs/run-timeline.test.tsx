import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router";
import type { RunEventEnvelope } from "~/lib/api/runs";
import { projectRunTimeline } from "~/lib/domain/run-events";
import { RunActivityPanel, RunActivitySummaryPanel, RunTimeline } from "./run-timeline";

function event(seq: number, kind: string, payload: Record<string, unknown>): RunEventEnvelope {
  return {
    seq,
    kind,
    event: { seq, kind, payload },
    raw: {},
    createdAt: "2026-01-01T00:00:00Z",
  };
}

describe("RunTimeline", () => {
  test("renders tool input/output and sub-agent result from fixture events", () => {
    const items = projectRunTimeline([
      event(1, "tool.call.started", {
        toolCallId: "tool-1",
        toolName: "execute_query",
        arguments: { query: "select product" },
      }),
      event(2, "tool.call.completed", {
        toolCallId: "tool-1",
        toolName: "execute_query",
        result: { product: "Sparkling Water" },
      }),
      event(3, "sub_agent.call.completed", {
        toolCallId: "agent-1",
        targetAgentId: "data_analyst",
        result: { response: "done" },
      }),
    ]);

    const html = renderToStaticMarkup(<RunTimeline items={items} />);

    expect(html).toContain("execute_query");
    expect(html).toContain("select product");
    expect(html).toContain("Sparkling Water");
    expect(html).toContain("data_analyst");
    expect(html).toContain("done");
  });

  test("renders compact run activity with a run detail link", () => {
    const items = projectRunTimeline([
      event(1, "tool.call.started", {
        toolCallId: "tool-1",
        toolName: "search_catalog",
        arguments: { query: "products" },
      }),
    ]);

    const html = renderToStaticMarkup(
      <MemoryRouter>
        <RunActivityPanel runId="run-1" items={items} />
      </MemoryRouter>
    );

    expect(html).toContain("Run activity");
    expect(html).toContain("/runs/run-1");
    expect(html).toContain("search_catalog");
  });

  test("renders summary run activity as a run-detail affordance only", () => {
    const items = projectRunTimeline([
      event(1, "tool.call.started", {
        toolCallId: "tool-1",
        toolName: "search_catalog",
        arguments: { query: "products" },
      }),
      event(2, "tool.call.completed", {
        toolCallId: "tool-1",
        toolName: "search_catalog",
        result: { count: 3 },
      }),
      event(3, "sub_agent.call.completed", {
        toolCallId: "agent-1",
        targetAgentId: "analyst",
        result: { response: "done" },
      }),
    ]);

    const html = renderToStaticMarkup(
      <MemoryRouter>
        <RunActivitySummaryPanel runId="run-1" items={items} />
      </MemoryRouter>
    );

    expect(html).toContain("Run activity");
    expect(html).toContain("2 events");
    expect(html).toContain("Go to run");
    expect(html).toContain("/runs/run-1");
    expect(html).not.toContain("Tools");
    expect(html).not.toContain("Sub-agents");
    expect(html).not.toContain("search_catalog");
    expect(html).not.toContain("raw event");
  });
});
