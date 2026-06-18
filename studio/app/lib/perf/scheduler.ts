/**
 * Scheduler utilities for deferring non-critical work.
 *
 * Uses requestIdleCallback for background tasks and
 * requestAnimationFrame for visual updates.
 *
 * @module lib/perf/scheduler
 */

// ============================================================================
// Polyfills for SSR/older browsers
// ============================================================================

type IdleDeadline = {
  didTimeout: boolean;
  timeRemaining: () => number;
};

type IdleCallbackHandle = number;

const hasIdleCallback = typeof window !== "undefined" && "requestIdleCallback" in window;
const hasRAF = typeof window !== "undefined" && "requestAnimationFrame" in window;

/**
 * Request idle callback with fallback to setTimeout.
 */
export function requestIdle(
  callback: (deadline: IdleDeadline) => void,
  options?: { timeout?: number }
): IdleCallbackHandle {
  if (hasIdleCallback) {
    return window.requestIdleCallback(callback, options);
  }
  // Fallback: simulate with setTimeout
  const start = Date.now();
  return window.setTimeout(() => {
    callback({
      didTimeout: false,
      timeRemaining: () => Math.max(0, 50 - (Date.now() - start)),
    });
  }, 1) as unknown as IdleCallbackHandle;
}

/**
 * Cancel idle callback.
 */
export function cancelIdle(handle: IdleCallbackHandle): void {
  if (hasIdleCallback) {
    window.cancelIdleCallback(handle);
  } else {
    window.clearTimeout(handle);
  }
}

/**
 * Request animation frame with fallback.
 */
export function requestFrame(callback: FrameRequestCallback): number {
  if (hasRAF) {
    return window.requestAnimationFrame(callback);
  }
  return window.setTimeout(() => callback(Date.now()), 16) as unknown as number;
}

/**
 * Cancel animation frame.
 */
export function cancelFrame(handle: number): void {
  if (hasRAF) {
    window.cancelAnimationFrame(handle);
  } else {
    window.clearTimeout(handle);
  }
}

// ============================================================================
// Task Queue for batching deferred work
// ============================================================================

type DeferredTask = {
  id: string;
  fn: () => void;
  priority: "high" | "normal" | "low";
};

const taskQueue: DeferredTask[] = [];
let isProcessing = false;
let idleHandle: IdleCallbackHandle | null = null;

/**
 * Schedule a task to run during idle time.
 * Tasks are deduplicated by ID - only the latest task with a given ID runs.
 */
export function scheduleIdle(
  id: string,
  fn: () => void,
  priority: "high" | "normal" | "low" = "normal"
): void {
  // Remove existing task with same ID (deduplication)
  const existingIndex = taskQueue.findIndex((t) => t.id === id);
  if (existingIndex !== -1) {
    taskQueue.splice(existingIndex, 1);
  }

  // Add task with priority sorting
  const task: DeferredTask = { id, fn, priority };
  const priorityOrder = { high: 0, normal: 1, low: 2 };

  // Insert in priority order
  let inserted = false;
  for (let i = 0; i < taskQueue.length; i++) {
    if (priorityOrder[priority] < priorityOrder[taskQueue[i].priority]) {
      taskQueue.splice(i, 0, task);
      inserted = true;
      break;
    }
  }
  if (!inserted) {
    taskQueue.push(task);
  }

  // Start processing if not already
  if (!isProcessing && idleHandle === null) {
    idleHandle = requestIdle(processTasks, { timeout: 100 });
  }
}

function processTasks(deadline: IdleDeadline): void {
  isProcessing = true;
  idleHandle = null;

  // Process tasks while we have time or until queue is empty
  while (taskQueue.length > 0 && (deadline.timeRemaining() > 0 || deadline.didTimeout)) {
    const task = taskQueue.shift();
    if (task) {
      try {
        task.fn();
      } catch (error) {
        console.error(`[Scheduler] Task ${task.id} failed:`, error);
      }
    }
  }

  isProcessing = false;

  // Schedule next batch if tasks remain
  if (taskQueue.length > 0) {
    idleHandle = requestIdle(processTasks, { timeout: 100 });
  }
}

/**
 * Cancel a scheduled idle task by ID.
 */
export function cancelIdleTask(id: string): void {
  const index = taskQueue.findIndex((t) => t.id === id);
  if (index !== -1) {
    taskQueue.splice(index, 1);
  }
}

/**
 * Clear all pending idle tasks.
 */
export function clearIdleTasks(): void {
  taskQueue.length = 0;
  if (idleHandle !== null) {
    cancelIdle(idleHandle);
    idleHandle = null;
  }
  isProcessing = false;
}

// ============================================================================
// RAF Throttling
// ============================================================================

const rafCallbacks = new Map<string, number>();

/**
 * RAF-throttled function execution.
 * Only one execution per ID per animation frame.
 */
export function rafThrottle(id: string, fn: () => void): void {
  if (rafCallbacks.has(id)) {
    return; // Already scheduled
  }

  const handle = requestFrame(() => {
    rafCallbacks.delete(id);
    fn();
  });

  rafCallbacks.set(id, handle);
}

/**
 * Cancel RAF-throttled execution.
 */
export function cancelRafThrottle(id: string): void {
  const handle = rafCallbacks.get(id);
  if (handle !== undefined) {
    cancelFrame(handle);
    rafCallbacks.delete(id);
  }
}

// ============================================================================
// Debouncing
// ============================================================================

type DebouncedFunction<T extends (...args: unknown[]) => unknown> = {
  (...args: Parameters<T>): void;
  cancel: () => void;
  flush: () => void;
};

