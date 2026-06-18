/**
 * Session Heartbeat Hook
 *
 * Keeps the session alive by periodically pinging the backend.
 * Provides session expiry warnings using a sliding window pattern.
 *
 * Design principles:
 * - Pure state machine (active → warning → expired)
 * - Effects at edges (useEffect hooks)
 * - Configurable and testable
 *
 * @module lib/hooks/use-session-heartbeat
 */

import { useCallback, useEffect, useRef, useState } from "react";

// ============================================================================
// Configuration
// ============================================================================

/** Heartbeat ping interval: 5 minutes */
const HEARTBEAT_INTERVAL_MS = 5 * 60 * 1000;

/** Warning threshold: show warning when < 10 minutes remain */
const WARNING_THRESHOLD_MS = 10 * 60 * 1000;

/** Default session timeout: 30 minutes (sliding window) */
const DEFAULT_SESSION_TIMEOUT_MS = 30 * 60 * 1000;

/** Countdown update interval: every second when in warning zone */
const COUNTDOWN_INTERVAL_MS = 1000;

// ============================================================================
// Types
// ============================================================================

/**
 * Session state discriminated union.
 * Represents the possible states of the session.
 */
export type SessionStatus = "active" | "warning" | "expired" | "unknown";

/**
 * Session state returned by the hook.
 */
export interface SessionState {
  /** Current session status */
  status: SessionStatus;
  /** Time remaining in session (ms), null if unknown */
  remainingMs: number | null;
  /** Whether session is in warning zone (<10 min remaining) */
  isWarning: boolean;
  /** Whether session has expired */
  isExpired: boolean;
  /** Session expiry timestamp (Unix ms), null if unknown */
  expiresAt: number | null;
  /** Last successful ping timestamp */
  lastPingAt: number | null;
  /** Force a heartbeat ping (returns success boolean) */
  ping: () => Promise<boolean>;
  /** Reset session state (call after re-authentication) */
  reset: () => void;
}

/**
 * Hook options for customization.
 */
export interface UseSessionHeartbeatOptions {
  /** Custom ping endpoint (defaults to /api/health) */
  pingEndpoint?: string;
  /** Session timeout in ms (defaults to 30 min) */
  sessionTimeoutMs?: number;
  /** Warning threshold in ms (defaults to 10 min) */
  warningThresholdMs?: number;
  /** Heartbeat interval in ms (defaults to 5 min) */
  heartbeatIntervalMs?: number;
  /** Whether to start in enabled state (defaults to true) */
  enabled?: boolean;
  /** Callback when session expires */
  onExpire?: () => void;
  /** Callback when session enters warning zone */
  onWarning?: (remainingMs: number) => void;
}

// ============================================================================
// Hook Implementation
// ============================================================================

/**
 * Hook that maintains session heartbeat and tracks expiry.
 *
 * Uses a sliding window pattern where each successful ping extends
 * the session expiry by the configured timeout duration.
 *
 * @param options - Configuration options
 * @returns SessionState with expiry info and manual ping function
 *
 * @example
 * ```tsx
 * function SessionBanner() {
 *   const { isWarning, isExpired, remainingMs } = useSessionHeartbeat();
 *
 *   if (isExpired) {
 *     return <SessionExpiredDialog onRefresh={() => window.location.reload()} />;
 *   }
 *
 *   if (isWarning && remainingMs !== null) {
 *     return <Banner>Session expires in {formatRemainingTime(remainingMs)}</Banner>;
 *   }
 *
 *   return null;
 * }
 * ```
 */
