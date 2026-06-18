import { beforeEach, describe, expect, test } from "bun:test";
import * as fc from "fast-check";
import { enableMapSet } from "immer";
import type { Message, MessagePart, StreamPart } from "~/lib/domain";
import { MessageId } from "~/lib/domain/message";
import {
  arbErrorStreamPart as arbErrorSP,
  arbNoOpStreamPart as arbNoOpSP,
  arbReasoningStreamPart as arbReasoningSP,
  arbTextStreamPart as arbTextSP,
} from "~/lib/test-utils/arbitraries";
import type { ChatStreamSession, StreamingStats } from "./chat-store";
import { selectStreamPhase, useConversation } from "./chat-store";

// Immer MapSet must be enabled before any store interaction
enableMapSet();

// ============================================================================
// Helpers
// ============================================================================

/** Reset store to clean state between tests. Preserves action functions. */
beforeEach(() => {
  useConversation.getState().reset();
});

/** Shorthand for store state. */
const state = () => useConversation.getState();

/** Shorthand for store actions. */
const act = () => useConversation.getState();

/** Get the stream session for a threadId. */
const session = (threadId: string): ChatStreamSession | undefined =>
  state().streamSessions.get(threadId);

/** Deterministic IDs for tests. */
let idCounter = 0;
const nextId = (): MessageId => MessageId(`test-id-${++idCounter}`);
const NOW_ISO = "2026-03-27T10:00:00.000Z";
const NOW_MS = 1_000_000;

/** Start a stream with deterministic values. */
const startStream = (threadId: string, ac?: AbortController) => {
  act().startStreaming(threadId, ac ?? new AbortController(), nextId(), NOW_MS);
};

/** Build a text StreamPart. */
const textPart = (text: string): StreamPart => ({ type: "text", text });

/** Build a reasoning StreamPart. */
const reasoningPart = (text: string): StreamPart => ({ type: "reasoning", text });

/** Build a tool-invocation "call" StreamPart. */
const toolCallPart = (id: string, name: string, args: unknown = {}): StreamPart => ({
  type: "tool-invocation",
  toolInvocationId: id,
  toolName: name,
  args,
  state: "call",
});

/** Build a tool-invocation "result" StreamPart. */
const toolResultPart = (id: string, name: string, result: unknown = "ok"): StreamPart => ({
  type: "tool-invocation",
  toolInvocationId: id,
  toolName: name,
  args: {},
  state: "result",
  result,
});

/** Build a tool-agent "call" StreamPart. */
const toolAgentCallPart = (id: string, agentName: string): StreamPart => ({
  type: "tool-agent",
  toolInvocationId: id,
  agentName,
  state: "call",
});

/** Build a data-cost-summary StreamPart. */
const costSummaryPart = (totalTokens = 100): StreamPart => ({
  type: "data-cost-summary",
  data: {
    agents: [],
    totalPromptTokens: 50,
    totalCompletionTokens: 50,
    totalTokens,
  },
});

/** Build a data-latency-summary StreamPart. */
const latencySummaryPart = (totalDurationMs = 500): StreamPart => ({
  type: "data-latency-summary",
  data: {
    totalDurationMs,
    phases: { llmTimeMs: 300, toolTimeMs: 100, llmCalls: 2 },
    toolTimings: [],
    retryCount: 0,
    hadTimeout: false,
  },
});

/** Build a data-file-registered StreamPart. */
const fileRegisteredPart = (threadId: string, fileId = "f1"): StreamPart => ({
  type: "data-file-registered",
  data: {
    fileId,
    filename: `${fileId}.csv`,
    threadId,
    timestamp: new Date().toISOString(),
  },
});

/** Build a finish StreamPart. */
const finishPart = (finishReason: "stop" | "tool-calls" = "stop"): StreamPart => ({
  type: "finish",
  finishReason,
  usage: {
    promptTokens: 50,
    completionTokens: 50,
    totalTokens: 100,
  },
});

/** Build a deterministic Message for seeding history. */
const mkMessage = (id: string, role: "user" | "assistant", text: string): Message => ({
  id: MessageId(id),
  role,
  parts: [{ type: "text", text }],
  createdAt: NOW_ISO,
});

// ============================================================================
// startStreaming
// ============================================================================

