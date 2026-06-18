import {
  getUserActionLabel,
  isUserActionMessage,
  parseUserAction,
  type ToolAgentMessagePart,
  type ToolGroupDisplayPart,
  type ToolInvocationMessagePart,
  type ToolPhase,
} from "@studio/core";
import {
  BotIcon,
  CheckIcon,
  ChevronDownIcon,
  DownloadIcon,
  FileTextIcon,
  LightbulbIcon,
  Loader2Icon,
  WrenchIcon,
  XIcon,
} from "lucide-react";
import { Fragment, memo, type ReactNode, useCallback, useMemo, useState } from "react";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

export type MarkdownRenderer = (text: string, className: string) => ReactNode;

type SubAgentSourcePart = ToolAgentMessagePart | ToolInvocationMessagePart;

const isInvocationComplete = (state: string): boolean => state === "result";
const isInvocationCancelled = (state: string): boolean => state === "cancelled";
const isInvocationTerminal = (state: string): boolean =>
  isInvocationComplete(state) || isInvocationCancelled(state);
const isErrorResult = (data: unknown): boolean => {
  if (!data || typeof data !== "object") return false;
  const record = data as Record<string, unknown>;
  return record.is_error === true || record.isError === true || record.success === false;
};
const isInvocationFailed = (state: string, result: unknown): boolean =>
  isInvocationComplete(state) && isErrorResult(result);

function displayValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (value === null || value === undefined) return "";
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function extractSubAgentExchange(sourceParts?: readonly SubAgentSourcePart[]): {
  input: string;
  response: string;
} {
  const invocation = sourceParts?.find(
    (part): part is ToolInvocationMessagePart => part.type === "tool-invocation"
  );
  return {
    input: displayValue(invocation?.args) || "No input captured",
    response: displayValue(invocation?.result) || "No response captured",
  };
}

export const SharedTextPart = memo(
  function SharedTextPart({
    text,
    isUser,
    renderMarkdown,
  }: {
    readonly text: string;
    readonly isUser: boolean;
    readonly renderMarkdown: MarkdownRenderer;
  }) {
    if (isUser && isUserActionMessage(text)) {
      const action = parseUserAction(text);
      const label = action ? getUserActionLabel(action.action) : "Action";
      return (
        <div className="flex items-center gap-2 text-sm italic text-primary-foreground/70">
          <CheckIcon aria-hidden="true" className="size-4" />
          <span>{label}</span>
        </div>
      );
    }

    return renderMarkdown(
      text,
      isUser ? "text-primary-foreground markdown-content-user" : "text-foreground"
    );
  },
  (prev, next) =>
    prev.text === next.text &&
    prev.isUser === next.isUser &&
    prev.renderMarkdown === next.renderMarkdown
);

export const SharedReasoningPart = memo(
  function SharedReasoningPart({
    text,
    renderMarkdown,
  }: {
    readonly text: string;
    readonly renderMarkdown: MarkdownRenderer;
  }) {
    const [isOpen, setIsOpen] = useState(false);
    const handleToggle = useCallback(() => setIsOpen((prev) => !prev), []);

    return (
      <div className="overflow-hidden rounded-lg border border-border">
        <button
          type="button"
          onClick={handleToggle}
          className="flex w-full items-center justify-between bg-muted/50 px-3 py-1.5 transition-colors hover:bg-muted"
        >
          <span className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
            <LightbulbIcon aria-hidden="true" className="size-3.5" />
            Reasoning
          </span>
          <ChevronDownIcon
            aria-hidden="true"
            className={cx(
              "size-3.5 text-muted-foreground/60 transition-transform duration-150",
              isOpen && "rotate-180"
            )}
          />
        </button>
        <div className={cx("collapsible-body", isOpen && "open")}>
          <div className="collapsible-inner">
            <div className="border-t border-border px-3 py-2">
              {renderMarkdown(text, "text-muted-foreground text-sm")}
            </div>
          </div>
        </div>
      </div>
    );
  },
  (prev, next) => prev.text === next.text && prev.renderMarkdown === next.renderMarkdown
);

