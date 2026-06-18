import type { ApiError } from "../domain/errors";
import type { Result } from "../domain/result";
import { isOk } from "../domain/result";

export type SSEConnectionStatus =
  | "idle"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "degraded"
  | "closed";

export interface SSEConnectionConfig<T> {
  readonly connect: (
    lastEventId: string | undefined,
    handlers: {
      onEvent: (event: T) => void;
      onComplete: () => void;
      onError: (error: ApiError) => void;
    }
  ) => Promise<Result<{ abort: () => void }, ApiError>>;
  readonly maxRetries: number;
  readonly baseDelayMs: number;
  readonly onEvent: (event: T) => void;
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
  readonly status: SSEConnectionStatus;
  connect(): void;
  disconnect(): void;
}

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