describe("startStreaming", () => {
  test("creates session with correct threadId", () => {
    startStream("thread-1");
    const s = session("thread-1");
    expect(s).toBeDefined();
    expect(s!.threadId).toBe("thread-1");
  });

  test("sets streamPhase to streaming", () => {
    startStream("thread-1");
    const s = session("thread-1")!;
    expect(s.streamPhase.phase).toBe("streaming");
    expect((s.streamPhase as { startedAt: number }).startedAt).toBe(NOW_MS);
  });

  test("initializes accumulator", () => {
    startStream("thread-1");
    const s = session("thread-1")!;
    expect(s.liveMessage).not.toBeNull();
    expect(s.liveMessage!.textBuffer).toBe("");
    expect(s.liveMessage!.parts).toEqual([]);
    expect(s.liveMessage!.pendingTools.size).toBe(0);
  });

  test("does not affect other sessions (concurrent streaming)", () => {
    startStream("thread-1");
    act().interpretStreamEvent("thread-1", textPart("hello from 1"));

    startStream("thread-2");

    // thread-1 should still have its accumulated text
    const s1 = session("thread-1")!;
    expect(s1.liveMessage!.textBuffer).toBe("hello from 1");

    // thread-2 should be fresh
    const s2 = session("thread-2")!;
    expect(s2.liveMessage!.textBuffer).toBe("");
  });
});

// ============================================================================
// interpretStreamEvent
// ============================================================================

describe("interpretStreamEvent", () => {
  beforeEach(() => {
    startStream("thread-1");
  });

  test("text part accumulates into accumulator correctly", () => {
    act().interpretStreamEvent("thread-1", textPart("Hello"));
    act().interpretStreamEvent("thread-1", textPart(" world"));

    const s = session("thread-1")!;
    expect(s.liveMessage!.textBuffer).toBe("Hello world");
  });

  test("tool call parts tracked in pendingTools", () => {
    act().interpretStreamEvent("thread-1", toolCallPart("tc-1", "search", { q: "test" }));

    const s = session("thread-1")!;
    expect(s.liveMessage!.pendingTools.size).toBe(1);
    expect(s.liveMessage!.pendingTools.has("tc-1")).toBe(true);
    const pending = s.liveMessage!.pendingTools.get("tc-1")!;
    expect(pending.toolName).toBe("search");
    expect(pending.state).toBe("call");
  });

  test("tool result clears pending and adds to parts", () => {
    act().interpretStreamEvent("thread-1", toolCallPart("tc-1", "search"));
    act().interpretStreamEvent("thread-1", toolResultPart("tc-1", "search", { rows: 5 }));

    const s = session("thread-1")!;
    expect(s.liveMessage!.pendingTools.size).toBe(0);
    const toolParts = s.liveMessage!.parts.filter((p) => p.type === "tool-invocation");
    expect(toolParts.length).toBe(1);
    expect((toolParts[0] as { state: string }).state).toBe("result");
  });

  test("cost summary extracted (data-cost-summary)", () => {
    act().interpretStreamEvent("thread-1", costSummaryPart(200));

    const s = session("thread-1")!;
    expect(s.tokenCost).not.toBeNull();
    expect(s.tokenCost!.totalTokens).toBe(200);
  });

  test("server latency metrics captured (data-latency-summary)", () => {
    act().interpretStreamEvent("thread-1", latencySummaryPart(750));

    const s = session("thread-1")!;
    expect(s.serverMetrics).not.toBeNull();
    expect(s.serverMetrics!.totalDurationMs).toBe(750);
    expect(s.serverMetrics!.phases.llmCalls).toBe(2);
    expect(s.serverMetrics!.retryCount).toBe(0);
    expect(s.serverMetrics!.hadTimeout).toBe(false);
  });

  test("file-registered events increment file counter", () => {
    expect(state().fileChangeCounters["thread-1"] ?? 0).toBe(0);

    act().interpretStreamEvent("thread-1", fileRegisteredPart("thread-1", "f1"));
    expect(state().fileChangeCounters["thread-1"]).toBe(1);

    act().interpretStreamEvent("thread-1", fileRegisteredPart("thread-1", "f2"));
    expect(state().fileChangeCounters["thread-1"]).toBe(2);
  });

  test("file-registered counter scoped to file's threadId, not stream's threadId", () => {
    act().interpretStreamEvent("thread-1", fileRegisteredPart("thread-other", "f1"));
    expect(state().fileChangeCounters["thread-other"]).toBe(1);
    expect(state().fileChangeCounters["thread-1"] ?? 0).toBe(0);
  });

  test("computeLiveParts returns correct pending tools + text buffer", () => {
    act().interpretStreamEvent("thread-1", toolCallPart("tc-1", "search"));
    act().interpretStreamEvent("thread-1", textPart("partial response"));

    const s = session("thread-1")!;
    const parts = s.liveParts;

    const toolParts = parts.filter((p) => p.type === "tool-invocation");
    const textParts = parts.filter((p) => p.type === "text");

    expect(toolParts.length).toBe(1);
    expect(textParts.length).toBe(1);
    expect((textParts[0] as { text: string }).text).toBe("partial response");
  });

  test("computeLiveStats counts text length, tool calls, reasoning", () => {
    act().interpretStreamEvent("thread-1", reasoningPart("thinking..."));
    act().interpretStreamEvent("thread-1", toolCallPart("tc-1", "search"));
    act().interpretStreamEvent("thread-1", toolResultPart("tc-1", "search"));
    act().interpretStreamEvent("thread-1", textPart("Hello"));

    const s = session("thread-1")!;
    const stats = s.liveStats!;

    expect(stats.hasReasoning).toBe(true);
    expect(stats.toolCallCount).toBe(1);
    expect(stats.completedToolCount).toBe(1);
    expect(stats.pendingToolCount).toBe(0);
    expect(stats.textLength).toBe(5); // "Hello" in textBuffer
  });

  test("finish event captures finishReason", () => {
    act().interpretStreamEvent("thread-1", finishPart("tool-calls"));

    const s = session("thread-1")!;
    expect(s.finishReason).toBe("tool-calls");
  });

  test("error event captures streamError", () => {
    const errorPart: StreamPart = {
      type: "error",
      error: { message: "rate limit exceeded", code: "429" },
    };
    act().interpretStreamEvent("thread-1", errorPart);

    const s = session("thread-1")!;
    expect(s.streamError).toBe("rate limit exceeded");
  });

  test("no-op when session does not exist", () => {
    act().interpretStreamEvent("nonexistent", textPart("ignored"));
    expect(session("nonexistent")).toBeUndefined();
  });
});