export function useSessionHeartbeat(options: UseSessionHeartbeatOptions = {}): SessionState {
  const {
    pingEndpoint = "/api/health",
    sessionTimeoutMs = DEFAULT_SESSION_TIMEOUT_MS,
    warningThresholdMs = WARNING_THRESHOLD_MS,
    heartbeatIntervalMs = HEARTBEAT_INTERVAL_MS,
    enabled = true,
    onExpire,
    onWarning,
  } = options;

  // State
  const [expiresAt, setExpiresAt] = useState<number | null>(null);
  const [lastPingAt, setLastPingAt] = useState<number | null>(null);
  const [isExpired, setIsExpired] = useState(false);
  const [remainingMs, setRemainingMs] = useState<number | null>(null);

  // Refs to avoid stale closures in intervals
  const expiresAtRef = useRef<number | null>(null);
  const onExpireRef = useRef(onExpire);
  const onWarningRef = useRef(onWarning);
  const hasWarningFired = useRef(false);

  // Keep refs in sync
  onExpireRef.current = onExpire;
  onWarningRef.current = onWarning;

  // Ping the backend to refresh session
  const ping = useCallback(async (): Promise<boolean> => {
    try {
      const response = await fetch(pingEndpoint, {
        credentials: "include",
        headers: {
          "Cache-Control": "no-cache",
        },
      });

      if (response.ok) {
        const now = Date.now();
        const newExpiry = now + sessionTimeoutMs;

        setExpiresAt(newExpiry);
        setLastPingAt(now);
        setIsExpired(false);
        expiresAtRef.current = newExpiry;
        hasWarningFired.current = false;

        return true;
      }

      if (response.status === 401) {
        console.warn("[SessionHeartbeat] Session expired (401)");
        setIsExpired(true);
        setRemainingMs(0);
        onExpireRef.current?.();
        return false;
      }

      // Other errors - don't mark as expired, could be transient
      console.error("[SessionHeartbeat] Ping failed:", response.status);
      return false;
    } catch (error) {
      console.error("[SessionHeartbeat] Ping error:", error);
      return false;
    }
  }, [pingEndpoint, sessionTimeoutMs]);

  // Reset session state
  const reset = useCallback(() => {
    setExpiresAt(null);
    setLastPingAt(null);
    setIsExpired(false);
    setRemainingMs(null);
    expiresAtRef.current = null;
    hasWarningFired.current = false;
  }, []);

  // Initial ping when enabled
  useEffect(() => {
    if (enabled) {
      ping();
    }
  }, [enabled, ping]);

  // Heartbeat interval - ping periodically to keep session alive
  useEffect(() => {
    if (!enabled || isExpired) {
      return;
    }

    const intervalId = setInterval(() => {
      ping();
    }, heartbeatIntervalMs);

    return () => clearInterval(intervalId);
  }, [enabled, isExpired, heartbeatIntervalMs, ping]);

  // Countdown timer - calculate remaining time from expiresAt
  useEffect(() => {
    if (!expiresAt || isExpired) {
      setRemainingMs(expiresAt ? 0 : null);
      return;
    }

    const updateRemaining = () => {
      const exp = expiresAtRef.current;
      if (!exp) {
        setRemainingMs(null);
        return;
      }

      const remaining = Math.max(0, exp - Date.now());
      setRemainingMs(remaining);

      // Fire warning callback once when entering warning zone
      if (remaining <= warningThresholdMs && remaining > 0 && !hasWarningFired.current) {
        hasWarningFired.current = true;
        onWarningRef.current?.(remaining);
      }

      if (remaining === 0) {
        setIsExpired(true);
        onExpireRef.current?.();
      }
    };

    // Update immediately
    updateRemaining();

    // Update every second for countdown
    const intervalId = setInterval(updateRemaining, COUNTDOWN_INTERVAL_MS);

    return () => clearInterval(intervalId);
  }, [expiresAt, isExpired, warningThresholdMs]);

  // Calculate warning state
  const isWarning = remainingMs !== null && remainingMs > 0 && remainingMs < warningThresholdMs;

  // Derive status from state
  const status: SessionStatus = isExpired
    ? "expired"
    : isWarning
      ? "warning"
      : expiresAt !== null
        ? "active"
        : "unknown";

  return {
    status,
    remainingMs,
    isWarning,
    isExpired,
    expiresAt,
    lastPingAt,
    ping,
    reset,
  };
}

// ============================================================================
// Utility Functions
// ============================================================================

/**
 * Format remaining time as human-readable string.
 *
 * @param ms - Milliseconds remaining (null for unknown)
 * @returns Formatted string like "2h 30m" or "5m 30s"
 */
export function formatRemainingTime(ms: number | null): string {
  if (ms === null) return "Unknown";
  if (ms <= 0) return "Expired";

  const seconds = Math.floor(ms / 1000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);

  if (hours > 0) {
    const remainingMinutes = minutes % 60;
    return remainingMinutes > 0 ? `${hours}h ${remainingMinutes}m` : `${hours}h`;
  }

  if (minutes > 0) {
    const remainingSeconds = seconds % 60;
    // Show seconds only when < 5 minutes remain
    return remainingSeconds > 0 && minutes < 5 ? `${minutes}m ${remainingSeconds}s` : `${minutes}m`;
  }

  return `${seconds}s`;
}

/**
 * Get status color class for styling.
 *
 * @param status - Session status
 * @returns Tailwind color class
 */
export function getSessionStatusColor(status: SessionStatus): string {
  switch (status) {
    case "active":
      return "text-[var(--dot-emerald)]";
    case "warning":
      return "text-[var(--dot-amber)]";
    case "expired":
      return "text-[var(--dot-red)]";
    default:
      return "text-muted-foreground";
  }
}
