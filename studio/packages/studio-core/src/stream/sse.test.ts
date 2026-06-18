import { afterEach, describe, expect, mock, test } from "bun:test";
import { getApiConfig, setApiConfig } from "../api/client";
import {
  sseReplayBlocksFixture,
  sseTerminalBlockFixture,
} from "../test-fixtures/shared-contract-fixtures";
import { type JsonSSEHandlers, readJsonSSEStream, startJsonSSEStream } from "./sse";

interface TestStreamEvent {
  readonly type: string;
  readonly text?: string;
  readonly status?: string;
}

function createReaderFromBlocks(blocks: readonly string[]) {
  const encoder = new TextEncoder();
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const block of blocks) {
        controller.enqueue(encoder.encode(`${block}\n\n`));
      }
      controller.close();
    },
  });

  return stream.getReader();
}

function makeHandlers() {
  const events: TestStreamEvent[] = [];
  const eventIds: string[] = [];
  const errors: string[] = [];
  let completed = 0;

  const handlers: JsonSSEHandlers<TestStreamEvent> = {
    onEvent: (event) => {
      events.push(event);
    },
    onComplete: () => {
      completed += 1;
    },
    onError: (error) => {
      errors.push(error.message);
    },
    onEventId: (eventId) => {
      eventIds.push(eventId);
    },
  };

  return {
    handlers,
    events,
    eventIds,
    errors,
    get completed() {
      return completed;
    },
  };
}

const decodeEvent = (parsed: unknown): TestStreamEvent | null => {
  if (!parsed || typeof parsed !== "object") return null;
  const event = parsed as Record<string, unknown>;
  if (typeof event.type !== "string") return null;

  return {
    type: event.type,
    text: typeof event.text === "string" ? event.text : undefined,
    status: typeof event.status === "string" ? event.status : undefined,
  };
};

const originalFetch = globalThis.fetch;

afterEach(() => {
  mock.restore();
  globalThis.fetch = originalFetch;
});

describe("shared SSE stream contracts", () => {
  test("tracks event ids across replayable event blocks and completes on terminal payload", async () => {
    const tracker = makeHandlers();
    const reader = createReaderFromBlocks([sseReplayBlocksFixture[0], sseTerminalBlockFixture]);

    await readJsonSSEStream(
      reader,
      tracker.handlers,
      (event) => (event.type === "done" ? {} : null),
      undefined,
      decodeEvent
    );

    expect(tracker.eventIds).toEqual(["evt-1", "evt-terminal"]);
    expect(tracker.events).toEqual([
      { type: "text", text: "hello" },
      { type: "done", status: "completed" },
    ]);
    expect(tracker.errors).toEqual([]);
    expect(tracker.completed).toBe(1);
  });

  test("surfaces resync events as reconnect errors while preserving the event id", async () => {
    const tracker = makeHandlers();
    const reader = createReaderFromBlocks([sseReplayBlocksFixture[1]]);

    await readJsonSSEStream(reader, tracker.handlers, () => null, undefined, decodeEvent);

    expect(tracker.eventIds).toEqual(["evt-2"]);
    expect(tracker.events).toEqual([]);
    expect(tracker.errors).toEqual(["Stream lagged, please reconnect"]);
    expect(tracker.completed).toBe(0);
  });

  test("forwards Last-Event-ID and extra headers when starting a stream", async () => {
    const originalConfig = getApiConfig();
    const fetchCalls: Array<{ readonly url: string; readonly headers: Headers }> = [];
    const tracker = makeHandlers();

    setApiConfig({
      baseUrl: "https://example.test/api",
      headers: {
        "Content-Type": "application/json",
        "X-Workspace-Id": "workspace-1",
      },
    });

    const fetchMock = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      fetchCalls.push({
        url: String(input),
        headers: new Headers(init?.headers),
      });

      const reader = createReaderFromBlocks([sseTerminalBlockFixture]);
      const stream = new ReadableStream<Uint8Array>({
        async pull(controller) {
          const { done, value } = await reader.read();
          if (done) {
            controller.close();
            return;
          }
          controller.enqueue(value);
        },
      });

      return new Response(stream, {
        status: 200,
        headers: { "Content-Type": "text/event-stream" },
      });
    });

    globalThis.fetch = fetchMock as unknown as typeof fetch;

    try {
      const result = await startJsonSSEStream(
        "GET",
        "/stream",
        undefined,
        tracker.handlers,
        (event) => (event.type === "done" ? {} : null),
        {
          lastEventId: "evt-9",
          headers: { "X-Trace-Id": "trace-1" },
        },
        decodeEvent
      );

      expect(result._tag).toBe("Ok");
      expect(fetchCalls).toHaveLength(1);
      expect(fetchCalls[0]?.url).toBe("https://example.test/api/stream");
      expect(fetchCalls[0]?.headers.get("Last-Event-ID")).toBe("evt-9");
      expect(fetchCalls[0]?.headers.get("X-Trace-Id")).toBe("trace-1");
      expect(fetchCalls[0]?.headers.get("X-Workspace-Id")).toBe("workspace-1");
    } finally {
      setApiConfig(originalConfig);
    }
  });
});