// ============================================================================
// finishStream
// ============================================================================

describe("finishStream", () => {
  test("finalizes accumulator into message", () => {
    act().setThreadId("thread-1");
    startStream("thread-1");
    act().interpretStreamEvent("thread-1", textPart("Final answer"));

    act().finishStream("thread-1", NOW_ISO);

    const lastMsg = state().history[state().history.length - 1];
    expect(lastMsg).toBeDefined();
    expect(lastMsg.role).toBe("assistant");
    expect(lastMsg.parts.length).toBeGreaterThan(0);
    const textContent = lastMsg.parts
      .filter((p) => p.type === "text")
      .map((p) => (p as { text: string }).text)
      .join("");
    expect(textContent).toBe("Final answer");
  });

  test("pushes finalized message to history (focused thread)", () => {
    act().setThreadId("thread-1");
    startStream("thread-1");
    act().interpretStreamEvent("thread-1", textPart("answer"));

    const beforeCount = state().history.length;
    act().finishStream("thread-1", NOW_ISO);

    expect(state().history.length).toBe(beforeCount + 1);
  });

  test("session cleaned up after completion", () => {
    act().setThreadId("thread-1");
    startStream("thread-1");
    act().interpretStreamEvent("thread-1", textPart("done"));
    act().finishStream("thread-1", NOW_ISO);

    expect(state().streamSessions.size).toBe(0);
    expect(session("thread-1")).toBeUndefined();
  });

  test("backgrounded thread updates cache instead of history", () => {
    act().setThreadId("thread-1");
    act().setMessages([mkMessage("m1", "user", "hello")]);

    // Start streaming on thread-1, then focus thread-2
    startStream("thread-1");
    act().interpretStreamEvent("thread-1", textPart("bg answer"));
    act().setThreadId("thread-2");

    // Finish the backgrounded stream
    act().finishStream("thread-1", NOW_ISO);

    // History should be thread-2's (empty), not thread-1's
    expect(state().threadId).toBe("thread-2");

    // The cached version of thread-1 should have the finalized message
    const cached = state().historyCache.get("thread-1");
    expect(cached).toBeDefined();
    expect(cached!.length).toBe(2); // original user msg + finalized assistant msg
  });

  test("uses caller-supplied timestamp for createdAt", () => {
    act().setThreadId("thread-1");
    startStream("thread-1");
    act().interpretStreamEvent("thread-1", textPart("timed"));
    act().finishStream("thread-1", "2026-03-27T12:00:00.000Z");

    const lastMsg = state().history[state().history.length - 1];
    expect(lastMsg.createdAt).toBe("2026-03-27T12:00:00.000Z");
  });
});

