/**
 * Web Worker interface for message transformation.
 *
 * Offloads expensive message parsing/transformation to a separate thread,
 * keeping the main thread free for UI updates during streaming.
 *
 * @module lib/perf/message-worker
 */

import type { Message } from "~/lib/domain/message";

// ============================================================================
// Worker Protocol Types
// ============================================================================

export type WorkerRequest = {
  id: string;
  type: "transform";
  payload: {
    messages: unknown[];
    format: "ui" | "backend";
  };
};

export type WorkerResponse = {
  id: string;
  type: "result" | "error" | "ready";
  payload: Message[] | string | null;
};

// ============================================================================
// Worker Instance Management
// ============================================================================

let workerInstance: Worker | null = null;
let workerReady = false;
let requestCounter = 0;

const pendingRequests = new Map<
  string,
  {
    resolve: (messages: Message[]) => void;
    reject: (error: Error) => void;
  }
>();

/**
 * Check if Web Workers are available in this environment.
 */
function workersAvailable(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof Worker !== "undefined" &&
    !import.meta.env.VITE_DISABLE_WORKERS
  );
}

/**
 * Get or create the worker instance.
 */
function getWorker(): Worker | null {
  if (!workersAvailable()) {
    return null;
  }

  if (workerInstance) {
    return workerInstance;
  }

  try {
    // Create worker from inline code (avoids separate file bundling issues)
    // Worker includes deduplication logic for off-main-thread processing
    const workerCode = `
      // Message deduplication utilities (inline for worker)
      function getBaseMessageId(id) {
        const splitIndex = id.indexOf('__split-');
        return splitIndex === -1 ? id : id.substring(0, splitIndex);
      }

      function deduplicateMessages(messages) {
        if (messages.length <= 1) return messages;

        const idSet = new Set();
        let hasDuplicates = false;

        for (const msg of messages) {
          const baseId = getBaseMessageId(msg.id);
          if (idSet.has(baseId)) {
            hasDuplicates = true;
            break;
          }
          idSet.add(baseId);
        }

        if (!hasDuplicates) return messages;

        const lastOccurrence = new Map();
        for (let i = 0; i < messages.length; i++) {
          const baseId = getBaseMessageId(messages[i].id);
          lastOccurrence.set(baseId, i);
        }

        return messages.filter((msg, index) => {
          const baseId = getBaseMessageId(msg.id);
          return lastOccurrence.get(baseId) === index;
        });
      }

      function sortMessagesByTimestamp(messages) {
        if (messages.length <= 1) return messages;

        let isSorted = true;
        for (let i = 1; i < messages.length; i++) {
          const prevTime = new Date(messages[i - 1].createdAt).getTime();
          const currTime = new Date(messages[i].createdAt).getTime();
          if (prevTime > currTime) {
            isSorted = false;
            break;
          }
        }

        if (isSorted) return messages;

        return [...messages].sort((a, b) => {
          const timeA = new Date(a.createdAt).getTime();
          const timeB = new Date(b.createdAt).getTime();
          return timeA - timeB;
        });
      }

      self.onmessage = function(e) {
        const { id, type, payload } = e.data;

        if (type === 'transform') {
          try {
            // Apply deduplication and sorting
            let result = deduplicateMessages(payload.messages);
            result = sortMessagesByTimestamp(result);
            self.postMessage({ id, type: 'result', payload: result });
          } catch (error) {
            self.postMessage({ id, type: 'error', payload: error.message });
          }
        }
      };

      // Signal ready
      self.postMessage({ id: 'init', type: 'ready', payload: null });
    `;

    const blob = new Blob([workerCode], { type: "application/javascript" });
    const url = URL.createObjectURL(blob);

    workerInstance = new Worker(url);

    workerInstance.onmessage = (e: MessageEvent<WorkerResponse>) => {
      const { id, type, payload } = e.data;

      if (id === "init" && type === "ready") {
        workerReady = true;
        return;
      }

      const pending = pendingRequests.get(id);
      if (!pending) return;

      pendingRequests.delete(id);

      if (type === "error") {
        pending.reject(new Error(payload as string));
      } else {
        pending.resolve(payload as Message[]);
      }
    };

    workerInstance.onerror = (e) => {
      console.error("[Worker] Error:", e);
      // Reject all pending requests
      for (const [id, { reject }] of pendingRequests) {
        reject(new Error("Worker error"));
        pendingRequests.delete(id);
      }
    };

    // Clean up URL after worker is created
    URL.revokeObjectURL(url);

    return workerInstance;
  } catch (error) {
    console.warn("[Worker] Failed to create worker:", error);
    return null;
  }
}

/**
 * Transform messages asynchronously using Web Worker.
 * Falls back to synchronous transformation if workers unavailable.
 */
export async function transformMessagesAsync(
  messages: unknown[],
  format: "ui" | "backend",
  signal?: AbortSignal
): Promise<Message[]> {
  const worker = getWorker();

  if (worker && workerReady) {
    const requestId = `req-${++requestCounter}`;

    return new Promise<Message[]>((resolve, reject) => {
      // Handle abort signal
      if (signal?.aborted) {
        reject(new DOMException("Aborted", "AbortError"));
        return;
      }

      const abortHandler = () => {
        pendingRequests.delete(requestId);
        reject(new DOMException("Aborted", "AbortError"));
      };

      signal?.addEventListener("abort", abortHandler, { once: true });

      pendingRequests.set(requestId, {
        resolve: (result) => {
          signal?.removeEventListener("abort", abortHandler);
          resolve(result);
        },
        reject: (error) => {
          signal?.removeEventListener("abort", abortHandler);
          reject(error);
        },
      });

      const request: WorkerRequest = {
        id: requestId,
        type: "transform",
        payload: { messages, format },
      };

      worker.postMessage(request);
    });
  }

  // Fallback: return messages as-is (actual transformation in domain layer)
  return messages as Message[];
}

/**
 * Terminate the worker and clean up.
 */
export function terminateWorker(): void {
  if (workerInstance) {
    workerInstance.terminate();
    workerInstance = null;
    workerReady = false;
    pendingRequests.clear();
  }
}

/**
 * Check if worker is ready.
 */
export function isWorkerReady(): boolean {
  return workerReady;
}
