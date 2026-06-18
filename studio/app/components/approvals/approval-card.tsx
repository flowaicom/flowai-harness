import { CheckIcon, PencilIcon, ShieldCheckIcon, XIcon } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import {
  type ApprovalOutcome,
  type RespondToApprovalInput,
  respondToApproval,
} from "~/lib/api/approvals";
import { parseApprovalPartialJson } from "~/lib/domain/approval-response";
import { isOk } from "~/lib/domain/result";
import { cn } from "~/lib/utils";

export interface ApprovalCardRef {
  readonly approvalId: string;
  readonly threadId?: string;
  readonly runId?: string;
  readonly status: string;
  readonly payload: Record<string, unknown>;
  readonly createdAt?: string;
  readonly updatedAt?: string;
}

interface ReviewAction {
  readonly kind: string;
  readonly payload: Record<string, unknown>;
}

function statusTone(status: string): string {
  switch (status) {
    case "approve":
    case "approved":
      return "bg-[var(--dot-emerald)]";
    case "reject":
    case "rejected":
      return "bg-[var(--dot-red)]";
    case "revise":
      return "bg-[var(--dot-amber)]";
    case "pending":
      return "bg-muted-foreground/70";
    default:
      return "bg-muted-foreground/50";
  }
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/** The gated body: the plan being approved, or the tool args. */
function approvalBody(payload: Record<string, unknown>): Record<string, unknown> | null {
  return asRecord(payload.payload) ?? asRecord(asRecord(payload.raw)?.payload) ?? null;
}

/** Flatten the runtime's ActionSeq (`{head, tail}`) or a plain array. */
function flattenActions(actions: unknown): ReviewAction[] {
  const items: unknown[] = [];
  const seq = asRecord(actions);
  if (seq?.head) {
    items.push(seq.head, ...(Array.isArray(seq.tail) ? seq.tail : []));
  } else if (Array.isArray(actions)) {
    items.push(...actions);
  }
  return items.flatMap((item) => {
    const record = asRecord(item);
    if (!record) return [];
    const kind = typeof record.kind === "string" ? record.kind : "action";
    return [{ kind, payload: asRecord(record.payload) ?? {} }];
  });
}

/** The plan rationale, which the runtime nests under `context`. */
function planRationale(body: Record<string, unknown>): string | null {
  if (typeof body.rationale === "string") return body.rationale;
  const context = asRecord(body.context);
  return typeof context?.rationale === "string" ? context.rationale : null;
}

/** Render the plan actions (or raw body) under review, so approving is informed. */
function ApprovalReview({ payload }: { readonly payload: Record<string, unknown> }) {
  const body = approvalBody(payload);
  if (!body) return null;
  const actions = flattenActions(body.actions);
  const rationale = planRationale(body);

  if (actions.length === 0) {
    return (
      <details className="rounded-md border border-border/60 bg-muted/20 px-2.5 py-2">
        <summary className="cursor-pointer text-xs text-muted-foreground">Details</summary>
        <pre className="mt-1 overflow-x-auto font-mono text-xs">
          {JSON.stringify(body, null, 2)}
        </pre>
      </details>
    );
  }

  return (
    <div className="space-y-2 rounded-md border border-border/60 bg-muted/20 p-2.5">
      {actions.map((action, index) => (
        <div key={`${action.kind}-${index}`} className="min-w-0 space-y-1 text-xs">
          <span className="inline-flex rounded border border-border/60 bg-background/70 px-1.5 py-0.5 font-mono font-medium text-foreground">
            {action.kind}
          </span>
          <code className="block min-w-0 break-words font-mono leading-relaxed text-muted-foreground">
            {JSON.stringify(action.payload)}
          </code>
        </div>
      ))}
      {rationale && <p className="text-xs italic text-muted-foreground">{rationale}</p>}
    </div>
  );
}

export function ApprovalCard({
  approval,
  onResolved,
  showContext = true,
  className,
}: {
  readonly approval: ApprovalCardRef;
  readonly onResolved?: () => void;
  readonly showContext?: boolean;
  readonly className?: string;
}) {
  const [feedback, setFeedback] = useState("");
  const [partialJson, setPartialJson] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState(approval.status);

  useEffect(() => {
    setStatus(approval.status);
  }, [approval.status]);

  const respond = useCallback(
    (outcome: ApprovalOutcome) => {
      const parsedPartial =
        outcome === "revise" ? parseApprovalPartialJson(partialJson) : { ok: true as const };
      if (!parsedPartial.ok) {
        setError(parsedPartial.error);
        return;
      }

      const input: RespondToApprovalInput = {
        feedback: feedback || undefined,
        partial: outcome === "revise" ? parsedPartial.partial : undefined,
      };

      setBusy(true);
      setError(null);
      respondToApproval(approval.approvalId, outcome, input).then((result) => {
        setBusy(false);
        if (isOk(result)) {
          setStatus(result.value.status);
          onResolved?.();
        } else {
          setError(result.error.message);
        }
      });
    },
    [approval.approvalId, feedback, partialJson, onResolved]
  );

  const title = String(approval.payload.title ?? approval.payload.kind ?? "Approval required");
  const kind = typeof approval.payload.kind === "string" ? approval.payload.kind : undefined;
  const target = typeof approval.payload.target === "string" ? approval.payload.target : undefined;
  const context = [
    showContext && approval.threadId ? `thread ${approval.threadId}` : null,
    showContext && approval.runId ? `run ${approval.runId}` : null,
    !showContext && kind ? kind : null,
    !showContext && target ? target : null,
  ].filter(Boolean);
  const pending = status === "pending";
  const decisionButtonClass =
    "inline-flex items-center gap-1.5 rounded-md border border-border/70 bg-background/80 px-2.5 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-40";
  const fieldClass =
    "w-full rounded-md border border-border/70 bg-background/80 px-2.5 py-1.5 text-sm outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring";

  return (
    <div
      className={cn(
        "space-y-3 rounded-lg border border-border/70 bg-background/70 p-4 shadow-sm",
        className
      )}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="flex items-center gap-1.5 text-sm font-medium text-foreground">
            <ShieldCheckIcon aria-hidden="true" className="size-4 text-muted-foreground/70" />
            <span>{title}</span>
          </p>
          {context.length > 0 && (
            <p className="mt-0.5 text-xs text-muted-foreground">{context.join(" · ")}</p>
          )}
        </div>
        <span className="inline-flex shrink-0 items-center gap-1.5 rounded-full border border-border/70 bg-background/80 px-2 py-0.5 text-xs font-medium text-muted-foreground">
          <span aria-hidden="true" className={cn("status-dot", statusTone(status))} />
          {status}
        </span>
      </div>
      <ApprovalReview payload={approval.payload} />
      {pending && (
        <>
          <textarea
            value={feedback}
            onChange={(event) => setFeedback(event.target.value)}
            placeholder="Feedback (optional)"
            rows={2}
            className={fieldClass}
          />
          <textarea
            value={partialJson}
            onChange={(event) => setPartialJson(event.target.value)}
            placeholder='Partial JSON for revise, e.g. {"query": "updated"}'
            rows={4}
            spellCheck={false}
            className={cn(fieldClass, "font-mono text-xs")}
          />
          {error && <p className="text-xs text-destructive">{error}</p>}
          <div className="flex flex-wrap gap-1.5">
            <button
              type="button"
              disabled={busy}
              onClick={() => respond("approve")}
              className={decisionButtonClass}
            >
              <CheckIcon aria-hidden="true" className="size-3.5" />
              Approve
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => respond("reject")}
              className={decisionButtonClass}
            >
              <XIcon aria-hidden="true" className="size-3.5" />
              Reject
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => respond("revise")}
              className={decisionButtonClass}
            >
              <PencilIcon aria-hidden="true" className="size-3.5" />
              Revise
            </button>
          </div>
        </>
      )}
    </div>
  );
}