// ============================================================================
// abortStream
// ============================================================================

describe("abortStream", () => {
  test("captures partial message", () => {
    act().setThreadId("thread-1");
    startStream("thread-1");
    act().interpretStreamEvent("thread-1", textPart("partial"));

    act().abortStream("thread-1", NOW_ISO);

    const lastMsg = state().history[state().history.length - 1];
    expect(lastMsg).toBeDefined();
    expect(lastMsg.role).toBe("assistant");
  });

  test("finalizes what was accumulated so far", () => {
    act().setThreadId("thread-1");
    startStream("thread-1");
    act().interpretStreamEvent("thread-1", textPart("Hello "));
    act().interpretStreamEvent("thread-1", textPart("wor"));

    act().abortStream("thread-1", NOW_ISO);

    const lastMsg = state().history[state().history.length - 1];
    const text = lastMsg.parts
      .filter((p) => p.type === "text")
      .map((p) => (p as { text: string }).text)
      .join("");
    expect(text).toBe("Hello wor");
  });

  test("marks pending sub-agent calls as cancelled", () => {
    act().setThreadId("thread-1");
    startStream("thread-1");
    act().interpretStreamEvent("thread-1", textPart("Routing request"));
    act().interpretStreamEvent("thread-1", toolAgentCallPart("agent-call-1", "data_analyst"));

    act().abortStream("thread-1", NOW_ISO);

    const lastMsg = state().history[state().history.length - 1];
    const agentPart = lastMsg.parts.find(
      (part): part is Extract<MessagePart, { type: "tool-agent" }> => part.type === "tool-agent"
    );
    expect(agentPart).toMatchObject({
      agentName: "data_analyst",
      state: "cancelled",
    });
  });

  test("properly cleans up session", () => {
    startStream("thread-1");
    act().abortStream("thread-1", NOW_ISO);

    expect(session("thread-1")).toBeUndefined();
    expect(state().streamSessions.size).toBe(0);
  });

  test("aborts the abort controller", () => {
    const ac = new AbortController();
    act().setThreadId("thread-1");
    act().startStreaming("thread-1", ac, nextId(), NOW_MS);

    expect(ac.signal.aborted).toBe(false);
    act().abortStream("thread-1", NOW_ISO);
    expect(ac.signal.aborted).toBe(true);
  });

  test("no-op when session does not exist", () => {
    act().abortStream("nonexistent", NOW_ISO);
    expect(state().streamSessions.size).toBe(0);
  });

  test("abort with empty accumulator still pushes finalized message", () => {
    act().setThreadId("thread-1");
    startStream("thread-1");
    // liveMessage is initialized but has no content
    const beforeCount = state().history.length;
    act().abortStream("thread-1", NOW_ISO);

    // The accumulator was non-null so it finalizes and pushes an empty assistant message
    expect(state().history.length).toBe(beforeCount + 1);
  });
});

// ============================================================================
// setThreadId
// ============================================================================

