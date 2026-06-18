import { describe, expect, mock, test } from "bun:test";
import type { ApiError } from "./client";
import type { JsonSSEHandlers, TerminalCheck } from "./sse";
import { readJsonSSEStream } from "./sse";

// ============================================================================
// Test Helpers
// ============================================================================

/**
 * Build a mock ReadableStream from raw string chunks.
 *
 * Each string in `chunks` is enqueued as a separate Uint8Array read,
 * simulating how network data arrives in arbitrary-sized pieces.
 */
function mockReader(chunks: string[]): ReadableStreamDefaultReader<Uint8Array> {
  const encoder = new TextEncoder();
  let index = 0;
  return {
    read: async () => {
      if (index >= chunks.length) {
        return { done: true, value: undefined } as ReadableStreamReadResult<Uint8Array>;
      }
      const value = encoder.encode(chunks[index]);
      index++;
      return { done: false, value } as ReadableStreamReadValueResult<Uint8Array>;
    },
    cancel: mock(() => Promise.resolve()),
    releaseLock: () => {},
    closed: Promise.resolve(undefined),
  } as unknown as ReadableStreamDefaultReader<Uint8Array>;
}

/** Simple event type for tests. */
interface TestEvent {
  type: string;
  message?: string;
}

/** Terminal check: events with type "done" are success terminal, type "error" are error terminal. */
const testTerminalCheck: TerminalCheck<TestEvent> = (event) => {
  if (event.type === "done") return { error: undefined };
  if (event.type === "error") return { error: event.message ?? "unknown error" };
  return null;
};

/** Create a tracked handlers object for assertions. */
function createTrackedHandlers() {
  const events: TestEvent[] = [];
  const errors: ApiError[] = [];
  let completeCount = 0;
  let errorCount = 0;

  const handlers: JsonSSEHandlers<TestEvent> = {
    onEvent: (event) => events.push(event),
    onComplete: () => {
      completeCount++;
    },
    onError: (error) => {
      errorCount++;
      errors.push(error);
    },
  };

  return {
    handlers,
    events,
    errors,
    getCompleteCount: () => completeCount,
    getErrorCount: () => errorCount,
  };
}

// ============================================================================
// SSE Parsing
// ============================================================================

describe("SSE Parsing", () => {
  test("data: lines parsed correctly (single event)", async () => {
    const { handlers, events } = createTrackedHandlers();
    const reader = mockReader([
      'data: {"type":"text","message":"hello"}\n\n',
      'data: {"type":"done"}\n\n',
    ]);

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(events).toEqual([{ type: "text", message: "hello" }, { type: "done" }]);
  });

  test("multi-event buffers split on double newline", async () => {
    const { handlers, events } = createTrackedHandlers();
    // Two events arrive in a single chunk, separated by \n\n
    const reader = mockReader([
      'data: {"type":"text","message":"a"}\n\ndata: {"type":"text","message":"b"}\n\ndata: {"type":"done"}\n\n',
    ]);

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(events).toEqual([
      { type: "text", message: "a" },
      { type: "text", message: "b" },
      { type: "done" },
    ]);
  });

  test("malformed JSON lines handled gracefully (logged, skipped)", async () => {
    const { handlers, events, getCompleteCount } = createTrackedHandlers();
    const reader = mockReader([
      "data: not-valid-json\n\n",
      'data: {"type":"text","message":"ok"}\n\n',
      'data: {"type":"done"}\n\n',
    ]);

    // Malformed JSON should be skipped without crashing
    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(events).toEqual([{ type: "text", message: "ok" }, { type: "done" }]);
    expect(getCompleteCount()).toBe(1);
  });

  test("whitespace-only lines skipped", async () => {
    const { handlers, events } = createTrackedHandlers();
    const reader = mockReader([
      "   \n\n",
      "\n\n",
      'data: {"type":"text","message":"real"}\n\n',
      'data: {"type":"done"}\n\n',
    ]);

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(events).toEqual([{ type: "text", message: "real" }, { type: "done" }]);
  });

  test("non-data lines within an event block are ignored", async () => {
    const { handlers, events } = createTrackedHandlers();
    // SSE can have id:, event:, retry: lines — only data: should be parsed
    const reader = mockReader([
      'id: 42\nevent: message\ndata: {"type":"text","message":"hello"}\n\n',
      'data: {"type":"done"}\n\n',
    ]);

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(events).toEqual([{ type: "text", message: "hello" }, { type: "done" }]);
  });

  test("partial chunks across multiple reads reassembled correctly", async () => {
    const { handlers, events } = createTrackedHandlers();
    // Split a single SSE event across three reads
    const reader = mockReader([
      'data: {"type":"te',
      'xt","message":"split"}\n',
      '\ndata: {"type":"done"}\n\n',
    ]);

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(events).toEqual([{ type: "text", message: "split" }, { type: "done" }]);
  });

  test("partial chunks splitting double-newline delimiter", async () => {
    const { handlers, events } = createTrackedHandlers();
    // The \n\n delimiter is split across two reads
    const reader = mockReader([
      'data: {"type":"text","message":"first"}\n',
      '\ndata: {"type":"done"}\n\n',
    ]);

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(events).toEqual([{ type: "text", message: "first" }, { type: "done" }]);
  });
});

