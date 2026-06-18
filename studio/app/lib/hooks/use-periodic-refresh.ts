import { useCallback, useEffect, useRef, useState } from "react";

interface UsePeriodicRefreshOptions {
  readonly enabled?: boolean;
  readonly intervalMs?: number;
  readonly onError?: (error: unknown) => void;
}

interface UsePeriodicRefreshResult {
  readonly isRefreshing: boolean;
  readonly lastCompletedAt: string | null;
  readonly refresh: () => Promise<void>;
}

export function usePeriodicRefresh(
  refreshFn: () => Promise<void>,
  { enabled = true, intervalMs = 10_000, onError }: UsePeriodicRefreshOptions = {}
): UsePeriodicRefreshResult {
  const refreshRef = useRef(refreshFn);
  const onErrorRef = useRef(onError);
  const inFlightRef = useRef<Promise<void> | null>(null);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [lastCompletedAt, setLastCompletedAt] = useState<string | null>(null);

  useEffect(() => {
    refreshRef.current = refreshFn;
  }, [refreshFn]);

  useEffect(() => {
    onErrorRef.current = onError;
  }, [onError]);

  const refresh = useCallback(async () => {
    if (inFlightRef.current) {
      return inFlightRef.current;
    }

    setIsRefreshing(true);
    const nextRefresh = refreshRef
      .current()
      .then(() => {
        setLastCompletedAt(new Date().toISOString());
      })
      .catch((error) => {
        onErrorRef.current?.(error);
      })
      .finally(() => {
        inFlightRef.current = null;
        setIsRefreshing(false);
      });

    inFlightRef.current = nextRefresh;
    return nextRefresh;
  }, []);

  useEffect(() => {
    if (!enabled) {
      return;
    }

    void refresh();

    const intervalId = window.setInterval(() => {
      void refresh();
    }, intervalMs);
    const handleFocus = () => {
      void refresh();
    };
    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        void refresh();
      }
    };

    window.addEventListener("focus", handleFocus);
    document.addEventListener("visibilitychange", handleVisibilityChange);

    return () => {
      window.clearInterval(intervalId);
      window.removeEventListener("focus", handleFocus);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [enabled, intervalMs, refresh]);

  return {
    isRefreshing,
    lastCompletedAt,
    refresh,
  };
}