describe("setThreadId", () => {
  test("caches old thread's history", () => {
    act().setThreadId("thread-1");
    act().setMessages([mkMessage("m1", "user", "hello")]);

    act().setThreadId("thread-2");

    expect(state().historyCache.has("thread-1")).toBe(true);
    expect(state().historyCache.get("thread-1")!.length).toBe(1);
  });

  test("restores from cache on re-focus", () => {
    act().setThreadId("thread-1");
    act().setMessages([mkMessage("m1", "user", "cached msg")]);

    act().setThreadId("thread-2");
    act().setThreadId("thread-1");

    expect(state().history.length).toBe(1);
    expect(state().history[0].id).toBe(MessageId("m1"));
  });

  test("sets loadPhase to loading on cache miss", () => {
    act().setThreadId("never-seen-thread");
    expect(state().loadPhase.phase).toBe("loading");
  });

  test("sets loadPhase to ready on cache hit", () => {
    act().setThreadId("thread-1");
    act().setMessages([mkMessage("m1", "user", "hi")]);
    act().setThreadId("thread-2"); // caches thread-1

    act().setThreadId("thread-1"); // cache hit
    expect(state().loadPhase.phase).toBe("ready");
  });

  test("does NOT interrupt active streams (streamSessions preserved)", () => {
    act().setThreadId("thread-1");
    startStream("thread-1");
    act().interpretStreamEvent("thread-1", textPart("streaming data"));

    act().setThreadId("thread-2");

    expect(session("thread-1")).toBeDefined();
    expect(session("thread-1")!.liveMessage!.textBuffer).toBe("streaming data");
  });

  test("defensive copy: cache mutations don't affect history", () => {
    act().setThreadId("thread-1");
    act().setMessages([mkMessage("m1", "user", "original")]);

    act().setThreadId("thread-2"); // cache
    act().setThreadId("thread-1"); // restore

    const historyRef = state().history;
    const cacheRef = state().historyCache.get("thread-1");

    // The history and cache entries should be independent copies
    expect(historyRef).not.toBe(cacheRef);
  });

  test("setting threadId to null resets to idle", () => {
    act().setThreadId("thread-1");
    act().setThreadId(null);

    expect(state().threadId).toBeNull();
    expect(state().history).toEqual([]);
    expect(state().loadPhase.phase).toBe("idle");
  });

  test("empty history is not cached", () => {
    act().setThreadId("thread-1");
    // Don't add any messages
    act().setThreadId("thread-2");

    expect(state().historyCache.has("thread-1")).toBe(false);
  });
});

// ============================================================================
// Multi-session concurrency
// ============================================================================

describe("multi-session concurrency", () => {
  test("multiple threads can have active streams simultaneously", () => {
    startStream("thread-1");
    startStream("thread-2");
    startStream("thread-3");

    expect(state().streamSessions.size).toBe(3);
    expect(session("thread-1")!.streamPhase.phase).toBe("streaming");
    expect(session("thread-2")!.streamPhase.phase).toBe("streaming");
    expect(session("thread-3")!.streamPhase.phase).toBe("streaming");
  });

  test("focusing thread A doesn't cancel thread B's stream", () => {
    act().setThreadId("thread-1");
    startStream("thread-1");
    act().interpretStreamEvent("thread-1", textPart("A data"));

    startStream("thread-2");
    act().interpretStreamEvent("thread-2", textPart("B data"));

    act().setThreadId("thread-1");

    expect(session("thread-1")!.liveMessage!.textBuffer).toBe("A data");
    expect(session("thread-2")!.liveMessage!.textBuffer).toBe("B data");
  });

  test("each session maintains independent state", () => {
    startStream("thread-1");
    startStream("thread-2");

    act().interpretStreamEvent("thread-1", textPart("alpha"));
    act().interpretStreamEvent("thread-2", textPart("beta"));
    act().interpretStreamEvent("thread-1", costSummaryPart(100));
    act().interpretStreamEvent("thread-2", costSummaryPart(999));

    const s1 = session("thread-1")!;
    const s2 = session("thread-2")!;

    expect(s1.liveMessage!.textBuffer).toBe("alpha");
    expect(s2.liveMessage!.textBuffer).toBe("beta");
    expect(s1.tokenCost!.totalTokens).toBe(100);
    expect(s2.tokenCost!.totalTokens).toBe(999);
  });

  test("finishing one stream does not affect others", () => {
    act().setThreadId("thread-1");
    startStream("thread-1");
    startStream("thread-2");

    act().interpretStreamEvent("thread-1", textPart("done"));
    act().interpretStreamEvent("thread-2", textPart("still going"));

    act().finishStream("thread-1", NOW_ISO);

    expect(session("thread-1")).toBeUndefined();
    expect(session("thread-2")).toBeDefined();
    expect(session("thread-2")!.liveMessage!.textBuffer).toBe("still going");
  });
});

