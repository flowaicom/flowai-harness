/**
 * Message part display components.
 *
 * Renders different types of message parts:
 * - Text (markdown)
 * - Reasoning (collapsible, rendered as markdown)
 * - Tool invocations (collapsible with structured result rendering)
 * - Tool groups (accordion for consecutive tool calls)
 * - Sub-agent calls (theme-aware)
 * - File attachments (theme-aware)
 * - CommandCard (approval DSL)
 * - Tool progress (phased)
 *
 * Performance optimizations:
 * - Memoized components with custom comparators
 * - useMemo for JSON formatting
 * - useCallback for stable handlers
 *
 * @module components/chat/message-part
 */

import {
  BotIcon,
  CheckIcon,
  ChevronDownIcon,
  CopyIcon,
  DownloadIcon,
  FileTextIcon,
  LightbulbIcon,
  Loader2Icon,
  WrenchIcon,
  XIcon,
} from "lucide-react";
import {
  Fragment,
  lazy,
  memo,
  type ReactNode,
  Suspense,
  useCallback,
  useMemo,
  useRef,
  useState,
} from "react";
import { ApprovalCard } from "~/components/approvals/approval-card";
import { Markdown } from "~/components/shared/markdown";
import type {
  DisplayPart,
  MessagePart,
  SubAgentInvocationDisplayPart,
  ToolGroupDisplayPart,
  ToolInvocationMessagePart,
  ToolPhase,
} from "~/lib/domain/message";
import { getUserActionLabel, isUserActionMessage, parseUserAction } from "~/lib/domain/user-action";
import { useScramble } from "~/lib/scramble";
import { cn } from "~/lib/utils";
import { useChatContext } from "./chat-context";

// Lazy-load CommandCard renderer to avoid bundling it when not needed
const CommandCardRenderer = lazy(() =>
  import("./command-card-renderer").then((m) => ({ default: m.CommandCardRenderer }))
);

// ============================================================================
// CommandCard Detection
// ============================================================================

/** Check if a tool invocation result contains CommandCard DSL */
function isCommandCardToolResult(
  toolName: string,
  result: unknown
): result is { success: true; dsl: string } {
  // Check for any tool that produces CommandCard DSL
  if (!toolName) return false;
  if (!result || typeof result !== "object") return false;
  const r = result as Record<string, unknown>;
  return r.success === true && typeof r.dsl === "string" && r.dsl.length > 0;
}

// ============================================================================
// Structured Result Renderer
// ============================================================================

/** Format a primitive value for display. */
function formatValue(v: unknown): string {
  if (v === null || v === undefined) return "\u2014";
  if (typeof v === "boolean") return v ? "true" : "false";
  if (typeof v === "number") return String(v);
  return String(v);
}

/** Classify a value for CSS styling. */
function valueClass(key: string, v: unknown): string {
  if (key === "success" || key === "pass") return v ? "sr-success" : "sr-error";
  if (key === "error" || key === "errorMessage") return "sr-error";
  if (typeof v === "number") return "sr-number";
  return "";
}

/**
 * Extract error hints/suggestions from a tool result.
 *
 * Handles multiple formats from the framework's error enrichment:
 * - `hints: string[]` — Rust ToolError serialization (PatternEnricher)
 * - `hint: string` — Python dict-based error format (singular)
 * - `suggestions: string[]` — Alternative array format
 */
function extractErrorHints(data: unknown): string[] {
  if (!data || typeof data !== "object") return [];
  const r = data as Record<string, unknown>;

  // Direct hints array (Rust ToolError serialization via PatternEnricher)
  if (Array.isArray(r.hints)) {
    return r.hints.filter((h): h is string => typeof h === "string");
  }

  // Single hint string (Python dict-based error format)
  if (typeof r.hint === "string" && r.hint.length > 0) {
    return [r.hint];
  }

  // Suggestions array (legacy format)
  if (Array.isArray(r.suggestions)) {
    return r.suggestions.filter((h): h is string => typeof h === "string");
  }

  return [];
}

/**
 * Check if a tool result represents an error (from framework ToolError).
 */
function isErrorResult(data: unknown): boolean {
  if (!data || typeof data !== "object") return false;
  const r = data as Record<string, unknown>;
  return r.is_error === true || r.isError === true || r.success === false;
}

/**
 * Render tool results as structured key-value table when possible,
 * falling back to compact JSON for complex structures.
 *
 * Enhanced with error hint rendering: when a tool result contains error hints
 * from the framework's error enrichment (PatternEnricher), they are displayed
 * as actionable suggestions with a lightbulb icon.
 */