interface DebounceOptions {
  /** Execute on the leading edge (default: false) */
  leading?: boolean;
  /** Execute on the trailing edge (default: true) */
  trailing?: boolean;
  /** Maximum time to wait before forcing execution (optional) */
  maxWait?: number;
}

const debounceTimers = new Map<string, ReturnType<typeof setTimeout>>();
const debounceMaxWaitTimers = new Map<string, ReturnType<typeof setTimeout>>();
const debouncePending = new Map<string, { fn: () => void; args: unknown[] }>();

/**
 * Create a debounced function that delays invoking func until after wait ms
 * have elapsed since the last invocation.
 *
 * Supports both leading and trailing edge execution, with optional maxWait.
 *
 * @param id - Unique identifier for this debounced function (enables cancellation)
 * @param fn - Function to debounce
 * @param wait - Milliseconds to wait
 * @param options - Debounce options
 * @returns Debounced function with cancel and flush methods
 *
 * @example
 * ```ts
 * // Basic trailing debounce
 * const handleSearch = debounce('search', (query: string) => {
 *   fetch(`/api/search?q=${query}`);
 * }, 300);
 *
 * // Leading edge (fire immediately, then wait)
 * const handleClick = debounce('click', onClick, 300, { leading: true, trailing: false });
 *
 * // With max wait (guaranteed execution within maxWait)
 * const handleScroll = debounce('scroll', onScroll, 100, { maxWait: 500 });
 * ```
 */
export function debounce<T extends (...args: unknown[]) => unknown>(
  id: string,
  fn: T,
  wait: number,
  options: DebounceOptions = {}
): DebouncedFunction<T> {
  const { leading = false, trailing = true, maxWait } = options;
  let leadingInvoked = false;

  const invokeFunc = (args: unknown[]) => {
    leadingInvoked = false;
    debouncePending.delete(id);
    fn(...args);
  };

  const debounced = (...args: Parameters<T>) => {
    // Store pending call info
    debouncePending.set(id, { fn: () => fn(...args), args });

    // Leading edge
    if (leading && !leadingInvoked) {
      leadingInvoked = true;
      invokeFunc(args);

      // If no trailing, we're done after leading
      if (!trailing) {
        return;
      }
    }

    // Clear existing timer
    const existingTimer = debounceTimers.get(id);
    if (existingTimer !== undefined) {
      clearTimeout(existingTimer);
    }

    // Set up trailing edge timer
    if (trailing) {
      const timer = setTimeout(() => {
        debounceTimers.delete(id);

        // Clear maxWait timer if exists
        const maxWaitTimer = debounceMaxWaitTimers.get(id);
        if (maxWaitTimer !== undefined) {
          clearTimeout(maxWaitTimer);
          debounceMaxWaitTimers.delete(id);
        }

        invokeFunc(args);
      }, wait);
      debounceTimers.set(id, timer);
    }

    // Set up maxWait timer if specified and not already set
    if (maxWait !== undefined && !debounceMaxWaitTimers.has(id)) {
      const maxWaitTimer = setTimeout(() => {
        debounceMaxWaitTimers.delete(id);

        // Clear regular timer
        const regularTimer = debounceTimers.get(id);
        if (regularTimer !== undefined) {
          clearTimeout(regularTimer);
          debounceTimers.delete(id);
        }

        const pending = debouncePending.get(id);
        if (pending) {
          invokeFunc(pending.args);
        }
      }, maxWait);
      debounceMaxWaitTimers.set(id, maxWaitTimer);
    }
  };

  debounced.cancel = () => {
    const timer = debounceTimers.get(id);
    if (timer !== undefined) {
      clearTimeout(timer);
      debounceTimers.delete(id);
    }

    const maxWaitTimer = debounceMaxWaitTimers.get(id);
    if (maxWaitTimer !== undefined) {
      clearTimeout(maxWaitTimer);
      debounceMaxWaitTimers.delete(id);
    }

    debouncePending.delete(id);
    leadingInvoked = false;
  };

  debounced.flush = () => {
    const timer = debounceTimers.get(id);
    if (timer !== undefined) {
      clearTimeout(timer);
      debounceTimers.delete(id);
    }

    const maxWaitTimer = debounceMaxWaitTimers.get(id);
    if (maxWaitTimer !== undefined) {
      clearTimeout(maxWaitTimer);
      debounceMaxWaitTimers.delete(id);
    }

    const pending = debouncePending.get(id);
    if (pending) {
      invokeFunc(pending.args);
    }
  };

  return debounced as DebouncedFunction<T>;
}

/**
 * Cancel a debounced function by ID.
 */
export function cancelDebounce(id: string): void {
  const timer = debounceTimers.get(id);
  if (timer !== undefined) {
    clearTimeout(timer);
    debounceTimers.delete(id);
  }

  const maxWaitTimer = debounceMaxWaitTimers.get(id);
  if (maxWaitTimer !== undefined) {
    clearTimeout(maxWaitTimer);
    debounceMaxWaitTimers.delete(id);
  }

  debouncePending.delete(id);
}

/**
 * Clear all debounce timers.
 */
export function clearAllDebounce(): void {
  for (const timer of debounceTimers.values()) {
    clearTimeout(timer);
  }
  debounceTimers.clear();

  for (const timer of debounceMaxWaitTimers.values()) {
    clearTimeout(timer);
  }
  debounceMaxWaitTimers.clear();

  debouncePending.clear();
}

// ============================================================================
// Microtask scheduling
// ============================================================================

/**
 * Schedule a microtask (runs before next paint).
 */
export function scheduleMicrotask(fn: () => void): void {
  if (typeof queueMicrotask === "function") {
    queueMicrotask(fn);
  } else {
    Promise.resolve().then(fn);
  }
}
