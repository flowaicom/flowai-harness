import { describe, expect, test } from "bun:test";
import * as fc from "fast-check";

import { err, ok } from "~/lib/domain/result";
import type { ApiError } from "./client";
import {
  advanceSequence,
  createSSEConnection,
  type SSEConnectionStatus,
  type SSEStatusDetail,
} from "./sse-connection";

// ============================================================================
// advanceSequence
// ============================================================================

describe("advanceSequence", () => {
  test("advances when seq is greater than current", () => {
    expect(advanceSequence("3", 5)).toBe("5");
  });

  test("does not regress when seq is less than current", () => {
    expect(advanceSequence("10", 5)).toBe("10");
  });

  test("does not change on equal seq", () => {
    expect(advanceSequence("5", 5)).toBe("5");
  });

  test("advances from undefined", () => {
    expect(advanceSequence(undefined, 1)).toBe("1");
  });

  test("returns current when seq is undefined", () => {
    expect(advanceSequence("3", undefined)).toBe("3");
  });

  test("returns undefined when both are undefined", () => {
    expect(advanceSequence(undefined, undefined)).toBeUndefined();
  });

  test("ignores NaN seq", () => {
    expect(advanceSequence("3", NaN)).toBe("3");
  });

  test("ignores Infinity seq", () => {
    expect(advanceSequence("3", Infinity)).toBe("3");
  });
});

// ============================================================================
// createSSEConnection
// ============================================================================

