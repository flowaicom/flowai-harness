import { ArrowLeftIcon, NetworkIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router";
import { CopyButton } from "~/components/shared/copy-button";
import { EmptyState } from "~/components/shared/empty-state";
import { ErrorBanner } from "~/components/shared/error-banner";
import { getTrace, type TracePayload, type TraceRecord, type TraceRow } from "~/lib/api/runs";
import { isOk } from "~/lib/domain/result";

function payloadPreview(payload: TracePayload | null | undefined): string {
  if (!payload) return "null";
  switch (payload.kind) {
    case "inline":
      return JSON.stringify(payload.value, null, 2);
    case "omitted":
      return `omitted: ${payload.reason}`;
    case "redacted":
      return JSON.stringify(payload.redaction, null, 2);
    default:
      return JSON.stringify(payload, null, 2);
  }
}

function row(label: string, value: string | number | null | undefined) {
  return (
    <div>
      <div className="text-[11px] uppercase tracking-wide text-muted-foreground/70">{label}</div>
      <div className="mt-1 break-words font-mono text-xs text-muted-foreground">
        {value ?? "none"}
      </div>
    </div>
  );
}

function TraceSummary({ trace }: { readonly trace: TraceRecord }) {
  return (
    <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
      <div className="rounded-lg border bg-card p-4">
        <div className="text-2xl font-semibold">{trace.steps.length}</div>
        <div className="mt-1 text-xs text-muted-foreground">steps</div>
      </div>
      <div className="rounded-lg border bg-card p-4">
        <div className="text-2xl font-semibold">{trace.status}</div>
        <div className="mt-1 text-xs text-muted-foreground">status</div>
      </div>
      <div className="rounded-lg border bg-card p-4">
        <div className="text-2xl font-semibold">{trace.stage}</div>
        <div className="mt-1 text-xs text-muted-foreground">stage</div>
      </div>
      <div className="rounded-lg border bg-card p-4">
        <div className="text-2xl font-semibold">{trace.scope.sampleIndex ?? "n/a"}</div>
        <div className="mt-1 text-xs text-muted-foreground">sample</div>
      </div>
    </div>
  );
}

export default function EvalTraceDetailRoute() {
  const { evalId, testCaseId, traceId } = useParams<{
    evalId: string;
    testCaseId: string;
    traceId: string;
  }>();
  const [searchParams] = useSearchParams();
  const sample = searchParams.get("sample") ?? "0";
  const [traceRow, setTraceRow] = useState<TraceRow | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!traceId) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    getTrace(traceId).then((result) => {
      if (cancelled) return;
      setLoading(false);
      if (isOk(result)) {
        setTraceRow(result.value);
      } else {
        setError(result.error.message);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [traceId]);

  if (!evalId || !testCaseId || !traceId) {
    return (
      <EmptyState
        icon={NetworkIcon}
        title="No trace selected"
        description="Choose a sample trace from an eval case detail page."
      />
    );
  }

  if (loading && !traceRow) {
    return (
      <div className="flex-1 p-8">
        <EmptyState
          icon={NetworkIcon}
          title="Loading trace"
          description="Studio is resolving the persisted runtime trace."
        />
      </div>
    );
  }

  const trace = traceRow?.trace ?? null;

  return (
    <div className="flex-1 overflow-y-auto scroll-container">
      <div className="mx-auto max-w-5xl p-8 space-y-6">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <Link
              to={`/evals/${evalId}/cases/${testCaseId}?sample=${sample}`}
              className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
            >
              <ArrowLeftIcon className="size-4" />
              Back to Eval Case
            </Link>
            <h1 className="mt-3 inline-flex items-center gap-2 font-mono text-2xl text-foreground">
              {traceId}
              <CopyButton text={traceId} label="Copy trace ID" />
            </h1>
          </div>
          {trace ? (
            <div className="rounded-md border px-3 py-1 text-xs font-medium text-muted-foreground">
              {trace.status}
            </div>
          ) : null}
        </div>

        {error ? <ErrorBanner message={error} onDismiss={() => setError(null)} /> : null}

        {!trace ? (
          <EmptyState
            icon={NetworkIcon}
            title="Trace unavailable"
            description="The trace reference exists, but no persisted trace record was found."
          />
        ) : (
          <>
            <TraceSummary trace={trace} />

            <div className="grid gap-4 lg:grid-cols-2">
              <section className="rounded-xl border bg-card p-4">
                <div className="text-[11px] uppercase tracking-wide text-muted-foreground/70">
                  Scope
                </div>
                <div className="mt-3 grid gap-3 sm:grid-cols-2">
                  {row("eval", trace.scope.evalRunId)}
                  {row("test case", trace.scope.testCaseId)}
                  {row("thread", trace.scope.threadId)}
                  {row("sample", trace.scope.sampleIndex)}
                </div>
              </section>
              <section className="rounded-xl border bg-card p-4">
                <div className="text-[11px] uppercase tracking-wide text-muted-foreground/70">
                  Timing
                </div>
                <div className="mt-3 grid gap-3 sm:grid-cols-2">
                  {row("started", trace.startedAt)}
                  {row("completed", trace.completedAt)}
                  {row("workspace", trace.workspaceId)}
                  {row("stored", traceRow?.updatedAt)}
                </div>
              </section>
            </div>

            <section className="rounded-xl border bg-card p-4">
              <div className="text-[11px] uppercase tracking-wide text-muted-foreground/70">
                Provenance
              </div>
              <pre className="mt-3 max-h-80 overflow-auto rounded-md bg-muted/40 p-3 text-xs text-muted-foreground whitespace-pre-wrap">
                {JSON.stringify(trace.provenance, null, 2)}
              </pre>
            </section>

            <section className="space-y-3">
              <div>
                <div className="text-[11px] uppercase tracking-wide text-muted-foreground/70">
                  Steps
                </div>
                <div className="mt-1 text-sm text-muted-foreground">
                  Tool-oriented runtime trace steps captured by the eval sample execution path.
                </div>
              </div>
              {trace.steps.length === 0 ? (
                <EmptyState
                  icon={NetworkIcon}
                  title="No steps recorded"
                  description="The sample completed without captured tool calls."
                />
              ) : (
                trace.steps.map((step) => (
                  <article key={step.ordinal} className="rounded-xl border bg-card p-4">
                    <div className="flex flex-wrap items-start justify-between gap-3">
                      <div>
                        <div className="font-medium text-foreground">{step.toolName}</div>
                        <div className="mt-1 flex flex-wrap gap-3 text-xs text-muted-foreground">
                          <span>#{step.ordinal}</span>
                          {step.actor ? <span>{step.actor}</span> : null}
                          {step.toolCallId ? <span>{step.toolCallId}</span> : null}
                        </div>
                      </div>
                      {step.error ? (
                        <div className="rounded-md bg-[var(--accent-red)] px-2 py-1 text-xs text-[var(--dot-red)]">
                          error
                        </div>
                      ) : null}
                    </div>
                    {step.error ? (
                      <div className="mt-3 rounded-md border border-[var(--dot-red)]/30 bg-[var(--accent-red)] px-3 py-2 text-sm text-[var(--dot-red)]">
                        {step.error}
                      </div>
                    ) : null}
                    <div className="mt-4 grid gap-4 lg:grid-cols-2">
                      <div>
                        <div className="text-[11px] uppercase tracking-wide text-muted-foreground/70">
                          Arguments
                        </div>
                        <pre className="mt-2 max-h-96 overflow-auto rounded-md bg-muted/40 p-3 text-xs text-muted-foreground whitespace-pre-wrap">
                          {payloadPreview(step.arguments)}
                        </pre>
                      </div>
                      <div>
                        <div className="text-[11px] uppercase tracking-wide text-muted-foreground/70">
                          Result
                        </div>
                        <pre className="mt-2 max-h-96 overflow-auto rounded-md bg-muted/40 p-3 text-xs text-muted-foreground whitespace-pre-wrap">
                          {payloadPreview(step.result)}
                        </pre>
                      </div>
                    </div>
                  </article>
                ))
              )}
            </section>
          </>
        )}
      </div>
    </div>
  );
}
