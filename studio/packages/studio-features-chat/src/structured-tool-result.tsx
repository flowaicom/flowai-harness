import { CheckIcon, ChevronDownIcon, CopyIcon, LightbulbIcon, Loader2Icon } from "lucide-react";
import { Fragment, memo, useCallback, useEffect, useRef, useState } from "react";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

function formatValue(value: unknown): string {
  if (value === null || value === undefined) return "\u2014";
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "number") return String(value);
  return String(value);
}

function valueClass(key: string, value: unknown): string {
  if (key === "success" || key === "pass") return value ? "sr-success" : "sr-error";
  if (key === "error" || key === "errorMessage") return "sr-error";
  if (typeof value === "number") return "sr-number";
  return "";
}

function ErrorHints({ hints }: { readonly hints: readonly string[] }) {
  if (hints.length === 0) return null;

  return (
    <div className="mt-1.5 rounded-md border border-[var(--dot-amber)]/20 bg-[var(--accent-amber)] px-2.5 py-1.5">
      <div className="mb-1 flex items-center gap-1.5">
        <LightbulbIcon aria-hidden="true" className="size-3.5 text-[var(--dot-amber)]" />
        <span className="text-[11px] font-medium text-[var(--dot-amber)]">Suggestions</span>
      </div>
      <ul className="list-disc space-y-0.5 pl-5 text-[11px] text-[var(--dot-amber)]/80">
        {hints.map((hint, index) => (
          <li key={`${hint}-${index}`}>{hint}</li>
        ))}
      </ul>
    </div>
  );
}

function ResultCopyButton({ data }: { readonly data: unknown }) {
  const [copied, setCopied] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  useEffect(() => () => clearTimeout(timerRef.current), []);

  const handleCopy = useCallback(() => {
    const text = typeof data === "string" ? data : JSON.stringify(data, null, 2);
    navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => setCopied(false), 1500);
    });
  }, [data]);

  return (
    <button
      type="button"
      onClick={handleCopy}
      className={cx(
        "rounded p-1 transition-colors",
        copied
          ? "text-[var(--dot-emerald)]"
          : "text-muted-foreground/40 hover:bg-muted hover:text-muted-foreground"
      )}
      title="Copy result"
      aria-label="Copy tool result to clipboard"
    >
      {copied ? (
        <CheckIcon aria-hidden="true" className="size-3" />
      ) : (
        <CopyIcon aria-hidden="true" className="size-3" />
      )}
    </button>
  );
}

function SubAgentResponseExpand({
  responseId,
  loadResponse,
}: {
  readonly responseId: string;
  readonly loadResponse: (responseId: string) => Promise<string | null>;
}) {
  const [expanded, setExpanded] = useState(false);
  const [body, setBody] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const handleExpand = useCallback(async () => {
    if (expanded) {
      setExpanded(false);
      return;
    }
    setExpanded(true);
    if (body !== null) return;
    setLoading(true);
    try {
      const loadedBody = await loadResponse(responseId);
      setBody(loadedBody ?? "[Response expired or unavailable]");
    } catch {
      setBody("[Failed to load response]");
    } finally {
      setLoading(false);
    }
  }, [body, expanded, loadResponse, responseId]);

  return (
    <div className="mt-2">
      <button
        type="button"
        onClick={handleExpand}
        className="flex items-center gap-1 text-[11px] text-[var(--dot-purple)] hover:underline"
      >
        <ChevronDownIcon
          className={cx("size-3 transition-transform duration-150", expanded && "rotate-180")}
        />
        {expanded ? "Collapse" : "Show full response"}
      </button>
      {expanded ? (
        <div className="scroll-container mt-1 max-h-64 overflow-y-auto whitespace-pre-wrap rounded bg-muted/30 p-2 font-mono text-xs">
          {loading ? (
            <Loader2Icon className="size-3 animate-spin text-muted-foreground" />
          ) : (
            (body ?? "")
          )}
        </div>
      ) : null}
    </div>
  );
}

export interface SharedStructuredToolResultProps {
  readonly data: unknown;
  readonly scramble?: (value: string) => string;
  readonly errorHints?: readonly string[];
  readonly isError?: boolean;
  readonly loadResponse?: (responseId: string) => Promise<string | null>;
}

export const SharedStructuredToolResult = memo(function SharedStructuredToolResult({
  data,
  scramble = (value) => value,
  errorHints = [],
  isError = false,
  loadResponse,
}: SharedStructuredToolResultProps) {
  if (typeof data === "string") {
    return <span className="text-xs text-muted-foreground">{scramble(data)}</span>;
  }

  if (data == null) {
    return <span className="text-xs italic text-muted-foreground">No result</span>;
  }

  const responseId =
    typeof data === "object" && data !== null
      ? (((data as Record<string, unknown>).response_id ??
          (data as Record<string, unknown>).responseId) as unknown)
      : undefined;

  if (typeof data === "object" && !Array.isArray(data)) {
    const entries = Object.entries(data as Record<string, unknown>);
    const isFlat = entries.every(([, value]) => value == null || typeof value !== "object");

    if (isFlat && entries.length > 0 && entries.length <= 16) {
      return (
        <>
          <div className="structured-result">
            {entries.map(([key, value]) => (
              <Fragment key={key}>
                <span className="sr-key">{scramble(key)}</span>
                <span className={cx("sr-val", valueClass(key, value))}>
                  {scramble(formatValue(value))}
                </span>
              </Fragment>
            ))}
          </div>
          <ErrorHints hints={errorHints} />
          {typeof responseId === "string" && loadResponse ? (
            <SubAgentResponseExpand responseId={responseId} loadResponse={loadResponse} />
          ) : null}
        </>
      );
    }
  }

  const json = scramble(JSON.stringify(data, null, 2));
  return (
    <>
      <pre
        className={cx(
          "whitespace-pre-wrap font-mono text-xs leading-relaxed",
          isError ? "text-[var(--dot-red)]" : "text-muted-foreground"
        )}
      >
        {json}
      </pre>
      <ErrorHints hints={errorHints} />
      {typeof responseId === "string" && loadResponse ? (
        <SubAgentResponseExpand responseId={responseId} loadResponse={loadResponse} />
      ) : null}
    </>
  );
});

export { ResultCopyButton };