describe("createSSEConnection", () => {
  function makeNetworkError(message = "connection refused"): ApiError {
    return { code: "NETWORK_ERROR", message };
  }

  test("status starts as idle", () => {
    const conn = createSSEConnection({
      connect: async () => ok({ abort: () => {} }),
      maxRetries: 3,
      baseDelayMs: 10,
      onEvent: () => {},
      onStatusChange: () => {},
    });
    expect(conn.status).toBe("idle");
  });

  test("transitions to connecting then connected on successful connect", async () => {
    const statuses: SSEConnectionStatus[] = [];
    let resolveConnect: (v: { abort: () => void }) => void = () => {};

    const conn = createSSEConnection({
      connect: async () => {
        return new Promise((resolve) => {
          resolveConnect = (v) => resolve(ok(v));
        });
      },
      maxRetries: 3,
      baseDelayMs: 10,
      onEvent: () => {},
      onStatusChange: (status) => {
        statuses.push(status);
      },
    });

    conn.connect();
    // Yield to let the async connect start
    await Promise.resolve();
    expect(statuses).toContain("connecting");

    resolveConnect({ abort: () => {} });
    await Promise.resolve();
    await Promise.resolve();

    expect(statuses).toContain("connected");
  });

  test("delivers events via onEvent callback", async () => {
    const events: string[] = [];
    let eventHandler: ((event: string) => void) | undefined;

    const conn = createSSEConnection<string>({
      connect: async (_lastEventId, handlers) => {
        eventHandler = handlers.onEvent;
        return ok({ abort: () => {} });
      },
      maxRetries: 3,
      baseDelayMs: 10,
      onEvent: (event) => events.push(event),
      onStatusChange: () => {},
    });

    conn.connect();
    await Promise.resolve();
    await Promise.resolve();

    eventHandler?.("hello");
    eventHandler?.("world");

    expect(events).toEqual(["hello", "world"]);
  });

  test("enters degraded on non-network error", async () => {
    const statuses: SSEConnectionStatus[] = [];
    const details: SSEStatusDetail[] = [];

    const conn = createSSEConnection({
      connect: async () => {
        return err({ code: "TIMEOUT" as const, message: "timed out" });
      },
      maxRetries: 3,
      baseDelayMs: 10,
      onEvent: () => {},
      onStatusChange: (status, detail) => {
        statuses.push(status);
        details.push(detail);
      },
    });

    conn.connect();
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();

    expect(statuses).toContain("degraded");
    const degradedDetail = details[details.length - 1];
    expect(degradedDetail?.message).toBe("timed out");
  });

  test("retries on network error up to maxRetries then degrades", async () => {
    const statuses: SSEConnectionStatus[] = [];
    let connectCount = 0;

    const conn = createSSEConnection({
      connect: async () => {
        connectCount++;
        return err(makeNetworkError());
      },
      maxRetries: 2,
      baseDelayMs: 1, // 1ms for fast tests
      onEvent: () => {},
      onStatusChange: (status) => {
        statuses.push(status);
      },
    });

    conn.connect();

    // Wait long enough for retries + backoff (1ms + 2ms)
    await new Promise((r) => setTimeout(r, 50));

    // 1 initial + 2 retries = 3 connect calls
    expect(connectCount).toBe(3);
    expect(statuses[statuses.length - 1]).toBe("degraded");
    expect(statuses).toContain("reconnecting");
  });

  test("disconnect cancels pending retries", async () => {
    let connectCount = 0;

    const conn = createSSEConnection({
      connect: async () => {
        connectCount++;
        return err(makeNetworkError());
      },
      maxRetries: 10,
      baseDelayMs: 100,
      onEvent: () => {},
      onStatusChange: () => {},
    });

    conn.connect();
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();

    const countBeforeDisconnect = connectCount;
    conn.disconnect();

    // Wait to see if more connects happen
    await new Promise((r) => setTimeout(r, 300));
    expect(connectCount).toBe(countBeforeDisconnect);
    expect(conn.status).toBe("closed");
  });

  test("disconnect calls abort on active connection", async () => {
    let aborted = false;

    const conn = createSSEConnection({
      connect: async () => {
        return ok({
          abort: () => {
            aborted = true;
          },
        });
      },
      maxRetries: 3,
      baseDelayMs: 10,
      onEvent: () => {},
      onStatusChange: () => {},
    });

    conn.connect();
    await Promise.resolve();
    await Promise.resolve();

    conn.disconnect();
    expect(aborted).toBe(true);
  });

  test("resets attempt counter on successful reconnect", async () => {
    let connectCount = 0;
    const statuses: SSEConnectionStatus[] = [];

    const conn = createSSEConnection({
      connect: async () => {
        connectCount++;
        // Fail first time, succeed second time
        if (connectCount === 1) {
          return err(makeNetworkError());
        }
        return ok({ abort: () => {} });
      },
      maxRetries: 3,
      baseDelayMs: 1,
      onEvent: () => {},
      onStatusChange: (status) => {
        statuses.push(status);
      },
    });

    conn.connect();
    await new Promise((r) => setTimeout(r, 50));

    expect(connectCount).toBe(2);
    expect(statuses).toContain("reconnecting");
    expect(statuses[statuses.length - 1]).toBe("connected");
  });

  test("onComplete handler transitions to idle", async () => {
    const statuses: SSEConnectionStatus[] = [];
    let completeHandler: (() => void) | undefined;

    const conn = createSSEConnection({
      connect: async (_lastEventId, handlers) => {
        completeHandler = handlers.onComplete;
        return ok({ abort: () => {} });
      },
      maxRetries: 3,
      baseDelayMs: 10,
      onEvent: () => {},
      onStatusChange: (status) => {
        statuses.push(status);
      },
    });

    conn.connect();
    await Promise.resolve();
    await Promise.resolve();

    expect(statuses).toContain("connected");

    completeHandler?.();
    expect(statuses[statuses.length - 1]).toBe("idle");
  });

  test("onError handler with NETWORK_ERROR triggers retry", async () => {
    const statuses: SSEConnectionStatus[] = [];
    let errorHandler: ((error: ApiError) => void) | undefined;
    let connectCount = 0;

    const conn = createSSEConnection({
      connect: async (_lastEventId, handlers) => {
        connectCount++;
        errorHandler = handlers.onError;
        return ok({ abort: () => {} });
      },
      maxRetries: 3,
      baseDelayMs: 1,
      onEvent: () => {},
      onStatusChange: (status) => {
        statuses.push(status);
      },
    });

    conn.connect();
    await Promise.resolve();
    await Promise.resolve();

    // Simulate a network error after connection is established
    errorHandler?.(makeNetworkError());
    await new Promise((r) => setTimeout(r, 50));

    // Should have reconnected
    expect(connectCount).toBeGreaterThan(1);
    expect(statuses).toContain("reconnecting");
  });
});

// ============================================================================
// Property-Based Tests — advanceSequence
// ============================================================================

describe("advanceSequence (property-based)", () => {
  test("monotonicity: result >= current (when both are finite numbers)", () => {
    fc.assert(
      fc.property(fc.nat(), fc.nat(), (current, seq) => {
        const result = advanceSequence(String(current), seq);
        expect(Number(result)).toBeGreaterThanOrEqual(current);
      })
    );
  });

  test("idempotence: advance(advance(c, s), s) = advance(c, s)", () => {
    fc.assert(
      fc.property(
        fc.option(fc.nat().map(String), { nil: undefined }),
        fc.option(fc.nat(), { nil: undefined }),
        (current, seq) => {
          const once = advanceSequence(current, seq);
          const twice = advanceSequence(once, seq);
          expect(twice).toBe(once);
        }
      )
    );
  });

  test("non-finite seq never changes current", () => {
    fc.assert(
      fc.property(
        fc.option(fc.nat().map(String), { nil: undefined }),
        fc.constantFrom(NaN, Infinity, -Infinity),
        (current, seq) => {
          expect(advanceSequence(current, seq)).toBe(current);
        }
      )
    );
  });

  test("undefined seq never changes current", () => {
    fc.assert(
      fc.property(fc.option(fc.nat().map(String), { nil: undefined }), (current) => {
        expect(advanceSequence(current, undefined)).toBe(current);
      })
    );
  });
});