export const SharedSubAgentInvocationPart = memo(
  function SharedSubAgentInvocationPart({
    agentName,
    state,
    sourceParts,
  }: {
    readonly agentName: string;
    readonly state: string;
    readonly sourceParts?: readonly SubAgentSourcePart[];
  }) {
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
            className={cx(
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
            className={cx(
              "size-3.5 shrink-0 text-muted-foreground/60 transition-transform duration-150",
              isOpen && "rotate-180"
            )}
          />
        </button>
        <section id={panelId} className={cx("collapsible-body", isOpen && "open")}>
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

export const SharedToolAgentPart = SharedSubAgentInvocationPart;

export const SharedFilePart = memo(
  function SharedFilePart({
    fileId,
    filename,
  }: {
    readonly fileId: string;
    readonly filename: string;
  }) {
    return (
      <a
        href={`/api/files/${fileId}/download`}
        download={filename}
        className="flex items-center gap-2 rounded-lg border border-[var(--dot-emerald)]/20 bg-[var(--accent-emerald)] px-3 py-1.5 transition-colors hover:bg-[var(--dot-emerald)]/10"
      >
        <FileTextIcon aria-hidden="true" className="size-3.5 text-[var(--dot-emerald)]" />
        <span className="text-xs font-medium text-[var(--dot-emerald)]">{filename}</span>
        <DownloadIcon aria-hidden="true" className="ml-auto size-3.5 text-[var(--dot-emerald)]" />
      </a>
    );
  },
  (prev, next) => prev.fileId === next.fileId && prev.filename === next.filename
);

const formatMilestone = (milestone: Record<string, unknown>): string =>
  Object.entries(milestone)
    .map(([key, value]) => `${value} ${key}`)
    .join(", ");

export const SharedToolProgressDisplay = memo(
  function SharedToolProgressDisplay({
    toolName,
    agentName,
    phases,
    currentPhaseIndex,
    totalPhases,
  }: {
    readonly toolName: string;
    readonly agentName?: string;
    readonly phases: readonly ToolPhase[];
    readonly currentPhaseIndex: number;
    readonly totalPhases: number;
  }) {
    const allPhases = useMemo(() => {
      const phaseMap = new Map(phases.map((phase) => [phase.index, phase]));
      const result: Array<{
        index: number;
        label: string;
        status: "completed" | "running" | "pending";
        milestone?: Record<string, unknown>;
      }> = [];

      for (let index = 0; index < totalPhases; index += 1) {
        const phase = phaseMap.get(index);
        const status =
          index < currentPhaseIndex
            ? "completed"
            : index === currentPhaseIndex
              ? "running"
              : "pending";

        result.push({
          index,
          label: phase?.label ?? `Phase ${index + 1}`,
          status,
          milestone: phase?.milestone,
        });
      }

      return result;
    }, [currentPhaseIndex, phases, totalPhases]);

    return (
      <div className="rounded-lg border border-border bg-muted/30 px-4 py-3">
        <div className="mb-3 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
          <WrenchIcon aria-hidden="true" className="size-3" />
          {agentName ? <span className="text-[var(--dot-purple)]">{agentName}/</span> : null}
          {toolName}
        </div>
        <div className="flex items-start">
          {allPhases.map((phase, index) => (
            <Fragment key={phase.index}>
              {index > 0 ? (
                <div
                  className={cx(
                    "mt-3 h-px w-8 shrink-0 transition-colors duration-300",
                    phase.status === "pending" ? "bg-border" : "bg-[var(--dot-emerald)]"
                  )}
                />
              ) : null}
              <div className="flex min-w-0 shrink-0 flex-col items-center gap-1">
                <div
                  className={cx(
                    "flex size-6 items-center justify-center rounded-full transition-colors duration-300",
                    phase.status === "completed" && "bg-[var(--dot-emerald)] text-white",
                    phase.status === "running" && "bg-[var(--dot-blue)] text-white",
                    phase.status === "pending" &&
                      "border border-border bg-muted text-muted-foreground"
                  )}
                >
                  {phase.status === "completed" ? (
                    <CheckIcon aria-hidden="true" className="size-3.5" />
                  ) : phase.status === "running" ? (
                    <Loader2Icon aria-hidden="true" className="size-3.5 animate-spin" />
                  ) : (
                    <span className="text-[10px] font-medium">{phase.index + 1}</span>
                  )}
                </div>
                <span
                  className={cx(
                    "max-w-[80px] text-center text-[11px] leading-tight",
                    phase.status === "running"
                      ? "font-medium text-[var(--dot-blue)]"
                      : "text-muted-foreground"
                  )}
                >
                  {phase.label}
                </span>
                {phase.milestone ? (
                  <span className="text-[10px] text-muted-foreground/70">
                    {formatMilestone(phase.milestone)}
                  </span>
                ) : null}
              </div>
            </Fragment>
          ))}
        </div>
      </div>
    );
  },
  (prev, next) =>
    prev.toolName === next.toolName &&
    prev.agentName === next.agentName &&
    prev.currentPhaseIndex === next.currentPhaseIndex &&
    prev.phases.length === next.phases.length
);

export const SharedToolGroupDisplay = memo(function SharedToolGroupDisplay({
  group,
  isStreaming,
  renderToolInvocation,
}: {
  readonly group: ToolGroupDisplayPart;
  readonly isStreaming?: boolean;
  readonly renderToolInvocation: (
    part: ToolInvocationMessagePart,
    key: string,
    context: { readonly isStreaming?: boolean }
  ) => ReactNode;
}) {
  const [isOpen, setIsOpen] = useState(false);
  const handleToggle = useCallback(() => setIsOpen((prev) => !prev), []);

  const toolParts = group.parts;
  const allComplete = toolParts.every((part) => isInvocationComplete(part.state));
  const allTerminal = toolParts.every((part) => isInvocationTerminal(part.state));
  const failedCount = toolParts.filter((part) => isInvocationFailed(part.state, part.result)).length;
  const cancelledCount = toolParts.filter((part) => isInvocationCancelled(part.state)).length;
  const runningCount = toolParts.filter((part) => !isInvocationTerminal(part.state)).length;
  const panelId = `tool-group-${group.parts[0]?.toolCallId ?? "unknown"}`;

  return (
    <div className="overflow-hidden rounded-lg border border-border">
      <button
        type="button"
        onClick={handleToggle}
        className="tool-group-header w-full"
        aria-expanded={isOpen}
        aria-controls={panelId}
        aria-label={`${group.parts.length} tool${group.parts.length !== 1 ? "s" : ""} — click to ${isOpen ? "collapse" : "expand"}`}
      >
        {failedCount > 0 ? (
          <XIcon aria-hidden="true" className="size-3.5 shrink-0 text-[var(--dot-red)]" />
        ) : allComplete ? (
          <CheckIcon aria-hidden="true" className="size-3.5 shrink-0 text-[var(--dot-emerald)]" />
        ) : allTerminal ? (
          <WrenchIcon aria-hidden="true" className="size-3.5 shrink-0 text-muted-foreground" />
        ) : (
          <Loader2Icon
            aria-hidden="true"
            className="size-3.5 shrink-0 animate-spin text-[var(--dot-blue)]"
          />
        )}
        <span className="shrink-0 text-xs font-medium text-foreground">
          {group.parts.length} tool{group.parts.length !== 1 ? "s" : ""}
          {failedCount > 0 ? (
            <span className="ml-1 font-normal text-muted-foreground">({failedCount} failed)</span>
          ) : null}
          {!allComplete && runningCount > 0 ? (
            <span className="ml-1 font-normal text-muted-foreground">({runningCount} running)</span>
          ) : null}
          {failedCount === 0 && !allComplete && runningCount === 0 && cancelledCount > 0 ? (
            <span className="ml-1 font-normal text-muted-foreground">
              ({cancelledCount} cancelled)
            </span>
          ) : null}
        </span>
        <div className="tool-group-pills">
          {toolParts.map((part) => {
            const stateClass = isInvocationFailed(part.state, part.result)
              ? "failed"
              : isInvocationComplete(part.state)
                ? "done"
                : isInvocationCancelled(part.state)
                  ? "cancelled"
                  : "running";
            return (
              <span key={part.toolCallId} className="tool-group-pill">
                <span className={cx("pill-dot", stateClass)} />
                {part.toolName}
              </span>
            );
          })}
        </div>
        <ChevronDownIcon
          aria-hidden="true"
          className={cx(
            "size-3.5 shrink-0 text-muted-foreground/60 transition-transform duration-150",
            isOpen && "rotate-180"
          )}
        />
      </button>

      <section id={panelId} className={cx("collapsible-body", isOpen && "open")}>
        <div className="collapsible-inner">
          <div className="space-y-1.5 border-t border-border p-2">
            {toolParts.map((part) => renderToolInvocation(part, part.toolCallId, { isStreaming }))}
          </div>
        </div>
      </section>
    </div>
  );
});