// ============================================================================
// Exactly-Once Settlement
// ============================================================================

describe("Exactly-Once Settlement", () => {
  test("terminal success event triggers onComplete exactly once", async () => {
    const { handlers, getCompleteCount, getErrorCount } = createTrackedHandlers();
    const reader = mockReader([
      'data: {"type":"text","message":"hi"}\n\n',
      'data: {"type":"done"}\n\n',
    ]);

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(getCompleteCount()).toBe(1);
    expect(getErrorCount()).toBe(0);
  });

  test("terminal error event triggers onError exactly once", async () => {
    const { handlers, getCompleteCount, getErrorCount, errors } = createTrackedHandlers();
    const reader = mockReader(['data: {"type":"error","message":"something broke"}\n\n']);

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(getErrorCount()).toBe(1);
    expect(getCompleteCount()).toBe(0);
    expect(errors[0].code).toBe("SERVER_ERROR");
    expect(errors[0].message).toBe("something broke");
  });

  test("after settlement, neither onComplete nor onError fire again", async () => {
    const { handlers, getCompleteCount, getErrorCount } = createTrackedHandlers();
    // Terminal event followed by more data — the post-terminal data should
    // never trigger a second settlement because readJsonSSEStream returns
    // after the terminal event.
    const reader = mockReader([
      'data: {"type":"done"}\n\n',
      'data: {"type":"error","message":"should not fire"}\n\n',
    ]);

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(getCompleteCount()).toBe(1);
    expect(getErrorCount()).toBe(0);
  });

  test("stream close without terminal event triggers NETWORK_ERROR", async () => {
    const { handlers, getCompleteCount, getErrorCount, errors } = createTrackedHandlers();
    const reader = mockReader([
      'data: {"type":"text","message":"partial"}\n\n',
      // Stream ends without a terminal event
    ]);

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(getErrorCount()).toBe(1);
    expect(getCompleteCount()).toBe(0);
    expect(errors[0].code).toBe("NETWORK_ERROR");
    expect(errors[0].message).toBe("Stream closed without completion event");
  });

  test("abort signal triggers settlement without error", async () => {
    const { handlers, getCompleteCount, getErrorCount } = createTrackedHandlers();
    const controller = new AbortController();
    // Abort immediately before reading starts
    controller.abort();

    const reader = mockReader(['data: {"type":"text","message":"never read"}\n\n']);

    await readJsonSSEStream(reader, handlers, testTerminalCheck, controller.signal);

    // Abort causes a clean settlement (onComplete), not onError
    expect(getCompleteCount()).toBe(1);
    expect(getErrorCount()).toBe(0);
  });

  test("error terminal and success terminal are mutually exclusive", async () => {
    // Verify that error terminal calls onError, not onComplete
    const { handlers, getCompleteCount, getErrorCount } = createTrackedHandlers();
    const reader = mockReader(['data: {"type":"error","message":"fail"}\n\n']);

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(getErrorCount()).toBe(1);
    expect(getCompleteCount()).toBe(0);
  });
});

// ============================================================================
// Abort Semantics
// ============================================================================

