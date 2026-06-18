import { describe, expect, test } from "bun:test";
import * as fc from "fast-check";

import type { StreamPart } from "~/lib/domain/stream-part";
import {
  arbContentStreamPart,
  arbMessagePart,
  arbNoOpStreamPart,
} from "~/lib/test-utils/arbitraries";
import {
  accumulatePart,
  createAccumulator,
  extractTextContent,
  finalizeAccumulator,
  groupParts,
  isApprovalRequiredMessagePart,
  isCommandCardPart,
  isFileMessagePart,
  isReasoningMessagePart,
  isTextMessagePart,
  isToolAgentMessagePart,
  isToolInvocationMessagePart,
  isToolProgressMessagePart,
  MessageId,
  parseBackendMessage,
  parsePersistedMessages,
  parseUiMessage,
} from "./message";

describe("message parsing", () => {
  test("parseUiMessage preserves persisted approval-required parts", () => {
    const parsed = parseUiMessage({
      id: "ui-approval",
      role: "assistant",
      parts: [
        {
          type: "approval-required",
          approvalId: "approval-1",
          title: "Execute remediation plan",
          kind: "plan",
          status: "pending",
          payload: { target: "plan-1" },
        },
      ],
    });

    expect(parsed.parts).toEqual([
      {
        type: "approval-required",
        approvalId: "approval-1",
        title: "Execute remediation plan",
        kind: "plan",
        status: "pending",
        payload: { target: "plan-1" },
      },
    ]);
  });

  test("parseBackendMessage filters unknown parts and normalizes progress payloads", () => {
    const parsed = parseBackendMessage({
      id: "msg-1",
      role: "assistant",
      content: null,
      parts: [
        { type: "text", text: "hello" },
        { type: "tool-progress", toolName: "buildPlan", phaseIndex: 1, label: "Planning" },
        { type: "unknown-part", value: 1 },
      ],
      createdAt: "2026-03-27T00:00:00.000Z",
    });

    expect(parsed.parts).toHaveLength(2);
    expect(parsed.parts[0]).toEqual({ type: "text", text: "hello" });
    expect(parsed.parts[1]).toEqual({
      type: "tool-progress",
      toolName: "buildPlan",
      agentName: undefined,
      currentPhaseIndex: 1,
      totalPhases: 1,
      phases: [{ index: 1, label: "Planning", milestone: undefined }],
    });
  });

  test("parseBackendMessage accepts harness messageId and metadata parts", () => {
    const parsed = parseBackendMessage({
      messageId: "harness-message-1",
      role: "assistant",
      content: "flat fallback",
      metadata: {
        runId: "run-1",
        parts: [
          {
            type: "tool-invocation",
            toolCallId: "tool-1",
            toolName: "execute_query",
            args: { query: "select 1" },
            state: "result",
            result: { rowCount: 1 },
          },
          { type: "text", text: "done" },
        ],
      },
    });

    expect(parsed.id).toBe("harness-message-1");
    expect(parsed.runId).toBe("run-1");
    expect(parsed.parts).toEqual([
      {
        type: "tool-invocation",
        toolCallId: "tool-1",
        toolName: "execute_query",
        args: { query: "select 1" },
        state: "result",
        result: { rowCount: 1 },
        progress: undefined,
      },
      { type: "text", text: "done" },
    ]);
  });

  test("parsePersistedMessages respects ui format and metadata timestamp fallback", () => {
    const parsed = parsePersistedMessages(
      [
        {
          id: "ui-1",
          role: "assistant",
          parts: [{ type: "text", text: "done" }],
          metadata: { createdAt: "2026-03-27T12:00:00.000Z", runId: "run-ui-1" },
        },
      ],
      "ui"
    );

    expect(parsed).toHaveLength(1);
    expect(parsed[0].createdAt).toBe("2026-03-27T12:00:00.000Z");
    expect(parsed[0].runId).toBe("run-ui-1");
    expect(parsed[0].parts).toEqual([{ type: "text", text: "done" }]);
  });

  test("parsePersistedMessages preserves backend text content", () => {
    const parsed = parsePersistedMessages(
      [
        {
          id: "backend-1",
          role: "user",
          content: "plain text",
        },
      ],
      "backend"
    );

    expect(parsed).toEqual([
      {
        id: MessageId("backend-1"),
        role: "user",
        parts: [{ type: "text", text: "plain text" }],
        createdAt: expect.any(String),
      },
    ]);
  });

  test("parseBackendMessage preserves response validation metadata", () => {
    const parsed = parseBackendMessage({
      id: "backend-contract",
      role: "assistant",
      content: '{"sku":"DEMO-1"}',
      responseValidation: {
        ok: true,
        contract: {
          role: "coordinator",
          modelRef: "models.OrderResponse",
          modelName: "OrderResponse",
        },
        parsed: { sku: "DEMO-1" },
      },
    });

    expect(parsed.responseValidation).toEqual({
      ok: true,
      contract: {
        role: "coordinator",
        modelRef: "models.OrderResponse",
        modelName: "OrderResponse",
        sourcePath: undefined,
        schema: undefined,
      },
      parsed: { sku: "DEMO-1" },
      errors: undefined,
    });
  });

  test("parseUiMessage preserves response validation metadata from ui payloads", () => {
    const parsed = parseUiMessage({
      id: "ui-contract",
      role: "assistant",
      parts: [{ type: "text", text: "done" }],
      metadata: {
        responseValidation: {
          ok: false,
          contract: {
            role: "coordinator",
            modelRef: "models.OrderResponse",
            modelName: "OrderResponse",
          },
          errors: ["quantity missing"],
        },
      },
    });

    expect(parsed.responseValidation?.ok).toBe(false);
    expect(parsed.responseValidation?.errors).toEqual(["quantity missing"]);
  });

  test("parseBackendMessage uses explicit now fallback deterministically", () => {
    const parsed = parseBackendMessage(
      {
        id: "backend-2",
        role: "assistant",
        content: "hello",
      },
      "2026-03-27T10:00:00.000Z"
    );

    expect(parsed.id).toBe(MessageId("backend-2"));
    expect(parsed.createdAt).toBe("2026-03-27T10:00:00.000Z");
  });

  test("parseUiMessage normalizes metadata timestamp fallback deterministically", () => {
    const parsed = parseUiMessage(
      {
        id: "ui-2",
        role: "assistant",
        parts: [{ type: "text", text: "done" }],
        metadata: { createdAt: "2026-03-27T12:34:56.000Z" },
      },
      "2099-12-31T23:59:59.999Z"
    );

    expect(parsed.id).toBe(MessageId("ui-2"));
    expect(parsed.createdAt).toBe("2026-03-27T12:34:56.000Z");
  });

  test("parsePersistedMessages threads explicit now through both formats", () => {
    const backend = parsePersistedMessages(
      [{ id: "backend-3", role: "assistant", content: "text" }],
      "backend",
      "2026-03-27T15:00:00.000Z"
    );
    const ui = parsePersistedMessages(
      [{ id: "ui-3", role: "assistant", parts: [{ type: "text", text: "ui" }] }],
      "ui",
      "2026-03-27T16:00:00.000Z"
    );

    expect(backend[0]?.createdAt).toBe("2026-03-27T15:00:00.000Z");
    expect(ui[0]?.createdAt).toBe("2026-03-27T16:00:00.000Z");
  });
});