// ============================================================================
// Derived state stability
// ============================================================================

describe("derived state stability", () => {
  test("idle stream phase selector returns a stable reference", () => {
    const first = selectStreamPhase(state());
    const second = selectStreamPhase(state());

    expect(first).toEqual({ phase: "idle" });
    expect(first).toBe(second);
  });

  test("empty accumulator returns EMPTY_PARTS constant (stable ref)", () => {
    startStream("thread-1");

    const parts1 = session("thread-1")!.liveParts;
    expect(parts1).toEqual([]);

    // Re-read -- should be the same reference (no new array created)
    const parts2 = session("thread-1")!.liveParts;
    expect(parts1).toBe(parts2);
  });

  test("accumulator with no pending tools and no text buffer returns stable parts", () => {
    startStream("thread-1");

    act().interpretStreamEvent("thread-1", toolCallPart("tc-1", "search"));
    act().interpretStreamEvent("thread-1", toolResultPart("tc-1", "search"));

    const s = session("thread-1")!;
    expect(s.liveParts.length).toBe(1);
    expect(s.liveParts[0].type).toBe("tool-invocation");
  });

  test("liveStats is null when no accumulator", () => {
    expect(session("thread-1")).toBeUndefined();
  });

  test("liveStats tracks pending vs completed tool counts", () => {
    startStream("thread-1");

    act().interpretStreamEvent("thread-1", toolCallPart("tc-1", "search"));
    act().interpretStreamEvent("thread-1", toolCallPart("tc-2", "profile"));

    let stats = session("thread-1")!.liveStats!;
    expect(stats.pendingToolCount).toBe(2);
    expect(stats.completedToolCount).toBe(0);
    expect(stats.toolCallCount).toBe(0);

    act().interpretStreamEvent("thread-1", toolResultPart("tc-1", "search"));

    stats = session("thread-1")!.liveStats!;
    expect(stats.pendingToolCount).toBe(1);
    expect(stats.completedToolCount).toBe(1);
    expect(stats.toolCallCount).toBe(1);
  });
});

// ============================================================================
// Edge cases
// ============================================================================

describe("edge cases", () => {
  test("reasoning parts tracked in liveStats", () => {
    startStream("thread-1");
    act().interpretStreamEvent("thread-1", reasoningPart("let me think"));

    const stats = session("thread-1")!.liveStats!;
    expect(stats.hasReasoning).toBe(true);
  });

  test("multiple text events concatenate in buffer", () => {
    startStream("thread-1");
    act().interpretStreamEvent("thread-1", textPart("a"));
    act().interpretStreamEvent("thread-1", textPart("b"));
    act().interpretStreamEvent("thread-1", textPart("c"));

    expect(session("thread-1")!.liveMessage!.textBuffer).toBe("abc");
  });

  test("reset clears all state", () => {
    act().setThreadId("thread-1");
    startStream("thread-1");
    act().interpretStreamEvent("thread-1", textPart("data"));

    act().reset();

    expect(state().threadId).toBeNull();
    expect(state().history).toEqual([]);
    expect(state().streamSessions.size).toBe(0);
    expect(state().loadPhase.phase).toBe("idle");
  });

  test("addUserMessage pushes to history", () => {
    const id = nextId();
    act().addUserMessage("Hello from user", id, NOW_ISO);

    expect(state().history.length).toBe(1);
    const msg = state().history[0];
    expect(msg.role).toBe("user");
    expect(msg.id).toBe(id);
    expect(msg.parts.length).toBe(1);
    expect(msg.parts[0].type).toBe("text");
    expect((msg.parts[0] as { text: string }).text).toBe("Hello from user");
  });

  test("setMessages replaces history and sets loadPhase to ready", () => {
    act().setMessages([mkMessage("m1", "assistant", "loaded")]);

    expect(state().history.length).toBe(1);
    expect(state().loadPhase.phase).toBe("ready");
  });

  test("clearHistory empties messages", () => {
    act().setMessages([mkMessage("m1", "user", "hi")]);
    act().clearHistory();

    expect(state().history).toEqual([]);
  });

  test("triggerFileChange increments counter", () => {
    expect(state().fileChangeCounters["thread-1"] ?? 0).toBe(0);
    act().triggerFileChange("thread-1");
    expect(state().fileChangeCounters["thread-1"]).toBe(1);
    act().triggerFileChange("thread-1");
    expect(state().fileChangeCounters["thread-1"]).toBe(2);
  });

  test("history cache evicts oldest when exceeding MAX_HISTORY_CACHE", () => {
    // MAX_HISTORY_CACHE is 5. Fill 6 threads to trigger eviction.
    for (let i = 1; i <= 6; i++) {
      act().setThreadId(`thread-${i}`);
      act().setMessages([mkMessage(`m${i}`, "user", `msg ${i}`)]);
    }
    // Switch to thread-7 to cache thread-6
    act().setThreadId("thread-7");

    // thread-1 should have been evicted (oldest)
    expect(state().historyCache.has("thread-1")).toBe(false);
    // thread-6 should still be cached (most recent)
    expect(state().historyCache.has("thread-6")).toBe(true);
    expect(state().historyCache.size).toBeLessThanOrEqual(5);
  });
});