describe("Abort Semantics", () => {
  test("AbortSignal.abort() stops reading and calls reader.cancel", async () => {
    const { handlers } = createTrackedHandlers();
    const controller = new AbortController();

    // Create a reader that aborts after the first read
    const encoder = new TextEncoder();
    let readCount = 0;
    const reader = {
      read: async () => {
        readCount++;
        if (readCount === 1) {
          // First read returns data, then abort
          controller.abort();
          return {
            done: false,
            value: encoder.encode('data: {"type":"text","message":"first"}\n\n'),
          } as ReadableStreamReadResult<Uint8Array>;
        }
        // Should not be called after abort
        return { done: true, value: undefined } as ReadableStreamReadResult<Uint8Array>;
      },
      cancel: mock(() => Promise.resolve()),
      releaseLock: () => {},
      closed: Promise.resolve(undefined),
    } as unknown as ReadableStreamDefaultReader<Uint8Array>;

    await readJsonSSEStream(reader, handlers, testTerminalCheck, controller.signal);

    // reader.cancel should be called in the finally block
    expect(reader.cancel).toHaveBeenCalled();
  });

  test("cleanup happens in finally block even on throw", async () => {
    const { handlers } = createTrackedHandlers();

    const cancelMock = mock(() => Promise.resolve());
    const reader = {
      read: async () => {
        throw new Error("network failure");
      },
      cancel: cancelMock,
      releaseLock: () => {},
      closed: Promise.resolve(undefined),
    } as unknown as ReadableStreamDefaultReader<Uint8Array>;

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    // Even though read() threw, cancel should still be called in finally
    expect(cancelMock).toHaveBeenCalled();
  });

  test("no callbacks fire after abort for already-settled stream", async () => {
    let completeCount = 0;
    let errorCount = 0;
    const events: TestEvent[] = [];

    const controller = new AbortController();

    const handlers: JsonSSEHandlers<TestEvent> = {
      onEvent: (event) => events.push(event),
      onComplete: () => {
        completeCount++;
      },
      onError: () => {
        errorCount++;
      },
    };

    // Abort before anything happens
    controller.abort();

    const reader = mockReader([
      'data: {"type":"text","message":"should not appear"}\n\n',
      'data: {"type":"done"}\n\n',
    ]);

    await readJsonSSEStream(reader, handlers, testTerminalCheck, controller.signal);

    // When aborted before first read, no events should be dispatched
    expect(events.length).toBe(0);
    // Settlement should happen exactly once (onComplete for abort)
    expect(completeCount).toBe(1);
    expect(errorCount).toBe(0);
  });

  test("AbortError from reader triggers clean settlement", async () => {
    const { handlers, getCompleteCount, getErrorCount } = createTrackedHandlers();

    const reader = {
      read: async () => {
        throw new DOMException("The operation was aborted", "AbortError");
      },
      cancel: mock(() => Promise.resolve()),
      releaseLock: () => {},
      closed: Promise.resolve(undefined),
    } as unknown as ReadableStreamDefaultReader<Uint8Array>;

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(getCompleteCount()).toBe(1);
    expect(getErrorCount()).toBe(0);
  });

  test("non-AbortError from reader triggers error settlement", async () => {
    const { handlers, getCompleteCount, getErrorCount, errors } = createTrackedHandlers();

    const reader = {
      read: async () => {
        throw new TypeError("Failed to fetch");
      },
      cancel: mock(() => Promise.resolve()),
      releaseLock: () => {},
      closed: Promise.resolve(undefined),
    } as unknown as ReadableStreamDefaultReader<Uint8Array>;

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(getErrorCount()).toBe(1);
    expect(getCompleteCount()).toBe(0);
    expect(errors[0].code).toBe("NETWORK_ERROR");
    expect(errors[0].message).toBe("Failed to fetch");
  });
});

// ============================================================================
// Integration-level: Mock ReadableStream SSE Responses
// ============================================================================

