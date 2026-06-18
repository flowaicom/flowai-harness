/**
 * Shared async lifecycle vocabulary.
 *
 * Uses `phase` (not `status`) to avoid collision with domain status fields.
 * Uses `failed` + `reason` (not `error`) to keep operational failures typed.
 */

export type AsyncPhase =
  | { readonly phase: "idle" }
  | { readonly phase: "loading" }
  | { readonly phase: "ready" }
  | { readonly phase: "failed"; readonly reason: string };

export const AsyncPhase = {
  idle: { phase: "idle" } as const,
  loading: { phase: "loading" } as const,
  ready: { phase: "ready" } as const,
  failed: (reason: string): AsyncPhase => ({ phase: "failed", reason }),
} as const;

const ASYNC_PHASE_VALUES = new Set<string>(["idle", "loading", "ready", "failed"]);

export function parseAsyncPhase(raw: unknown): AsyncPhase | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const { phase } = raw as { phase?: unknown };
  if (typeof phase !== "string" || !ASYNC_PHASE_VALUES.has(phase)) return undefined;
  if (phase === "failed") {
    const { reason } = raw as { reason?: unknown };
    return typeof reason === "string" ? { phase, reason } : undefined;
  }
  return { phase } as AsyncPhase;
}