describe("message constructors", () => {
  test("finalizeAccumulator uses provided now for deterministic createdAt", () => {
    const acc = createAccumulator(MessageId("acc-1"));
    const message = finalizeAccumulator(acc, "2026-03-27T18:00:00.000Z");

    expect(message.createdAt).toBe("2026-03-27T18:00:00.000Z");
    expect(message.role).toBe("assistant");
    expect(message.isStreaming).toBe(false);
  });
});

// ============================================================================
// Property-Based Tests (Interpreter Layer)
// ============================================================================

// -- Accumulator State Machine --

describe("accumulatePart (property-based)", () => {
  test("ID preservation: id is invariant through any sequence of parts", () => {
    fc.assert(
      fc.property(fc.string({ minLength: 1 }), fc.array(arbContentStreamPart), (id, parts) => {
        let acc = createAccumulator(MessageId(id));
        for (const part of parts) {
          acc = accumulatePart(acc, part);
        }
        expect(acc.id).toBe(MessageId(id));
      })
    );
  });

  test("text concatenation: consecutive text parts accumulate in buffer", () => {
    fc.assert(
      fc.property(fc.array(fc.string(), { minLength: 1 }), (texts) => {
        let acc = createAccumulator(MessageId("test"));
        for (const t of texts) {
          acc = accumulatePart(acc, { type: "text", text: t });
        }
        expect(acc.textBuffer).toBe(texts.join(""));
      })
    );
  });

  test("no-op parts leave accumulator unchanged", () => {
    fc.assert(
      fc.property(arbNoOpStreamPart, (part) => {
        const acc = createAccumulator(MessageId("test"));
        const after = accumulatePart(acc, part);
        expect(after.textBuffer).toBe(acc.textBuffer);
        expect(after.parts).toEqual(acc.parts);
        expect(after.pendingTools.size).toBe(acc.pendingTools.size);
        expect(after.pendingAgents.size).toBe(acc.pendingAgents.size);
      })
    );
  });

  test("part count is monotonically non-decreasing through content parts", () => {
    fc.assert(
      fc.property(fc.array(arbContentStreamPart), (parts) => {
        let acc = createAccumulator(MessageId("test"));
        let prevCount = 0;
        for (const part of parts) {
          acc = accumulatePart(acc, part);
          // Parts can only grow or stay same (reasoning merge is the only case
          // where a new part doesn't increase count, but it replaces in-place)
          expect(acc.parts.length).toBeGreaterThanOrEqual(prevCount);
          prevCount = acc.parts.length;
        }
      })
    );
  });
});

