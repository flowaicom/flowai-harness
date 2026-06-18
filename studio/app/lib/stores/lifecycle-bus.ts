/**
 * Cross-tab lifecycle event bus.
 *
 * Lightweight pub-sub enabling the Connect → Chat → Test → Eval flow:
 * - Connect tab emits "importComplete" after ETL finishes
 * - Chat tab emits "agentTaskCompleted" after an agent completes a task
 * - Test tab emits "testCaseCreated" after saving from builder
 * - Eval tab emits "evalCompleted" with summary
 *
 * Design: simple typed EventTarget wrapper — no external deps, no store
 * coupling. Stores emit; components subscribe via useLifecycleEvent hook.
 *
 * @module stores/lifecycle-bus
 */

// =============================================================================
// Event Types
// =============================================================================

/**
 * Shared base fields present on every lifecycle event.
 *
 * `workspaceId` enables multi-tab workspace isolation — subscribers can
 * ignore events from a different workspace context. Optional for backward
 * compat (older emission sites that don't yet supply it).
 */
interface LifecycleEventBase {
  readonly workspaceId?: string;
}

export type LifecycleEvent =
  | (LifecycleEventBase & {
      readonly type: "importComplete";
      readonly sourceId: string;
      readonly tableCount: number;
      readonly totalRowCount: number;
    })
  | (LifecycleEventBase & {
      readonly type: "profilingComplete";
      readonly sourceId: string;
      readonly tableCount: number;
    })
  | (LifecycleEventBase & {
      readonly type: "agentTaskCompleted";
      readonly threadId: string;
      readonly taskId: string;
    })
  | (LifecycleEventBase & {
      readonly type: "testCaseCreated";
      readonly testCaseId: string;
      readonly sourceThreadId: string | null;
    })
  | (LifecycleEventBase & {
      readonly type: "evalCompleted";
      readonly evalId: string;
      readonly passRate: number;
      readonly totalCases: number;
    });

// =============================================================================
// Bus Implementation
// =============================================================================

type Listener = (event: LifecycleEvent) => void;

class LifecycleBus {
  private listeners = new Set<Listener>();

  /** Emit a lifecycle event to all subscribers. */
  emit(event: LifecycleEvent): void {
    for (const listener of this.listeners) {
      try {
        listener(event);
      } catch (e) {
        if (import.meta.env.DEV) {
          console.error("[lifecycle-bus] listener error:", e);
        }
      }
    }
  }

  /** Subscribe to all lifecycle events. Returns unsubscribe function. */
  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  /** Subscribe to a specific event type. Returns unsubscribe function. */
  on<T extends LifecycleEvent["type"]>(
    type: T,
    handler: (event: Extract<LifecycleEvent, { type: T }>) => void
  ): () => void {
    const filtered: Listener = (event) => {
      if (event.type === type) {
        handler(event as Extract<LifecycleEvent, { type: T }>);
      }
    };
    return this.subscribe(filtered);
  }
}

/** Singleton lifecycle event bus — shared across all stores and components. */
export const lifecycleBus = new LifecycleBus();
