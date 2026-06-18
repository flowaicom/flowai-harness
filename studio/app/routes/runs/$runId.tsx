import { useEffect, useState } from "react";
import { Link, useParams } from "react-router";
import { RunTimeline } from "~/components/runs/run-timeline";
import { getRun, type RunSummary } from "~/lib/api/runs";
import { isOk } from "~/lib/domain/result";
import { isEffectivelyTerminalRun, isStaleRunningRun } from "~/lib/domain/run-events";
import { useRunEvents } from "~/lib/hooks/use-run-events";

/** Run detail with an event timeline (tool calls, sub-agent calls, status). */
export default function RunDetail() {
  const { runId = "" } = useParams();
  const [run, setRun] = useState<RunSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const {
    timelineItems,
    loading: eventsLoading,
    error: eventsError,
  } = useRunEvents(runId, { poll: !isEffectivelyTerminalRun(run) });

  useEffect(() => {
    let cancelled = false;
    getRun(runId).then((runResult) => {
      if (cancelled) return;
      if (isOk(runResult)) {
        setRun(runResult.value);
        setError(null);
      } else {
        setError(runResult.error.message);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [runId]);

  if (error) {
    return <p className="p-6 text-sm text-red-600">{error}</p>;
  }

  return (
    <div className="space-y-4 p-6">
      <Link to="/runs" className="text-xs text-muted-foreground hover:underline">
        ← All runs
      </Link>
      <div>
        <h2 className="font-mono text-sm">{runId}</h2>
        {run && (
          <p className="text-xs text-muted-foreground">
            {run.operation} · {isStaleRunningRun(run) ? "interrupted" : run.status} ·{" "}
            {run.eventCount} events
          </p>
        )}
      </div>
      {eventsError && <p className="text-sm text-red-600">{eventsError}</p>}
      {eventsLoading && timelineItems.length === 0 ? (
        <div className="space-y-2">
          <div className="h-20 rounded-lg bg-muted animate-shimmer" />
          <div className="h-20 rounded-lg bg-muted/70 animate-shimmer" />
        </div>
      ) : (
        <RunTimeline items={timelineItems} />
      )}
    </div>
  );
}