describe("Integration: ReadableStream SSE", () => {
  function mockSSEResponse(lines: string[]): Response {
    const encoder = new TextEncoder();
    const stream = new ReadableStream({
      start(controller) {
        for (const line of lines) {
          controller.enqueue(encoder.encode(line));
        }
        controller.close();
      },
    });
    return new Response(stream, {
      headers: { "content-type": "text/event-stream" },
    });
  }

  test("full SSE response parsed through ReadableStream", async () => {
    const response = mockSSEResponse([
      'data: {"type":"text","message":"chunk1"}\n\n',
      'data: {"type":"text","message":"chunk2"}\n\n',
      'data: {"type":"done"}\n\n',
    ]);

    const { handlers, events, getCompleteCount, getErrorCount } = createTrackedHandlers();
    const reader = response.body!.getReader();

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(events).toEqual([
      { type: "text", message: "chunk1" },
      { type: "text", message: "chunk2" },
      { type: "done" },
    ]);
    expect(getCompleteCount()).toBe(1);
    expect(getErrorCount()).toBe(0);
  });

  test("mixed valid and malformed events in ReadableStream", async () => {
    const response = mockSSEResponse([
      'data: {"type":"text","message":"ok"}\n\n',
      "data: {broken json}\n\n",
      'data: {"type":"text","message":"also ok"}\n\n',
      'data: {"type":"done"}\n\n',
    ]);

    const { handlers, events, getCompleteCount } = createTrackedHandlers();
    const reader = response.body!.getReader();

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(events).toEqual([
      { type: "text", message: "ok" },
      { type: "text", message: "also ok" },
      { type: "done" },
    ]);
    expect(getCompleteCount()).toBe(1);
  });

  test("error terminal in ReadableStream triggers onError", async () => {
    const response = mockSSEResponse([
      'data: {"type":"text","message":"progress"}\n\n',
      'data: {"type":"error","message":"server crashed"}\n\n',
    ]);

    const { handlers, events, getErrorCount, errors } = createTrackedHandlers();
    const reader = response.body!.getReader();

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(events).toEqual([
      { type: "text", message: "progress" },
      { type: "error", message: "server crashed" },
    ]);
    expect(getErrorCount()).toBe(1);
    expect(errors[0].message).toBe("server crashed");
  });

  test("empty stream triggers NETWORK_ERROR (no terminal event)", async () => {
    const response = mockSSEResponse([]);

    const { handlers, getErrorCount, errors, getCompleteCount } = createTrackedHandlers();
    const reader = response.body!.getReader();

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(getErrorCount()).toBe(1);
    expect(getCompleteCount()).toBe(0);
    expect(errors[0].code).toBe("NETWORK_ERROR");
    expect(errors[0].message).toBe("Stream closed without completion event");
  });

  test("slow drip: each byte in separate chunk still assembles correctly", async () => {
    // Simulate byte-at-a-time delivery
    const fullMessage = 'data: {"type":"done"}\n\n';
    const encoder = new TextEncoder();
    const stream = new ReadableStream({
      start(controller) {
        for (const char of fullMessage) {
          controller.enqueue(encoder.encode(char));
        }
        controller.close();
      },
    });
    const response = new Response(stream, {
      headers: { "content-type": "text/event-stream" },
    });

    const { handlers, events, getCompleteCount } = createTrackedHandlers();
    const reader = response.body!.getReader();

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(events).toEqual([{ type: "done" }]);
    expect(getCompleteCount()).toBe(1);
  });

  test("multiple data: lines within one event block", async () => {
    // SSE spec allows multiple data: lines in a single event block.
    // Each data: line should be parsed independently.
    const response = mockSSEResponse([
      'data: {"type":"text","message":"line1"}\ndata: {"type":"text","message":"line2"}\n\n',
      'data: {"type":"done"}\n\n',
    ]);

    const { handlers, events, getCompleteCount } = createTrackedHandlers();
    const reader = response.body!.getReader();

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(events).toEqual([
      { type: "text", message: "line1" },
      { type: "text", message: "line2" },
      { type: "done" },
    ]);
    expect(getCompleteCount()).toBe(1);
  });
});

// ============================================================================
// Edge Cases
// ============================================================================

describe("Edge Cases", () => {
  test("event with no message field still parsed", async () => {
    const { handlers, events, getCompleteCount } = createTrackedHandlers();
    const reader = mockReader(['data: {"type":"done"}\n\n']);

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(events).toEqual([{ type: "done" }]);
    expect(getCompleteCount()).toBe(1);
  });

  test("deeply nested JSON in data: line", async () => {
    const { handlers, events } = createTrackedHandlers();
    const nested = JSON.stringify({
      type: "text",
      message: "nested",
      meta: { deep: { array: [1, 2, { x: true }] } },
    });
    const reader = mockReader([`data: ${nested}\n\n`, 'data: {"type":"done"}\n\n']);

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(events[0]).toEqual({
      type: "text",
      message: "nested",
      meta: { deep: { array: [1, 2, { x: true }] } },
    });
  });

  test("unicode in JSON data", async () => {
    const { handlers, events } = createTrackedHandlers();
    const reader = mockReader([
      'data: {"type":"text","message":"Hello \\u4e16\\u754c"}\n\n',
      'data: {"type":"done"}\n\n',
    ]);

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(events[0]).toEqual({ type: "text", message: "Hello \u4e16\u754c" });
  });

  test("reader.cancel failure in finally block does not throw", async () => {
    const { handlers, getCompleteCount } = createTrackedHandlers();

    const reader = {
      read: async () => {
        return { done: true, value: undefined } as ReadableStreamReadResult<Uint8Array>;
      },
      cancel: () => {
        throw new Error("cancel failed");
      },
      releaseLock: () => {},
      closed: Promise.resolve(undefined),
    } as unknown as ReadableStreamDefaultReader<Uint8Array>;

    // Should not throw even if cancel() throws
    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    // Settlement still happens (stream closed without terminal = NETWORK_ERROR)
    // The cancel error is swallowed by the catch in finally
  });

  test("concurrent settlement attempts are idempotent", async () => {
    // If both terminal event detection and stream-end happen to race,
    // only the first settle call should take effect
    let completeCount = 0;
    let errorCount = 0;

    const handlers: JsonSSEHandlers<TestEvent> = {
      onEvent: () => {},
      onComplete: () => {
        completeCount++;
      },
      onError: () => {
        errorCount++;
      },
    };

    // Terminal event is the last chunk, so the loop will settle on terminal
    // and then the stream ends — but settled guard prevents double-fire
    const reader = mockReader(['data: {"type":"done"}\n\n']);

    await readJsonSSEStream(reader, handlers, testTerminalCheck);

    expect(completeCount + errorCount).toBe(1);
  });
});