const StructuredResult = memo(function StructuredResult({
  data,
  scramble,
}: {
  data: unknown;
  scramble: (s: string) => string;
}) {
  // String → inline text
  if (typeof data === "string") {
    return <span className="text-xs text-muted-foreground">{scramble(data)}</span>;
  }

  // Null/undefined
  if (data == null) {
    return <span className="text-xs text-muted-foreground italic">No result</span>;
  }

  // Extract error hints for display after the result
  const hints = extractErrorHints(data);
  const hasError = isErrorResult(data);

  // Flat object → structured key-value table
  if (typeof data === "object" && !Array.isArray(data)) {
    const entries = Object.entries(data as Record<string, unknown>);
    const isFlat = entries.every(([, v]) => v === null || v === undefined || typeof v !== "object");

    if (isFlat && entries.length > 0 && entries.length <= 16) {
      const responseId =
        (data as Record<string, unknown>).response_id ??
        (data as Record<string, unknown>).responseId;
      return (
        <>
          <div className="structured-result">
            {entries.map(([key, value]) => (
              <Fragment key={key}>
                <span className="sr-key">{scramble(key)}</span>
                <span className={cn("sr-val", valueClass(key, value))}>
                  {scramble(formatValue(value))}
                </span>
              </Fragment>
            ))}
          </div>
          {hints.length > 0 && <ErrorHints hints={hints} />}
          {typeof responseId === "string" && <SubAgentResponseExpand responseId={responseId} />}
        </>
      );
    }
  }

  // Fallback: prettified JSON
  const json = scramble(JSON.stringify(data, null, 2));
  return (
    <>
      <pre
        className={cn(
          "whitespace-pre-wrap font-mono text-xs leading-relaxed",
          hasError ? "text-[var(--dot-red)]" : "text-muted-foreground"
        )}
      >
        {json}
      </pre>
      {hints.length > 0 && <ErrorHints hints={hints} />}
    </>
  );
});

// ============================================================================
// Error Hints — actionable recovery suggestions from error enrichment
// ============================================================================

/**
 * Render error hints from the framework's error enrichment system.
 *
 * These hints come from PatternEnricher and guide the LLM (or user)
 * toward self-correction. Displayed with a lightbulb icon.
 */
function ErrorHints({ hints }: { hints: string[] }) {
  return (
    <div className="mt-1.5 rounded-md bg-[var(--accent-amber)] border border-[var(--dot-amber)]/20 px-2.5 py-1.5">
      <div className="flex items-center gap-1.5 mb-1">
        <LightbulbIcon aria-hidden="true" className="size-3.5 text-[var(--dot-amber)]" />
        <span className="text-[11px] font-medium text-[var(--dot-amber)]">Suggestions</span>
      </div>
      <ul className="text-[11px] text-[var(--dot-amber)]/80 space-y-0.5 pl-5 list-disc">
        {hints.map((hint, i) => (
          <li key={i}>{hint}</li>
        ))}
      </ul>
    </div>
  );
}

// ============================================================================
// Sub-Agent Response Expand — lazy-loaded full response
// ============================================================================

function SubAgentResponseExpand({ responseId }: { responseId: string }) {
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
      const { getSubAgentResponse } = await import("~/lib/api");
      const result = await getSubAgentResponse(responseId);
      if (result._tag === "Ok") {
        setBody(result.value.body);
      } else {
        setBody("[Response expired or unavailable]");
      }
    } catch {
      setBody("[Failed to load response]");
    } finally {
      setLoading(false);
    }
  }, [expanded, body, responseId]);

  return (
    <div className="mt-2">
      <button
        type="button"
        onClick={handleExpand}
        className="text-[11px] text-[var(--dot-purple)] hover:underline flex items-center gap-1"
      >
        <ChevronDownIcon
          className={cn("size-3 transition-transform duration-150", expanded && "rotate-180")}
        />
        {expanded ? "Collapse" : "Show full response"}
      </button>
      {expanded && (
        <div className="mt-1 p-2 bg-muted/30 rounded text-xs max-h-64 overflow-y-auto scroll-container whitespace-pre-wrap font-mono">
          {loading ? (
            <Loader2Icon className="size-3 animate-spin text-muted-foreground" />
          ) : (
            (body ?? "")
          )}
        </div>
      )}
    </div>
  );
}

// ============================================================================
// Payload Copy Button — copies raw tool payload as JSON
// ============================================================================