// -- Finalizer --

describe("finalizeAccumulator (property-based)", () => {
  test("role is always assistant, isStreaming always false", () => {
    fc.assert(
      fc.property(fc.array(arbContentStreamPart), (parts) => {
        let acc = createAccumulator(MessageId("test"));
        for (const part of parts) {
          acc = accumulatePart(acc, part);
        }
        const msg = finalizeAccumulator(acc, "2026-01-01T00:00:00Z");
        expect(msg.role).toBe("assistant");
        expect(msg.isStreaming).toBe(false);
      })
    );
  });

  test("captures text buffer: non-empty buffer appears in finalized parts", () => {
    fc.assert(
      fc.property(fc.string({ minLength: 1 }), (text) => {
        let acc = createAccumulator(MessageId("test"));
        acc = accumulatePart(acc, { type: "text", text });
        const msg = finalizeAccumulator(acc, "2026-01-01T00:00:00Z");
        const allText = msg.parts
          .filter((p): p is { type: "text"; text: string } => p.type === "text")
          .map((p) => p.text)
          .join("");
        expect(allText).toContain(text);
      })
    );
  });

  test("ID roundtrip: finalized message.id = accumulator.id", () => {
    fc.assert(
      fc.property(fc.string({ minLength: 1 }), fc.array(arbContentStreamPart), (id, parts) => {
        let acc = createAccumulator(MessageId(id));
        for (const part of parts) {
          acc = accumulatePart(acc, part);
        }
        const msg = finalizeAccumulator(acc, "2026-01-01T00:00:00Z");
        expect(msg.id).toBe(MessageId(id));
      })
    );
  });
});

// -- MessagePart Type Guard Mutual Exclusivity --

describe("MessagePart type guards (property-based)", () => {
  test("exactly one guard returns true per part", () => {
    const guards = [
      isTextMessagePart,
      isReasoningMessagePart,
      isToolInvocationMessagePart,
      isToolAgentMessagePart,
      isFileMessagePart,
      isCommandCardPart,
      isApprovalRequiredMessagePart,
      isToolProgressMessagePart,
    ];
    fc.assert(
      fc.property(arbMessagePart, (part) => {
        const matches = guards.filter((g) => g(part));
        expect(matches).toHaveLength(1);
      })
    );
  });
});

// -- Utility Functions --

