import { describe, expect, test } from "bun:test";
import type { AgentEndpointOverride, ChatRequestMessage } from "./streaming";
import { createChatRequest } from "./streaming";

// ============================================================================
// createChatRequest — Pure Function Tests
// ============================================================================

describe("createChatRequest", () => {
  test("minimal request: threadId + message, no history", () => {
    const req = createChatRequest("thread-1", "Hello");

    expect(req.threadId).toBe("thread-1");
    expect(req.messages).toEqual([{ role: "user", content: "Hello" }]);
    expect(req.agentModels).toBeUndefined();
    expect(req.agentEndpoints).toBeUndefined();
    expect(req.agentId).toBeUndefined();
    expect(req.role).toBeUndefined();
    expect(req.sessionId).toBeUndefined();
    expect(req.maxTokens).toBeUndefined();
    expect(req.thinkingBudgetTokens).toBeUndefined();
    expect(req.reasoningEffort).toBeUndefined();
    expect(req.cacheControl).toBeUndefined();
  });

  test("history messages are prepended before user message", () => {
    const history: ChatRequestMessage[] = [
      { role: "user", content: "First" },
      { role: "assistant", content: "Response" },
    ];

    const req = createChatRequest("thread-2", "Follow-up", history);

    expect(req.messages).toEqual([
      { role: "user", content: "First" },
      { role: "assistant", content: "Response" },
      { role: "user", content: "Follow-up" },
    ]);
  });

  test("empty history treated as empty array", () => {
    const req = createChatRequest("thread-3", "Solo");

    expect(req.messages).toHaveLength(1);
    expect(req.messages[0]).toEqual({ role: "user", content: "Solo" });
  });

  test("all optional fields passed through", () => {
    const agentModels = { agent1: "claude-3-opus", agent2: "gpt-4" };
    const agentEndpoints: Record<string, AgentEndpointOverride> = {
      agent1: {
        transport: "http",
        settings: { url: "https://example.com" },
        targetModel: "claude-3-opus",
      },
    };

    const req = createChatRequest("thread-4", "Hey", [], {
      agentId: "my-agent",
      role: "analyst",
      sessionId: "sess-42",
      agentModels,
      agentEndpoints,
      maxTokens: 4096,
      thinkingBudgetTokens: 1024,
      reasoningEffort: "high",
      cacheControl: false,
    });

    expect(req.agentId).toBe("my-agent");
    expect(req.role).toBe("analyst");
    expect(req.sessionId).toBe("sess-42");
    expect(req.agentModels).toEqual(agentModels);
    expect(req.agentEndpoints).toEqual(agentEndpoints);
    expect(req.maxTokens).toBe(4096);
    expect(req.thinkingBudgetTokens).toBe(1024);
    expect(req.reasoningEffort).toBe("high");
    expect(req.cacheControl).toBe(false);
  });

  test("modelSettings description serializes to backend chat fields", () => {
    const req = createChatRequest("thread-settings", "Hey", [], {
      modelSettings: {
        maxTokens: 8192,
        thinkingBudgetTokens: 0,
        reasoningEffort: "max",
        cacheControl: true,
      },
    });

    expect(req.maxTokens).toBe(8192);
    expect(req.thinkingBudgetTokens).toBe(0);
    expect(req.reasoningEffort).toBe("max");
    expect(req.cacheControl).toBe(true);
  });

  test("does not mutate the input history array", () => {
    const history: ChatRequestMessage[] = [{ role: "user", content: "Original" }];
    const originalLength = history.length;

    createChatRequest("thread-5", "New message", history);

    expect(history).toHaveLength(originalLength);
  });

  test("empty user message is valid", () => {
    const req = createChatRequest("thread-6", "");

    expect(req.messages).toEqual([{ role: "user", content: "" }]);
  });

  test("system messages in history are preserved", () => {
    const history: ChatRequestMessage[] = [
      { role: "system", content: "You are helpful" },
      { role: "user", content: "Hi" },
      { role: "assistant", content: "Hello!" },
    ];

    const req = createChatRequest("thread-7", "Thanks", history);

    expect(req.messages).toHaveLength(4);
    expect(req.messages[0].role).toBe("system");
    expect(req.messages[3]).toEqual({ role: "user", content: "Thanks" });
  });
});

// ============================================================================
// parseSSELine — Pure Parser (from stream-part module, used by streaming.ts)
// ============================================================================

// parseSSELine is defined in stream-part.ts and imported by streaming.ts.
// We test it here because it is the pure parsing core of the SSE pipeline.

import { parseSSELine } from "~/lib/domain/stream-part";

describe("parseSSELine", () => {
  test("valid data: line returns parsed StreamPart", () => {
    const part = parseSSELine('data: {"type":"text","text":"hello"}');
    expect(part).toEqual({ type: "text", text: "hello" });
  });

  test("data: with extra whitespace after colon", () => {
    const part = parseSSELine('data:   {"type":"text","text":"spaced"}');
    expect(part).toEqual({ type: "text", text: "spaced" });
  });

  test("leading whitespace on line is handled", () => {
    const part = parseSSELine('  data: {"type":"text","text":"indented"}');
    expect(part).toEqual({ type: "text", text: "indented" });
  });

  test("[DONE] sentinel returns null", () => {
    const part = parseSSELine("data: [DONE]");
    expect(part).toBeNull();
  });

  test("empty data payload returns null", () => {
    const part = parseSSELine("data: ");
    expect(part).toBeNull();
  });

  test("non-data line returns null", () => {
    expect(parseSSELine("event: message")).toBeNull();
    expect(parseSSELine("id: 42")).toBeNull();
    expect(parseSSELine("retry: 5000")).toBeNull();
    expect(parseSSELine(": comment")).toBeNull();
  });

  test("empty string returns null", () => {
    expect(parseSSELine("")).toBeNull();
  });

  test("malformed JSON returns null (no throw)", () => {
    const part = parseSSELine("data: {not valid json}");
    expect(part).toBeNull();
  });

  test("finish part parsed correctly", () => {
    const json = JSON.stringify({
      type: "finish",
      finishReason: "stop",
      usage: {
        promptTokens: 100,
        completionTokens: 50,
        totalTokens: 150,
      },
    });
    const part = parseSSELine(`data: ${json}`);
    expect(part).not.toBeNull();
    expect(part!.type).toBe("finish");
  });

  test("error part parsed correctly", () => {
    const json = JSON.stringify({
      type: "error",
      error: { message: "rate limited", code: "rate_limit" },
    });
    const part = parseSSELine(`data: ${json}`);
    expect(part).not.toBeNull();
    expect(part!.type).toBe("error");
  });

  test("tool-invocation part parsed correctly", () => {
    const json = JSON.stringify({
      type: "tool-invocation",
      toolInvocationId: "call-1",
      toolName: "search",
      args: { query: "test" },
      state: "call",
    });
    const part = parseSSELine(`data: ${json}`);
    expect(part).not.toBeNull();
    expect(part!.type).toBe("tool-invocation");
  });
});