// ============================================================================
// Generator DSL for Property-Based Tests
// ============================================================================

/** Mix of content and no-op StreamParts. */
const arbAnySP: fc.Arbitrary<StreamPart> = fc.oneof(
  arbTextSP,
  arbReasoningSP,
  arbErrorSP,
  arbNoOpSP
) as fc.Arbitrary<StreamPart>;

// ============================================================================
// Property-Based Tests (Interpreter Layer)
// ============================================================================

describe("streaming lifecycle (property-based)", () => {
  test("start → random events → finish always produces exactly 1 valid message", () => {
    fc.assert(
      fc.property(fc.array(arbAnySP, { maxLength: 50 }), (parts) => {
        state().reset();
        const threadId = "pbt-lifecycle";
        const msgId = MessageId("pbt-msg");

        act().setThreadId(threadId);
        act().startStreaming(threadId, new AbortController(), msgId, Date.now());

        for (const part of parts) {
          act().interpretStreamEvent(threadId, part);
        }

        act().finishStream(threadId, "2026-01-01T00:00:00Z");

        expect(state().history).toHaveLength(1);
        const msg = state().history[0];
        expect(msg.role).toBe("assistant");
        expect(msg.isStreaming).toBe(false);
        expect(msg.id).toBe(msgId);
      })
    );
  });

  test("start → random events → abort also produces exactly 1 message", () => {
    fc.assert(
      fc.property(fc.array(arbAnySP, { maxLength: 50 }), (parts) => {
        state().reset();
        const threadId = "pbt-abort";
        const msgId = MessageId("pbt-abort-msg");

        act().setThreadId(threadId);
        act().startStreaming(threadId, new AbortController(), msgId, Date.now());

        for (const part of parts) {
          act().interpretStreamEvent(threadId, part);
        }

        act().abortStream(threadId, "2026-01-01T00:00:00Z");

        expect(state().history).toHaveLength(1);
        const msg = state().history[0];
        expect(msg.role).toBe("assistant");
        expect(msg.isStreaming).toBe(false);
      })
    );
  });
});

describe("session isolation (property-based)", () => {
  test("events on one thread do not affect another thread's session", () => {
    fc.assert(
      fc.property(fc.array(arbAnySP, { maxLength: 30 }), (parts) => {
        state().reset();

        act().startStreaming("t-active", new AbortController(), MessageId("m1"), Date.now());
        act().startStreaming("t-idle", new AbortController(), MessageId("m2"), Date.now());

        const idleBefore = session("t-idle")!.liveMessage!;
        const idlePartsBefore = idleBefore.parts.length;
        const idleBufferBefore = idleBefore.textBuffer;

        for (const part of parts) {
          act().interpretStreamEvent("t-active", part);
        }

        const idleAfter = session("t-idle")!.liveMessage!;
        expect(idleAfter.parts.length).toBe(idlePartsBefore);
        expect(idleAfter.textBuffer).toBe(idleBufferBefore);
      })
    );
  });
});
