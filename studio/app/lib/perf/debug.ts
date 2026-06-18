/**
 * Debug logging utilities with production optimization (P28-P32).
 *
 * Principles:
 * - Zero overhead in production (function calls become no-ops)
 * - Namespace-based filtering for targeted debugging
 * - Worker-safe (no DOM dependencies)
 * - SSR-safe (handles typeof window checks)
 *
 * @module lib/perf/debug
 */

// ============================================================================
// Configuration
// ============================================================================

/**
 * Debug namespace configuration.
 * Set via env variable: VITE_DEBUG="namespace1,namespace2,*"
 * Use "*" to enable all namespaces.
 */
const DEBUG_ENV =
  typeof import.meta !== "undefined" && import.meta.env?.DEV
    ? (import.meta.env?.VITE_DEBUG ?? "")
    : "";

const DEBUG_NAMESPACES = new Set(DEBUG_ENV.split(",").filter(Boolean));
const DEBUG_ALL = DEBUG_NAMESPACES.has("*");

/**
 * Check if debug mode is enabled for any namespace.
 */
export const isDebugMode =
  typeof import.meta !== "undefined" && import.meta.env?.DEV && DEBUG_NAMESPACES.size > 0;

// ============================================================================
// Namespace Checking
// ============================================================================

/**
 * Check if a specific namespace is enabled for debugging.
 *
 * @param namespace - The namespace to check
 * @returns True if debugging is enabled for this namespace
 */
export function isDebugEnabled(namespace: string): boolean {
  if (typeof import.meta === "undefined" || !import.meta.env?.DEV) {
    return false;
  }
  return DEBUG_ALL || DEBUG_NAMESPACES.has(namespace);
}

// ============================================================================
// Debug Logging Functions
// ============================================================================

type LogFn = (label: string, data?: unknown) => void;

/**
 * Create a namespaced debug logger.
 *
 * In production: Returns a no-op function (zero overhead).
 * In development: Returns a colored console logger if namespace is enabled.
 *
 * @param namespace - The namespace for this logger
 * @param color - CSS color for console output (default: #9333ea)
 * @returns A logging function
 *
 * @example
 * ```ts
 * const debug = createDebugLogger("MessageTransformer", "#9333ea");
 * debug("Deduplicating", { count: 100 }); // Only logs if VITE_DEBUG includes "MessageTransformer"
 * ```
 */
export function createDebugLogger(namespace: string, color = "#9333ea"): LogFn {
  // Production: return no-op immediately
  if (typeof import.meta === "undefined" || !import.meta.env?.DEV) {
    return () => {};
  }

  // Check if this namespace is enabled
  if (!DEBUG_ALL && !DEBUG_NAMESPACES.has(namespace)) {
    return () => {};
  }

  // Development with enabled namespace: return actual logger
  return (label: string, data?: unknown): void => {
    if (data !== undefined) {
      console.log(`%c[${namespace}] ${label}`, `color: ${color}`, data);
    } else {
      console.log(`%c[${namespace}] ${label}`, `color: ${color}`);
    }
  };
}

/**
 * Create a debug timer for measuring performance.
 *
 * In production: Returns a no-op timer.
 * In development: Returns a timer that logs duration when ended.
 *
 * @param namespace - The namespace for this timer
 * @param label - Label for the timing measurement
 * @returns A function that, when called, ends the timer and logs the duration
 *
 * @example
 * ```ts
 * const endTimer = createDebugTimer("Transformer", "deduplication");
 * // ... expensive operation ...
 * endTimer(); // Logs: "[Transformer] deduplication: 45.2ms"
 * ```
 */
export function createDebugTimer(namespace: string, label: string): () => void {
  // Production: return no-op
  if (typeof import.meta === "undefined" || !import.meta.env?.DEV) {
    return () => {};
  }

  if (!DEBUG_ALL && !DEBUG_NAMESPACES.has(namespace)) {
    return () => {};
  }

  const start = performance.now();
  return () => {
    const duration = performance.now() - start;
    console.log(`%c[${namespace}] ${label}: ${duration.toFixed(2)}ms`, "color: #f59e0b");
  };
}

// ============================================================================
// Pre-configured Loggers
// ============================================================================

/**
 * Pre-configured debug loggers for common namespaces.
 * Usage: debugLoggers.transformer("label", data)
 */
export const debugLoggers = {
  /** Message transformer debugging */
  transformer: createDebugLogger("MessageTransformer", "#9333ea"),
  /** Store actions debugging */
  store: createDebugLogger("Store", "#3b82f6"),
  /** Streaming debugging */
  streaming: createDebugLogger("Streaming", "#22c55e"),
  /** Latency tracking debugging */
  latency: createDebugLogger("Latency", "#f59e0b"),
  /** Worker debugging */
  worker: createDebugLogger("Worker", "#ec4899"),
  /** Scheduler debugging */
  scheduler: createDebugLogger("Scheduler", "#8b5cf6"),
} as const;

// ============================================================================
// Conditional Execution
// ============================================================================

/**
 * Execute a function only in debug mode for a specific namespace.
 *
 * Useful for expensive debug-only computations.
 *
 * @param namespace - The namespace to check
 * @param fn - Function to execute if debugging is enabled
 *
 * @example
 * ```ts
 * debugOnly("Transformer", () => {
 *   // This expensive validation only runs when debugging
 *   validateMessageIntegrity(messages);
 * });
 * ```
 */
export function debugOnly(namespace: string, fn: () => void): void {
  if (isDebugEnabled(namespace)) {
    fn();
  }
}

/**
 * Execute a function only in development mode.
 *
 * @param fn - Function to execute in development
 */
export function devOnly(fn: () => void): void {
  if (typeof import.meta !== "undefined" && import.meta.env?.DEV) {
    fn();
  }
}
