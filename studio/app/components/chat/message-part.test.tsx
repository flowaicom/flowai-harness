import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { groupParts } from "~/lib/domain/message";
import { DisplayPartDisplay, MessagePartDisplay } from "./message-part";

describe("MessagePartDisplay", () => {
  test("renders approval-required parts with inline decision actions", () => {
    const html = renderToStaticMarkup(
      <MessagePartDisplay
        part={{
          type: "approval-required",
          approvalId: "approval-1",
          title: "Execute remediation plan",
          kind: "plan",
          status: "pending",
          payload: { target: "plan-1" },
        }}
        isUserMessage={false}
      />
    );

    expect(html).toContain("Execute remediation plan");
    expect(html).toContain("Approve");
    expect(html).toContain("Reject");
    expect(html).toContain("Revise");
  });

  test("renders sub-agent delegation above tool calls without call_agent as a tool label", () => {
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
        result: { response: "Products listed" },
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

    const html = renderToStaticMarkup(
      <div>
        {grouped.map((part, index) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: static regression fixture
          <DisplayPartDisplay key={index} part={part} isUserMessage={false} />
        ))}
      </div>
    );

    expect(html).toContain("data_analyst");
    expect(html).toContain("lucide-bot");
    expect(html).toContain("Input");
    expect(html).toContain("agent");
    expect(html).toContain("List products");
    expect(html).toContain("Response");
    expect(html).toContain("response");
    expect(html).toContain("Products listed");
    expect(html).toContain("execute_query");
    expect(html).toContain("search_catalog");
    expect(html).not.toContain("dot-purple");
    expect(html).not.toContain("call_agent");
    expect(html.indexOf("data_analyst")).toBeLessThan(html.indexOf("execute_query"));
  });

  test("renders cancelled sub-agent delegation as terminal", () => {
    const html = renderToStaticMarkup(
      <DisplayPartDisplay
        part={{
          type: "sub-agent-invocation",
          toolCallId: "agent-call-1",
          agentName: "data_analyst",
          state: "cancelled",
          parts: [
            {
              type: "tool-agent",
              toolCallId: "agent-call-1",
              agentName: "data_analyst",
              state: "cancelled",
            },
          ],
        }}
        isUserMessage={false}
      />
    );

    expect(html).toContain("data_analyst");
    expect(html).toContain("cancelled");
    expect(html).not.toContain("working...");
    expect(html).not.toContain("animate-pulse");
  });

  test("renders failed tool calls with a cross icon", () => {
    const html = renderToStaticMarkup(
      <MessagePartDisplay
        part={{
          type: "tool-invocation",
          toolCallId: "tool-1",
          toolName: "execute_query",
          args: { sql: "select * from missing_table" },
          state: "result",
          result: {
            error: "relation missing_table does not exist",
            isError: true,
          },
        }}
        isUserMessage={false}
      />
    );

    expect(html).toContain("execute_query");
    expect(html).toContain("lucide-x");
    expect(html).not.toContain("lucide-check");
  });
});
