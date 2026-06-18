/**
 * Consume-once auto-send from URL search params.
 *
 * Used for cross-route message hand-off: Test "Try in Chat", Eval "Run as
 * Chat", Data "Ask in Chat" navigate to `/playground/:id?autoSend=<msg>`.
 * This hook captures the param at mount, then fires the send callback
 * exactly once after the chat finishes loading, clearing the URL param to
 * prevent re-send on refresh.
 *
 * @module hooks/use-pending-auto-send
 */

import { useEffect, useRef } from "react";
import { useSearchParams } from "react-router";

interface UsePendingAutoSendOptions {
  /** True while the message list is still loading. */
  readonly isLoading: boolean;
  /** True while a stream response is in progress. */
  readonly isStreaming: boolean;
  /** The send callback (typically `handleSendMessage`). */
  readonly onSend: (message: string) => void;
}

/**
 * Consume the `autoSend` URL search param exactly once after messages have
 * loaded and no stream is active. Clears the param from the URL via
 * `replaceState` so browser refresh does not re-trigger.
 */
export function usePendingAutoSend({
  isLoading,
  isStreaming,
  onSend,
}: UsePendingAutoSendOptions): void {
  const [searchParams, setSearchParams] = useSearchParams();
  const pendingRef = useRef<string | null>(searchParams.get("autoSend"));

  useEffect(() => {
    if (isLoading || isStreaming) return;

    const msg = pendingRef.current;
    if (!msg) return;

    // Mark consumed before side-effects to guard against re-entry.
    pendingRef.current = null;

    // Clear from URL (prevent re-send on browser refresh)
    setSearchParams(
      (prev) => {
        prev.delete("autoSend");
        return prev;
      },
      { replace: true }
    );

    // Send via existing chat pipeline (handles titling, streaming, etc.)
    onSend(msg);
  }, [isLoading, isStreaming, setSearchParams, onSend]);
}
