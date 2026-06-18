import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getRunEvents, type RunEventEnvelope } from "~/lib/api/runs";
import { isOk } from "~/lib/domain/result";
import { projectRunTimeline, runHasTerminalEvent } from "~/lib/domain/run-events";

interface UseRunEventsOptions {
  readonly enabled?: boolean;
  readonly poll?: boolean;
  readonly pollIntervalMs?: number;
}

function mergeBySeq(
  existing: readonly RunEventEnvelope[],
  incoming: readonly RunEventEnvelope[]
): readonly RunEventEnvelope[] {
  if (incoming.length === 0) return existing;
  const bySeq = new Map<number, RunEventEnvelope>();
  for (const event of existing) bySeq.set(event.seq, event);
  for (const event of incoming) bySeq.set(event.seq, event);
  return Array.from(bySeq.values()).sort((a, b) => a.seq - b.seq);
}

export function useRunEvents(
  runId: string | null | undefined,
  { enabled = true, poll = false, pollIntervalMs = 2000 }: UseRunEventsOptions = {}
) {
  const [events, setEvents] = useState<readonly RunEventEnvelope[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const lastSeqRef = useRef<number | undefined>(undefined);
  const loadingRef = useRef(false);
  const runIdRef = useRef<string | null>(runId ?? null);
  runIdRef.current = runId ?? null;

  const load = useCallback(
    async (sinceSeq?: number) => {
      if (!enabled || !runId || loadingRef.current) return;
      const targetRunId = runId;
      loadingRef.current = true;
      setLoading(true);
      const result = await getRunEvents(targetRunId, sinceSeq);
      if (runIdRef.current !== targetRunId) return;
      loadingRef.current = false;
      setLoading(false);

      if (!isOk(result)) {
        setError(result.error.message);
        return;
      }

      setError(null);
      setEvents((current) => {
        const next =
          sinceSeq === undefined ? mergeBySeq([], result.value) : mergeBySeq(current, result.value);
        lastSeqRef.current = next.length > 0 ? next[next.length - 1]?.seq : undefined;
        return next;
      });
    },
    [enabled, runId]
  );

  useEffect(() => {
    loadingRef.current = false;
    lastSeqRef.current = undefined;
    setEvents([]);
    setError(null);
    if (enabled && runId) void load(undefined);
  }, [enabled, load, runId]);

  useEffect(() => {
    if (!enabled || !runId || !poll || runHasTerminalEvent(events)) return;
    const id = setInterval(() => {
      void load(lastSeqRef.current);
    }, pollIntervalMs);
    return () => clearInterval(id);
  }, [enabled, events, load, poll, pollIntervalMs, runId]);

  const timelineItems = useMemo(() => projectRunTimeline(events), [events]);
  const refresh = useCallback(() => load(lastSeqRef.current), [load]);

  return {
    events,
    timelineItems,
    loading,
    error,
    lastSeq: lastSeqRef.current,
    refresh,
  };
}