describe("extractTextContent (property-based)", () => {
  test("extracts only text parts and concatenates", () => {
    fc.assert(
      fc.property(fc.array(arbMessagePart), (parts) => {
        const result = extractTextContent(parts);
        const expected = parts
          .filter((p) => p.type === "text")
          .map((p) => (p as { text: string }).text)
          .join("");
        expect(result).toBe(expected);
      })
    );
  });

  test("empty parts produce empty string", () => {
    expect(extractTextContent([])).toBe("");
  });
});

describe("groupParts (property-based)", () => {
  test("no parts are lost: total tool/non-tool count is preserved", () => {
    fc.assert(
      fc.property(fc.array(arbMessagePart), (parts) => {
        const grouped = groupParts(parts);
        let count = 0;
        for (const g of grouped) {
          if (g.type === "tool-group") {
            count += g.parts.length;
          } else {
            count++;
          }
        }
        expect(count).toBe(parts.length);
      })
    );
  });

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

  test("single tools pass through ungrouped", () => {
    fc.assert(
      fc.property(arbMessagePart, (part) => {
        const grouped = groupParts([part]);
        // A single part should never produce a tool-group
        expect(grouped.some((g) => g.type === "tool-group")).toBe(false);
      })
    );
  });
});

describe("accumulatePart tool boundaries", () => {
  test("approval-required custom events become visible message parts", () => {
    let acc = createAccumulator(MessageId("approval-visible"));
    acc = accumulatePart(acc, { type: "text", text: "I need confirmation first." });
    acc = accumulatePart(acc, {
      type: "custom",
      name: "approval.required",
      data: {
        approvalId: "approval-1",
        title: "Execute remediation plan",
        kind: "plan",
        raw: { target: "plan-1" },
      },
    });

    const message = finalizeAccumulator(acc, "2026-01-01T00:00:00Z");
    expect(message.parts).toEqual([
      { type: "text", text: "I need confirmation first." },
      {
        type: "approval-required",
        approvalId: "approval-1",
        title: "Execute remediation plan",
        kind: "plan",
        status: "pending",
        payload: { target: "plan-1" },
      },
    ]);
  });

  test("approval-decision custom events update an existing approval part", () => {
    let acc = createAccumulator(MessageId("approval-decision"));
    acc = accumulatePart(acc, {
      type: "custom",
      name: "approval.required",
      data: {
        approvalId: "approval-1",
        title: "Execute remediation plan",
        kind: "plan",
        raw: { target: "plan-1" },
      },
    });
    acc = accumulatePart(acc, {
      type: "custom",
      name: "approval.decision",
      data: {
        approvalId: "approval-1",
        status: "approve",
      },
    });

    const message = finalizeAccumulator(acc, "2026-01-01T00:00:00Z");
    expect(message.parts).toEqual([
      {
        type: "approval-required",
        approvalId: "approval-1",
        title: "Execute remediation plan",
        kind: "plan",
        status: "approve",
        payload: { target: "plan-1" },
      },
    ]);
  });

  test("tool result preserves pending call args when completion omits arguments", () => {
    let acc = createAccumulator(MessageId("tool-args"));
    acc = accumulatePart(acc, {
      type: "tool-invocation",
      toolInvocationId: "tool-1",
      toolName: "execute_query",
      args: { query: "select * from orders" },
      state: "call",
    });
    acc = accumulatePart(acc, {
      type: "tool-invocation",
      toolInvocationId: "tool-1",
      toolName: "execute_query",
      args: undefined,
      state: "result",
      result: { rowCount: 1 },
    });

    const message = finalizeAccumulator(acc, "2026-01-01T00:00:00Z");
    expect(message.parts).toEqual([
      {
        type: "tool-invocation",
        toolCallId: "tool-1",
        toolName: "execute_query",
        args: { query: "select * from orders" },
        state: "result",
        result: { rowCount: 1 },
      },
    ]);
  });

  test("duplicate tool results with the same call id update instead of appending", () => {
    let acc = createAccumulator(MessageId("tool-dedupe"));
    acc = accumulatePart(acc, {
      type: "tool-invocation",
      toolInvocationId: "tool-1",
      toolName: "search_catalog",
      args: { query: "products" },
      state: "call",
    });
    acc = accumulatePart(acc, {
      type: "tool-invocation",
      toolInvocationId: "tool-1",
      toolName: "search_catalog",
      args: undefined,
      state: "result",
      result: { count: 1 },
    });
    acc = accumulatePart(acc, {
      type: "tool-invocation",
      toolInvocationId: "tool-1",
      toolName: "search_catalog",
      args: undefined,
      state: "result",
      result: { count: 1 },
    });

    const message = finalizeAccumulator(acc, "2026-01-01T00:00:00Z");
    expect(message.parts).toEqual([
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

  test("tool and sub-agent calls flush preceding text before later text", () => {
    let acc = createAccumulator(MessageId("tool-boundary"));
    acc = accumulatePart(acc, { type: "text", text: "Before tool. " });
    acc = accumulatePart(acc, {
      type: "tool-invocation",
      toolInvocationId: "tool-1",
      toolName: "execute_query",
      args: {},
      state: "call",
    });
    acc = accumulatePart(acc, { type: "text", text: "After tool call. " });
    acc = accumulatePart(acc, {
      type: "tool-invocation",
      toolInvocationId: "tool-1",
      toolName: "execute_query",
      args: {},
      state: "result",
      result: { ok: true },
    });
    acc = accumulatePart(acc, {
      type: "tool-agent",
      toolInvocationId: "agent-1",
      agentName: "analyst",
      state: "call",
    });
    acc = accumulatePart(acc, { type: "text", text: "After agent call." });

    const message = finalizeAccumulator(acc, "2026-01-01T00:00:00Z");
    expect(message.parts).toEqual([
      { type: "text", text: "Before tool. " },
      { type: "text", text: "After tool call. " },
      {
        type: "tool-invocation",
        toolCallId: "tool-1",
        toolName: "execute_query",
        args: {},
        state: "result",
        result: { ok: true },
      },
      { type: "text", text: "After agent call." },
      {
        type: "tool-agent",
        toolCallId: "agent-1",
        agentName: "analyst",
        state: "call",
      },
    ]);
  });
});

// ============================================================================
// Full Tool Lifecycle PBT
//
// Generates valid tool-call → (optional progress) → tool-result sequences
// interleaved with text/reasoning, then verifies accumulator invariants:
// - Every tool-call that received a result appears in finalized parts
// - Pending tools without results are still captured on finalize
// - hadToolActivity boundary prevents reasoning merge across tools
// - Text buffer is flushed before tool results
// ============================================================================

/** Generate a complete tool lifecycle: call → optional progress → result. */
const arbToolLifecycle = fc
  .record({
    toolId: fc.uuid(),
    toolName: fc.string({ minLength: 1, maxLength: 15 }),
    hasProgress: fc.boolean(),
    progressLabel: fc.string({ minLength: 1, maxLength: 20 }),
  })
  .map(({ toolId, toolName, hasProgress, progressLabel }) => {
    const parts: StreamPart[] = [
      {
        type: "tool-invocation",
        toolInvocationId: toolId,
        toolName,
        args: {},
        state: "call" as const,
      },
    ];
    if (hasProgress) {
      parts.push({
        type: "tool-progress",
        toolName,
        toolCallId: toolId,
        label: progressLabel,
        phaseIndex: 0,
        totalPhases: 1,
      });
    }
    parts.push({
      type: "tool-invocation",
      toolInvocationId: toolId,
      toolName,
      args: {},
      state: "result" as const,
      result: "ok",
    });
    return { toolId, toolName, parts };
  });

describe("full tool lifecycle (property-based)", () => {
  test("completed tools appear in finalized parts with state=result", () => {
    fc.assert(
      fc.property(arbToolLifecycle, ({ toolId, parts }) => {
        let acc = createAccumulator(MessageId("lifecycle-test"));
        for (const part of parts) {
          acc = accumulatePart(acc, part);
        }
        const msg = finalizeAccumulator(acc, "2026-01-01T00:00:00Z");
        const toolParts = msg.parts.filter((p) => p.type === "tool-invocation");
        expect(toolParts.length).toBeGreaterThanOrEqual(1);
        const match = toolParts.find(
          (p) => p.type === "tool-invocation" && (p as { toolCallId: string }).toolCallId === toolId
        );
        expect(match).toBeDefined();
        expect((match as { state: string }).state).toBe("result");
      })
    );
  });

  test("tool call sets hadToolActivity — reasoning after tool is separate", () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 1, maxLength: 20 }),
        fc.string({ minLength: 1, maxLength: 20 }),
        (reasonBefore, reasonAfter) => {
          let acc = createAccumulator(MessageId("boundary-test"));
          // Reasoning before tool
          acc = accumulatePart(acc, { type: "reasoning", text: reasonBefore });
          // Tool call
          acc = accumulatePart(acc, {
            type: "tool-invocation",
            toolInvocationId: "t1",
            toolName: "test",
            args: {},
            state: "call" as const,
          });
          // Tool result
          acc = accumulatePart(acc, {
            type: "tool-invocation",
            toolInvocationId: "t1",
            toolName: "test",
            args: {},
            state: "result" as const,
            result: "ok",
          });
          // Reasoning after tool — must NOT merge with the one before
          acc = accumulatePart(acc, { type: "reasoning", text: reasonAfter });

          const msg = finalizeAccumulator(acc, "2026-01-01T00:00:00Z");
          const reasoningParts = msg.parts.filter((p) => p.type === "reasoning");
          // Should be 2 separate reasoning parts (not merged)
          expect(reasoningParts.length).toBe(2);
        }
      )
    );
  });

  test("interleaved text + tools: text flushed before each tool result", () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 1, maxLength: 20 }),
        arbToolLifecycle,
        (textBefore, { parts: toolParts }) => {
          let acc = createAccumulator(MessageId("flush-test"));
          // Text before tool
          acc = accumulatePart(acc, { type: "text", text: textBefore });
          // Tool lifecycle
          for (const part of toolParts) {
            acc = accumulatePart(acc, part);
          }
          const msg = finalizeAccumulator(acc, "2026-01-01T00:00:00Z");
          // Text part should appear before the tool invocation
          const textIdx = msg.parts.findIndex((p) => p.type === "text");
          const toolIdx = msg.parts.findIndex((p) => p.type === "tool-invocation");
          if (textIdx !== -1 && toolIdx !== -1) {
            expect(textIdx).toBeLessThan(toolIdx);
          }
        }
      )
    );
  });

  test("multiple tools: each gets its own part in finalized message", () => {
    fc.assert(
      fc.property(arbToolLifecycle, arbToolLifecycle, (tool1, tool2) => {
        // Skip if same toolId (unlikely with uuid but possible)
        if (tool1.toolId === tool2.toolId) return;

        let acc = createAccumulator(MessageId("multi-tool"));
        for (const part of tool1.parts) {
          acc = accumulatePart(acc, part);
        }
        for (const part of tool2.parts) {
          acc = accumulatePart(acc, part);
        }
        const msg = finalizeAccumulator(acc, "2026-01-01T00:00:00Z");
        const toolParts = msg.parts.filter((p) => p.type === "tool-invocation");
        expect(toolParts.length).toBeGreaterThanOrEqual(2);
      })
    );
  });

  test("pending tool (call without result) captured on finalize", () => {
    fc.assert(
      fc.property(fc.uuid(), fc.string({ minLength: 1, maxLength: 15 }), (toolId, toolName) => {
        let acc = createAccumulator(MessageId("pending-test"));
        // Only send the call, no result
        acc = accumulatePart(acc, {
          type: "tool-invocation",
          toolInvocationId: toolId,
          toolName,
          args: {},
          state: "call" as const,
        });
        const msg = finalizeAccumulator(acc, "2026-01-01T00:00:00Z");
        // Pending tool should still appear in finalized parts
        const toolParts = msg.parts.filter((p) => p.type === "tool-invocation");
        expect(toolParts.length).toBe(1);
        expect((toolParts[0] as { state: string }).state).toBe("call");
      })
    );
  });
});
