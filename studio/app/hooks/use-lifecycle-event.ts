/**
 * React hook for subscribing to lifecycle events.
 *
 * @module hooks/use-lifecycle-event
 */

import { useEffect, useRef } from "react";
import { type LifecycleEvent, lifecycleBus } from "~/lib/stores/lifecycle-bus";

/**
 * Subscribe to a specific lifecycle event type.
 * The handler is stable across renders (latest ref captured via useRef).
 */
export function useLifecycleEvent<T extends LifecycleEvent["type"]>(
  type: T,
  handler: (event: Extract<LifecycleEvent, { type: T }>) => void
): void {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;

  useEffect(() => {
    return lifecycleBus.on(type, (event) => handlerRef.current(event));
  }, [type]);
}