function PayloadCopyButton({ data }: { data: unknown }) {
  const [copied, setCopied] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  const handleCopy = useCallback(() => {
    const text = stringifyToolPayload(data, 2);
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
      className={cn(
        "p-1 rounded transition-colors",
        copied
          ? "text-[var(--dot-emerald)]"
          : "text-muted-foreground/40 hover:text-muted-foreground hover:bg-muted"
      )}
      title="Copy payload"
      aria-label="Copy tool payload to clipboard"
    >
      {copied ? (
        <CheckIcon aria-hidden="true" className="size-3" />
      ) : (
        <CopyIcon aria-hidden="true" className="size-3" />
      )}
    </button>
  );
}

const TOOL_PAYLOAD_PREVIEW_LIMIT = 96;

function stringifyToolPayload(data: unknown, space?: number): string {
  if (typeof data === "string") return data;
  try {
    const json = JSON.stringify(data, null, space);
    return json === undefined ? String(data) : json;
  } catch {
    return String(data);
  }
}

function formatToolPayloadPreview(data: unknown): string {
  const compact = stringifyToolPayload(data).replace(/\s+/g, " ").trim();
  if (compact.length <= TOOL_PAYLOAD_PREVIEW_LIMIT) return compact;
  return `${compact.slice(0, TOOL_PAYLOAD_PREVIEW_LIMIT - 3)}...`;
}

// ============================================================================
// Props
// ============================================================================

interface DisplayPartDisplayProps {
  part: DisplayPart;
  isUserMessage: boolean;
}

// ============================================================================
// Text Part (Memoized)
// ============================================================================

interface TextPartProps {
  text: string;
  isUser: boolean;
}

const TextPart = memo(
  function TextPart({ text, isUser }: TextPartProps) {
    // Render user action messages as styled labels instead of raw JSON
    if (isUser && isUserActionMessage(text)) {
      const action = parseUserAction(text);
      const label = action ? getUserActionLabel(action.action) : "Action";
      return (
        <div className="flex items-center gap-2 text-sm text-primary-foreground/70 italic">
          <CheckIcon aria-hidden="true" className="size-4" />
          <span>{label}</span>
        </div>
      );
    }

    return (
      <Markdown
        text={text}
        className={cn(isUser ? "text-primary-foreground markdown-content-user" : "text-foreground")}
      />
    );
  },
  (prev, next) => prev.text === next.text && prev.isUser === next.isUser
);

// ============================================================================
// Reasoning Part (Collapsible, rendered as Markdown)
// ============================================================================

interface ReasoningPartProps {
  text: string;
}

const ReasoningPart = memo(function ReasoningPart({ text }: ReasoningPartProps) {
  const [isOpen, setIsOpen] = useState(false);
  const handleToggle = useCallback(() => setIsOpen((prev) => !prev), []);

  return (
    <div className="border border-border rounded-lg overflow-hidden">
      <button
        type="button"
        onClick={handleToggle}
        className="w-full flex items-center justify-between px-3 py-1.5 bg-muted/50 hover:bg-muted transition-colors"
      >
        <span className="text-xs font-medium text-muted-foreground flex items-center gap-1.5">
          <LightbulbIcon aria-hidden="true" className="size-3.5" />
          Reasoning
        </span>
        <ChevronDownIcon
          aria-hidden="true"
          className={cn(
            "size-3.5 text-muted-foreground/60 transition-transform duration-150",
            isOpen && "rotate-180"
          )}
        />
      </button>
      <div className={cn("collapsible-body", isOpen && "open")}>
        <div className="collapsible-inner">
          <div className="px-3 py-2 border-t border-border">
            <Markdown text={text} className="text-muted-foreground text-sm" />
          </div>
        </div>
      </div>
    </div>
  );
});

// ============================================================================
// Tool Invocation Part (Collapsible with structured result)
// ============================================================================

interface ToolInvocationPartProps {
  toolName: string;
  state: string;
  args: unknown;
  result?: unknown;
  progress?: {
    phases: readonly ToolPhase[];
    currentPhaseIndex: number;
    totalPhases: number;
  };
}

const areToolInvocationsEqual = (
  prev: ToolInvocationPartProps,
  next: ToolInvocationPartProps
): boolean => {
  if (prev.toolName !== next.toolName) return false;
  if (prev.state !== next.state) return false;
  if (prev.args !== next.args) return false;
  if (prev.result !== next.result) return false;
  if (prev.progress?.phases.length !== next.progress?.phases.length) return false;
  if (prev.progress?.currentPhaseIndex !== next.progress?.currentPhaseIndex) return false;
  if (prev.progress?.totalPhases !== next.progress?.totalPhases) return false;
  return true;
};

const isInvocationComplete = (state: string): boolean => state === "result";
const isInvocationCancelled = (state: string): boolean => state === "cancelled";
const isInvocationTerminal = (state: string): boolean =>
  isInvocationComplete(state) || isInvocationCancelled(state);
const isInvocationFailed = (state: string, result: unknown): boolean =>
  isInvocationComplete(state) && isErrorResult(result);

function ToolPayloadSection({
  label,
  data,
  children,
}: {
  label: "input" | "output";
  data: unknown;
  children: ReactNode;
}) {
  const [isOpen, setIsOpen] = useState(false);
  const preview = useMemo(() => formatToolPayloadPreview(data), [data]);
  const handleToggle = useCallback(() => setIsOpen((prev) => !prev), []);

  return (
    <div className="overflow-hidden rounded-md border border-border bg-background/40">
      <button
        type="button"
        onClick={handleToggle}
        className="flex w-full min-w-0 items-center gap-2 px-3 py-1.5 text-left transition-colors hover:bg-muted/50"
      >
        <span className="shrink-0 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
          {label}
        </span>
        <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-muted-foreground/80">
          {preview}
        </span>
        <ChevronDownIcon
          aria-hidden="true"
          className={cn(
            "size-3.5 shrink-0 text-muted-foreground/60 transition-transform duration-150",
            isOpen && "rotate-180"
          )}
        />
      </button>
      <div className={cn("collapsible-body", isOpen && "open")}>
        <div className="collapsible-inner">
          <div className="group/payload relative max-h-80 overflow-x-auto overflow-y-auto border-t border-border px-3 py-2 scroll-container">
            <div className="absolute right-1 top-1 opacity-0 transition-opacity group-hover/payload:opacity-100">
              <PayloadCopyButton data={data} />
            </div>
            {children}
          </div>
        </div>
      </div>
    </div>
  );
}

const ToolInvocationPart = memo(function ToolInvocationPart({
  toolName,
  state,
  args,
  result,
  progress,
}: ToolInvocationPartProps) {
  const [isOpen, setIsOpen] = useState(false);
  const isComplete = isInvocationComplete(state);
  const isCancelled = isInvocationCancelled(state);
  const isFailed = isInvocationFailed(state, result);
  const isTerminal = isInvocationTerminal(state);
  const { onCommandCardAction, isStreaming } = useChatContext();
  const { s } = useScramble();

  const handleToggle = useCallback(() => setIsOpen((prev) => !prev), []);

  const hasInput = args !== undefined;
  const hasOutput = result !== undefined;
  const hasCommandCardOutput = isComplete && isCommandCardToolResult(toolName, result);

  return (
    <div className="border border-border rounded-lg overflow-hidden">
      <button
        type="button"
        onClick={handleToggle}
        className="w-full flex items-center justify-between px-3 py-1.5 bg-muted/50 hover:bg-muted transition-colors"
      >
        <span className="text-xs font-medium text-foreground flex items-center gap-1.5">
          {isFailed ? (
            <XIcon aria-hidden="true" className="size-3.5 text-[var(--dot-red)]" />
          ) : isComplete ? (
            <CheckIcon aria-hidden="true" className="size-3.5 text-[var(--dot-emerald)]" />
          ) : isCancelled ? (
            <WrenchIcon aria-hidden="true" className="size-3.5 text-muted-foreground" />
          ) : (
            <Loader2Icon
              aria-hidden="true"
              className="size-3.5 text-[var(--dot-blue)] animate-spin"
            />
          )}
          <span className="font-mono">{toolName}</span>
        </span>
        <ChevronDownIcon
          aria-hidden="true"
          className={cn(
            "size-3.5 text-muted-foreground/60 transition-transform duration-150",
            isOpen && "rotate-180"
          )}
        />
      </button>
      {progress && (
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1 px-3 py-1 text-[11px] text-muted-foreground border-t border-border">
          {progress.phases.map((phase, i) => {
            const isRunning = phase.index === progress.currentPhaseIndex && !isTerminal;
            return (
              <Fragment key={phase.index}>
                {i > 0 && <span className="text-muted-foreground/40">&rsaquo;</span>}
                <span className="flex items-center gap-1">
                  {isRunning ? (
                    <Loader2Icon
                      aria-hidden="true"
                      className="size-3 animate-spin text-[var(--dot-blue)]"
                    />
                  ) : (
                    <CheckIcon aria-hidden="true" className="size-3 text-[var(--dot-emerald)]/70" />
                  )}
                  {phase.label}
                  {phase.milestone && (
                    <span className="text-muted-foreground/50">
                      ({formatMilestone(phase.milestone)})
                    </span>
                  )}
                </span>
              </Fragment>
            );
          })}
        </div>
      )}
      <div className={cn("collapsible-body", isOpen && "open")}>
        <div className="collapsible-inner">
          {(hasInput || hasOutput) && (
            <div className="space-y-1.5 border-t border-border p-2">
              {hasInput && (
                <ToolPayloadSection label="input" data={args}>
                  <StructuredResult data={args} scramble={s} />
                </ToolPayloadSection>
              )}
              {hasOutput && (
                <ToolPayloadSection label="output" data={result}>
                  {hasCommandCardOutput ? (
                    <Suspense
                      fallback={<div className="h-32 animate-pulse rounded-lg bg-muted p-4" />}
                    >
                      <CommandCardRenderer
                        dsl={result.dsl}
                        onAction={onCommandCardAction}
                        isReadOnly={isStreaming}
                      />
                    </Suspense>
                  ) : (
                    <StructuredResult data={result} scramble={s} />
                  )}
                </ToolPayloadSection>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}, areToolInvocationsEqual);

// ============================================================================
// Tool Group Display (Accordion for consecutive tool calls)
// ============================================================================

interface ToolGroupDisplayProps {
  group: ToolGroupDisplayPart;
}

const ToolGroupDisplay = memo(function ToolGroupDisplay({ group }: ToolGroupDisplayProps) {
  const [isOpen, setIsOpen] = useState(false);
  const handleToggle = useCallback(() => setIsOpen((prev) => !prev), []);

  const allComplete = group.parts.every((p) => isInvocationComplete(p.state));
  const allTerminal = group.parts.every((p) => isInvocationTerminal(p.state));
  const failedCount = group.parts.filter((p) => isInvocationFailed(p.state, p.result)).length;
  const cancelledCount = group.parts.filter((p) => isInvocationCancelled(p.state)).length;
  const runningCount = group.parts.filter((p) => !isInvocationTerminal(p.state)).length;
  const panelId = `tool-group-${group.parts[0]?.toolCallId ?? "unknown"}`;

  return (
    <div className="border border-border rounded-lg overflow-hidden">
      {/* Group header */}
      <button
        type="button"
        onClick={handleToggle}
        className="tool-group-header w-full"
        aria-expanded={isOpen}
        aria-controls={panelId}
        aria-label={`${group.parts.length} tool${group.parts.length !== 1 ? "s" : ""} — click to ${isOpen ? "collapse" : "expand"}`}
      >
        {failedCount > 0 ? (
          <XIcon aria-hidden="true" className="size-3.5 text-[var(--dot-red)] shrink-0" />
        ) : allComplete ? (
          <CheckIcon aria-hidden="true" className="size-3.5 text-[var(--dot-emerald)] shrink-0" />
        ) : allTerminal ? (
          <WrenchIcon aria-hidden="true" className="size-3.5 text-muted-foreground shrink-0" />
        ) : (
          <Loader2Icon
            aria-hidden="true"
            className="size-3.5 text-[var(--dot-blue)] animate-spin shrink-0"
          />
        )}
        <span className="text-xs font-medium text-foreground shrink-0">
          {group.parts.length} tool{group.parts.length !== 1 ? "s" : ""}
          {failedCount > 0 && (
            <span className="text-muted-foreground font-normal ml-1">({failedCount} failed)</span>
          )}
          {!allComplete && runningCount > 0 && (
            <span className="text-muted-foreground font-normal ml-1">({runningCount} running)</span>
          )}
          {failedCount === 0 && !allComplete && runningCount === 0 && cancelledCount > 0 && (
            <span className="text-muted-foreground font-normal ml-1">
              ({cancelledCount} cancelled)
            </span>
          )}
        </span>
        <div className="tool-group-pills">
          {group.parts.map((p) => {
            const stateClass = isInvocationFailed(p.state, p.result)
              ? "failed"
              : isInvocationComplete(p.state)
                ? "done"
                : isInvocationCancelled(p.state)
                  ? "cancelled"
                  : "running";
            return (
              <span key={p.toolCallId} className="tool-group-pill">
                <span className={cn("pill-dot", stateClass)} />
                {p.toolName}
              </span>
            );
          })}
        </div>
        <ChevronDownIcon
          aria-hidden="true"
          className={cn(
            "size-3.5 text-muted-foreground/60 transition-transform duration-150 shrink-0",
            isOpen && "rotate-180"
          )}
        />
      </button>

      {/* Expanded: individual tool cards */}
      <section id={panelId} className={cn("collapsible-body", isOpen && "open")}>
        <div className="collapsible-inner">
          <div className="border-t border-border p-2 space-y-1.5">
            {group.parts.map((p) => (
              <ToolInvocationPart
                key={p.toolCallId}
                toolName={p.toolName}
                state={p.state}
                args={p.args}
                result={p.result}
                progress={p.progress}
              />
            ))}
          </div>
        </div>
      </section>
    </div>
  );
});

// ============================================================================
// Tool Agent Part (Theme-aware)
// ============================================================================

function subAgentDisplayValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (value === null || value === undefined) return "";
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function extractSubAgentExchange(sourceParts?: SubAgentInvocationDisplayPart["parts"]): {
  input: string;
  response: string;
} {
  const invocation = sourceParts?.find(
    (part): part is ToolInvocationMessagePart => part.type === "tool-invocation"
  );
  return {
    input: subAgentDisplayValue(invocation?.args) || "No input captured",
    response: subAgentDisplayValue(invocation?.result) || "No response captured",
  };
}

interface ToolAgentPartProps {
  agentName: string;
  state: string;
  sourceParts?: SubAgentInvocationDisplayPart["parts"];
}

const ToolAgentPart = memo(
  function ToolAgentPart({ agentName, state, sourceParts }: ToolAgentPartProps) {
    const [isOpen, setIsOpen] = useState(false);
    const handleToggle = useCallback(() => setIsOpen((prev) => !prev), []);
    const isComplete = isInvocationComplete(state);
    const isCancelled = isInvocationCancelled(state);
    const exchange = useMemo(() => extractSubAgentExchange(sourceParts), [sourceParts]);
    const panelId = `sub-agent-${agentName}`;

    return (
      <div className="overflow-hidden rounded-lg border border-border bg-muted/30">
        <button
          type="button"
          onClick={handleToggle}
          className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-muted/50"
          aria-expanded={isOpen}
          aria-controls={panelId}
        >
          <BotIcon
            aria-hidden="true"
            className={cn(
              "size-3.5 shrink-0 text-muted-foreground",
              !isComplete && !isCancelled && "animate-pulse"
            )}
          />
          <span className="min-w-0 flex-1 truncate font-mono text-xs font-medium text-foreground">
            {agentName}
          </span>
          <span className="shrink-0 text-xs text-muted-foreground">
            {isComplete ? "completed" : isCancelled ? "cancelled" : "working..."}
          </span>
          <ChevronDownIcon
            aria-hidden="true"
            className={cn(
              "size-3.5 shrink-0 text-muted-foreground/60 transition-transform duration-150",
              isOpen && "rotate-180"
            )}
          />
        </button>
        <section id={panelId} className={cn("collapsible-body", isOpen && "open")}>
          <div className="collapsible-inner">
            <div className="space-y-2 border-t border-border px-3 py-2">
              <div>
                <div className="mb-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                  Input
                </div>
                <pre className="whitespace-pre-wrap rounded-md border border-border bg-background/40 px-3 py-2 font-mono text-xs leading-relaxed text-muted-foreground">
                  {exchange.input}
                </pre>
              </div>
              <div>
                <div className="mb-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                  Response
                </div>
                <pre className="whitespace-pre-wrap rounded-md border border-border bg-background/40 px-3 py-2 font-mono text-xs leading-relaxed text-muted-foreground">
                  {exchange.response}
                </pre>
              </div>
            </div>
          </div>
        </section>
      </div>
    );
  },
  (prev, next) =>
    prev.agentName === next.agentName &&
    prev.state === next.state &&
    prev.sourceParts === next.sourceParts
);

// ============================================================================
// File Part (Theme-aware)
// ============================================================================

interface FilePartProps {
  fileId: string;
  filename: string;
}

const FilePart = memo(
  function FilePart({ fileId, filename }: FilePartProps) {
    return (
      <a
        href={`/api/files/${fileId}/download`}
        download={filename}
        className="flex items-center gap-2 px-3 py-1.5 rounded-lg border border-[var(--dot-emerald)]/20 bg-[var(--accent-emerald)] hover:bg-[var(--dot-emerald)]/10 transition-colors"
      >
        <FileTextIcon aria-hidden="true" className="size-3.5 text-[var(--dot-emerald)]" />
        <span className="text-xs font-medium text-[var(--dot-emerald)]">{filename}</span>
        <DownloadIcon aria-hidden="true" className="size-3.5 text-[var(--dot-emerald)] ml-auto" />
      </a>
    );
  },
  (prev, next) => prev.fileId === next.fileId && prev.filename === next.filename
);

// ============================================================================
// CommandCard Display (Pre-computed approval DSL)
// ============================================================================

interface CommandCardDisplayProps {
  dsl: string;
}

const CommandCardDisplay = memo(
  function CommandCardDisplay({ dsl }: CommandCardDisplayProps) {
    const { onCommandCardAction, isStreaming } = useChatContext();

    return (
      <Suspense fallback={<div className="animate-pulse bg-muted rounded-lg p-4 h-32" />}>
        <CommandCardRenderer dsl={dsl} onAction={onCommandCardAction} isReadOnly={isStreaming} />
      </Suspense>
    );
  },
  (prev, next) => prev.dsl === next.dsl
);

// ============================================================================
// Tool Progress Part — Phase Indicator
// ============================================================================

/** Format milestone data as a compact annotation string. */
const formatMilestone = (milestone: Record<string, unknown>): string =>
  Object.entries(milestone)
    .map(([k, v]) => `${v} ${k}`)
    .join(", ");

interface ToolProgressPartProps {
  toolName: string;
  agentName?: string;
  phases: readonly ToolPhase[];
  currentPhaseIndex: number;
  totalPhases: number;
}

const areToolProgressEqual = (prev: ToolProgressPartProps, next: ToolProgressPartProps): boolean =>
  prev.toolName === next.toolName &&
  prev.agentName === next.agentName &&
  prev.currentPhaseIndex === next.currentPhaseIndex &&
  prev.phases.length === next.phases.length;

const ToolProgressDisplay = memo(function ToolProgressDisplay({
  toolName,
  agentName,
  phases,
  currentPhaseIndex,
  totalPhases,
}: ToolProgressPartProps) {
  const allPhases = useMemo(() => {
    const phaseMap = new Map(phases.map((p) => [p.index, p]));
    const result: Array<{
      index: number;
      label: string;
      status: "completed" | "running" | "pending";
      milestone?: Record<string, unknown>;
    }> = [];

    for (let i = 0; i < totalPhases; i++) {
      const phase = phaseMap.get(i);
      const status =
        i < currentPhaseIndex ? "completed" : i === currentPhaseIndex ? "running" : "pending";

      result.push({
        index: i,
        label: phase?.label ?? `Phase ${i + 1}`,
        status,
        milestone: phase?.milestone,
      });
    }
    return result;
  }, [phases, currentPhaseIndex, totalPhases]);

  return (
    <div className="rounded-lg border border-border bg-muted/30 px-4 py-3">
      <div className="text-xs font-medium text-muted-foreground mb-3 flex items-center gap-1.5">
        <WrenchIcon aria-hidden="true" className="size-3" />
        {agentName && <span className="text-[var(--dot-purple)]">{agentName}/</span>}
        {toolName}
      </div>
      <div className="flex items-start">
        {allPhases.map((phase, i) => (
          <Fragment key={phase.index}>
            {i > 0 && (
              <div
                className={cn(
                  "h-px w-8 mt-3 flex-shrink-0 transition-colors duration-300",
                  phase.status === "pending" ? "bg-border" : "bg-[var(--dot-emerald)]"
                )}
              />
            )}
            <div className="flex flex-col items-center gap-1 flex-shrink-0 min-w-0">
              <div
                className={cn(
                  "size-6 rounded-full flex items-center justify-center transition-colors duration-300",
                  phase.status === "completed" && "bg-[var(--dot-emerald)] text-white",
                  phase.status === "running" && "bg-[var(--dot-blue)] text-white",
                  phase.status === "pending" &&
                    "bg-muted border border-border text-muted-foreground"
                )}
              >
                {phase.status === "completed" && (
                  <CheckIcon aria-hidden="true" className="size-3.5" />
                )}
                {phase.status === "running" && (
                  <Loader2Icon aria-hidden="true" className="size-3.5 animate-spin" />
                )}
                {phase.status === "pending" && (
                  <span className="text-[10px] font-medium">{phase.index + 1}</span>
                )}
              </div>
              <span
                className={cn(
                  "text-[11px] leading-tight text-center max-w-[80px]",
                  phase.status === "running"
                    ? "text-[var(--dot-blue)] font-medium"
                    : "text-muted-foreground"
                )}
              >
                {phase.label}
              </span>
              {phase.milestone && (
                <span className="text-[10px] text-muted-foreground/70">
                  {formatMilestone(phase.milestone)}
                </span>
              )}
            </div>
          </Fragment>
        ))}
      </div>
    </div>
  );
}, areToolProgressEqual);

// ============================================================================
// Approval Required Part
// ============================================================================

interface ApprovalRequiredPartProps {
  approvalId: string;
  title: string;
  kind: string;
  status: string;
  payload: Record<string, unknown>;
}

const ApprovalRequiredPart = memo(
  function ApprovalRequiredPart({
    approvalId,
    title,
    kind,
    status,
    payload,
  }: ApprovalRequiredPartProps) {
    return (
      <ApprovalCard
        approval={{
          approvalId,
          status,
          payload: {
            ...payload,
            title,
            kind,
          },
        }}
        showContext={false}
      />
    );
  },
  (prev, next) =>
    prev.approvalId === next.approvalId &&
    prev.title === next.title &&
    prev.kind === next.kind &&
    prev.status === next.status &&
    prev.payload === next.payload
);

// ============================================================================
// Main Component — Handles both MessagePart and DisplayPart (with tool groups)
// ============================================================================

/**
 * Named comparator for display parts.
 * Type-specific comparison ensures minimal re-renders.
 */
const areDisplayPartsEqual = (
  prev: DisplayPartDisplayProps,
  next: DisplayPartDisplayProps
): boolean => {
  if (prev.part.type !== next.part.type) return false;
  if (prev.isUserMessage !== next.isUserMessage) return false;

  switch (prev.part.type) {
    case "text":
      return prev.part.text === (next.part as typeof prev.part).text;
    case "reasoning":
      return prev.part.text === (next.part as typeof prev.part).text;
    case "tool-invocation": {
      const nextPart = next.part as typeof prev.part;
      return (
        prev.part.toolCallId === nextPart.toolCallId &&
        prev.part.state === nextPart.state &&
        prev.part.args === nextPart.args &&
        prev.part.result === nextPart.result &&
        prev.part.progress?.phases.length === nextPart.progress?.phases.length
      );
    }
    case "tool-agent": {
      const nextPart = next.part as typeof prev.part;
      return prev.part.toolCallId === nextPart.toolCallId && prev.part.state === nextPart.state;
    }
    case "sub-agent-invocation": {
      const nextPart = next.part as typeof prev.part;
      return (
        prev.part.toolCallId === nextPart.toolCallId &&
        prev.part.agentName === nextPart.agentName &&
        prev.part.state === nextPart.state &&
        prev.part.parts.length === nextPart.parts.length
      );
    }
    case "file": {
      const nextPart = next.part as typeof prev.part;
      return prev.part.fileId === nextPart.fileId;
    }
    case "flow-ui":
      return prev.part.dsl === (next.part as typeof prev.part).dsl;
    case "approval-required": {
      const nextPart = next.part as typeof prev.part;
      return (
        prev.part.approvalId === nextPart.approvalId &&
        prev.part.title === nextPart.title &&
        prev.part.kind === nextPart.kind &&
        prev.part.status === nextPart.status &&
        prev.part.payload === nextPart.payload
      );
    }
    case "tool-progress": {
      const nextPart = next.part as typeof prev.part;
      return (
        prev.part.toolName === nextPart.toolName &&
        prev.part.currentPhaseIndex === nextPart.currentPhaseIndex &&
        prev.part.phases.length === nextPart.phases.length
      );
    }
    case "tool-group": {
      const nextPart = next.part as typeof prev.part;
      if (prev.part.parts.length !== nextPart.parts.length) return false;
      return prev.part.parts.every(
        (p, i) =>
          p.toolCallId === nextPart.parts[i].toolCallId &&
          p.state === nextPart.parts[i].state &&
          p.args === nextPart.parts[i].args &&
          p.result === nextPart.parts[i].result
      );
    }
    default:
      return prev.part === next.part;
  }
};

/**
 * Renders a single display part (MessagePart or ToolGroupDisplayPart).
 *
 * Use this with `groupParts()` for grouped tool call rendering,
 * or directly with individual MessagePart values.
 */
export const DisplayPartDisplay = memo(function DisplayPartDisplay({
  part,
  isUserMessage,
}: DisplayPartDisplayProps) {
  switch (part.type) {
    case "text":
      return <TextPart text={part.text} isUser={isUserMessage} />;

    case "reasoning":
      return <ReasoningPart text={part.text} />;

    case "tool-invocation":
      return (
        <ToolInvocationPart
          toolName={part.toolName}
          state={part.state}
          args={part.args}
          result={part.result}
          progress={part.progress}
        />
      );

    case "tool-agent":
      return <ToolAgentPart agentName={part.agentName} state={part.state} />;

    case "sub-agent-invocation":
      return (
        <ToolAgentPart agentName={part.agentName} state={part.state} sourceParts={part.parts} />
      );

    case "file":
      return <FilePart fileId={part.fileId} filename={part.filename} />;

    case "flow-ui":
      return <CommandCardDisplay dsl={part.dsl} />;

    case "approval-required":
      return (
        <ApprovalRequiredPart
          approvalId={part.approvalId}
          title={part.title}
          kind={part.kind}
          status={part.status}
          payload={part.payload}
        />
      );

    case "tool-progress":
      return (
        <ToolProgressDisplay
          toolName={part.toolName}
          agentName={part.agentName}
          phases={part.phases}
          currentPhaseIndex={part.currentPhaseIndex}
          totalPhases={part.totalPhases}
        />
      );

    case "tool-group":
      return <ToolGroupDisplay group={part} />;

    default:
      return null;
  }
}, areDisplayPartsEqual);

// ============================================================================
// Legacy Export — Backward-compatible wrapper for ungrouped parts
// ============================================================================

interface MessagePartDisplayProps {
  part: MessagePart;
  isUserMessage: boolean;
}

/**
 * Backward-compatible export for components that pass individual MessageParts.
 * Delegates to DisplayPartDisplay (MessagePart is a subtype of DisplayPart).
 */
export const MessagePartDisplay = memo(
  function MessagePartDisplay({ part, isUserMessage }: MessagePartDisplayProps) {
    return <DisplayPartDisplay part={part} isUserMessage={isUserMessage} />;
  },
  (prev, next) => {
    if (prev.part.type !== next.part.type) return false;
    if (prev.isUserMessage !== next.isUserMessage) return false;
    // Delegate to the same type-specific comparison logic
    switch (prev.part.type) {
      case "text":
        return prev.part.text === (next.part as typeof prev.part).text;
      case "reasoning":
        return prev.part.text === (next.part as typeof prev.part).text;
      case "tool-invocation": {
        const nextPart = next.part as typeof prev.part;
        return (
          prev.part.toolCallId === nextPart.toolCallId &&
          prev.part.state === nextPart.state &&
          prev.part.result === nextPart.result &&
          prev.part.progress?.phases.length === nextPart.progress?.phases.length
        );
      }
      case "tool-agent": {
        const nextPart = next.part as typeof prev.part;
        return prev.part.toolCallId === nextPart.toolCallId && prev.part.state === nextPart.state;
      }
      case "file":
        return prev.part.fileId === (next.part as typeof prev.part).fileId;
      case "flow-ui":
        return prev.part.dsl === (next.part as typeof prev.part).dsl;
      case "approval-required": {
        const nextPart = next.part as typeof prev.part;
        return (
          prev.part.approvalId === nextPart.approvalId &&
          prev.part.title === nextPart.title &&
          prev.part.kind === nextPart.kind &&
          prev.part.status === nextPart.status &&
          prev.part.payload === nextPart.payload
        );
      }
      case "tool-progress": {
        const nextPart = next.part as typeof prev.part;
        return (
          prev.part.toolName === nextPart.toolName &&
          prev.part.currentPhaseIndex === nextPart.currentPhaseIndex &&
          prev.part.phases.length === nextPart.phases.length
        );
      }
      default:
        return prev.part === next.part;
    }
  }
);
