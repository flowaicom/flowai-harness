/**
 * Pure SSE connection state machine.
 *
 * Separates the connect -> reconnect (exponential backoff) -> degrade lifecycle
 * from React effect scheduling. The caller (typically a React hook) drives
 * `connect()` / `disconnect()` at the appropriate lifecycle boundaries; this
 * module owns the retry loop, abort-controller management, and status
 * transitions.
 *
 * @module api/sse-connection
 */

import type { Result } from "~/lib/domain/result";
import { isOk } from "~/lib/domain/result";
import type { ApiError } from "./client";

// =============================================================================
// Types
// =============================================================================

export type SSEConnectionStatus =
  | "idle"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "degraded"
  | "closed";

export interface SSEConnectionConfig<T> {
  /** Build and start the underlying SSE stream for a given lastEventId. */
  readonly connect: (
    lastEventId: string | undefined,
    handlers: {
      onEvent: (event: T) => void;
      onComplete: () => void;
      onError: (error: ApiError) => void;
    }
  ) => Promise<Result<{ abort: () => void }, ApiError>>;

  /** Maximum number of reconnect attempts before entering degraded mode. */
  readonly maxRetries: number;

  /** Base delay (ms) for exponential backoff: delay = baseDelayMs * 2^(attempt-1). */
  readonly baseDelayMs: number;

  /** Called on every incoming stream event. */
  readonly onEvent: (event: T) => void;

  /** Called whenever the connection status changes. */
  readonly onStatusChange: (status: SSEConnectionStatus, detail: SSEStatusDetail) => void;
}

export interface SSEStatusDetail {
  readonly attempt: number | null;
  readonly delayMs: number | null;
  readonly startedAt: number | null;
  readonly lastEventId: string | null;
  readonly message: string | null;
}

export interface SSEConnection {
  /** Current connection status. */
  readonly status: SSEConnectionStatus;
  /** Start (or restart) the connection. Aborts any prior connection first. */
  connect(): void;
  /** Gracefully disconnect and clean up. */
  disconnect(): void;
}

// =============================================================================
// Factory
// =============================================================================

/**
 * Create a controllable SSE connection with built-in retry and degradation.
 *
 * The returned object is a plain description — no React dependencies. The
 * caller drives `connect()` / `disconnect()` from effect boundaries.
 */
export function createSSEConnection<T>(config: SSEConnectionConfig<T>): SSEConnection {
  let currentStatus: SSEConnectionStatus = "idle";
  let cancelled = false;
  let attempt = 0;
  let abortFn: (() => void) | null = null;
  let delayTimeoutId: ReturnType<typeof setTimeout> | undefined;
  let lastEventId: string | undefined;

  const setStatus = (status: SSEConnectionStatus, detail: Partial<SSEStatusDetail> = {}) => {
    currentStatus = status;
    config.onStatusChange(status, {
      attempt: detail.attempt ?? null,
      delayMs: detail.delayMs ?? null,
      startedAt: detail.startedAt ?? null,
      lastEventId: detail.lastEventId ?? lastEventId ?? null,
      message: detail.message ?? null,
    });
  };

  const onEvent = (event: T) => {
    config.onEvent(event);
  };

  const scheduleReconnect = () => {
    if (cancelled) return;
    if (attempt >= config.maxRetries) {
      setStatus("degraded", { attempt: attempt > 0 ? attempt : null });
      return;
    }
    attempt += 1;
    void tryConnect();
  };

  const tryConnect = async () => {
    if (cancelled) return;

    // Status transitions
    if (attempt === 0) {
      setStatus("connecting", { lastEventId: lastEventId ?? null });
    }

    if (attempt > 0) {
      const delayMs = config.baseDelayMs * 2 ** (attempt - 1);
      setStatus("reconnecting", {
        attempt,
        delayMs,
        startedAt: Date.now(),
        lastEventId: lastEventId ?? null,
      });
      await new Promise<void>((resolve) => {
        delayTimeoutId = setTimeout(resolve, delayMs);
      });
      if (cancelled) return;
    }

    const result = await config.connect(lastEventId, {
      onEvent,
      onComplete: () => {
        if (cancelled) return;
        abortFn = null;
        setStatus("idle");
      },
      onError: (error) => {
        if (cancelled) return;
        abortFn = null;
        if (error.code === "NETWORK_ERROR" && attempt < config.maxRetries) {
          scheduleReconnect();
          return;
        }
        setStatus("degraded", {
          attempt: attempt > 0 ? attempt : null,
          message: error.message,
          lastEventId: lastEventId ?? null,
        });
      },
    });

    if (cancelled) return;

    if (isOk(result)) {
      attempt = 0;
      setStatus("connected", { lastEventId: lastEventId ?? null });
      abortFn = result.value.abort;
      return;
    }

    // Connection setup itself failed
    if (result.error.code === "NETWORK_ERROR" && attempt < config.maxRetries) {
      scheduleReconnect();
      return;
    }

    setStatus("degraded", {
      attempt: attempt > 0 ? attempt : null,
      message: result.error.message,
      lastEventId: lastEventId ?? null,
    });
  };

  return {
    get status() {
      return currentStatus;
    },

    connect() {
      // Reset state for a fresh connection cycle
      cancelled = false;
      attempt = 0;
      abortFn?.();
      abortFn = null;
      if (delayTimeoutId !== undefined) clearTimeout(delayTimeoutId);
      void tryConnect();
    },

    disconnect() {
      cancelled = true;
      if (delayTimeoutId !== undefined) {
        clearTimeout(delayTimeoutId);
        delayTimeoutId = undefined;
      }
      abortFn?.();
      abortFn = null;
      currentStatus = "closed";
    },
  };
}

/**
 * Advance the high-water mark for event sequence numbers.
 *
 * Returns the updated lastEventId. Only moves forward — out-of-order events
 * cannot regress the sequence, preventing duplicate re-requests on reconnect.
 */
export function advanceSequence(
  current: string | undefined,
  seq: number | undefined
): string | undefined {
  if (typeof seq !== "number" || !Number.isFinite(seq)) return current;
  const prev = current != null ? Number.parseInt(current, 10) : 0;
  if (Number.isFinite(prev) && seq > prev) {
    return String(seq);
  }
  return current;
}
