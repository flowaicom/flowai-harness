import { memo } from "react";

import type { ThreadRunReport, ThreadRunToolCall } from "~/lib/api";
import { cn } from "~/lib/utils";

interface RunReportCardProps {
  readonly report: ThreadRunReport;
}

function statusClass(status: ThreadRunReport["status"]): string {
  switch (status) {
    case "completed":
      return "bg-[var(--accent-emerald)] text-[var(--dot-emerald)]";
    case "error":
      return "bg-[var(--accent-red)] text-[var(--dot-red)]";
    default:
      return "bg-muted text-muted-foreground";
  }
}

function formatMs(value: number | undefined): string {
  if (value === undefined || Number.isNaN(value)) return "-";
  return `${Math.round(value)}ms`;
}

function toolKey(tool: ThreadRunToolCall): string {
  return `${tool.toolCallId}:${tool.toolName}:${tool.index}`;
}

export const RunReportCard = memo(function RunReportCard({ report }: RunReportCardProps) {
  const latency = report.latency;
  const usage = report.usage;
  const responseValidation = report.responseValidation;
  const topTools = report.toolCalls.slice(0, 6);
  const topSubAgents = report.subAgents.slice(0, 4);

  return (
    <div className="mt-4 rounded-xl border bg-card/80 p-4">
      <div className="flex flex-wrap items-center gap-2">
        <div className="text-[11px] font-medium uppercase tracking-[0.12em] text-muted-foreground">
          Last Run
        </div>
        <span
          className={cn(
            "rounded-full px-2 py-0.5 text-[11px] font-medium",
            statusClass(report.status)
          )}
        >
          {report.status}
        </span>
        {report.finishReason && (
          <span className="rounded-full bg-muted px-2 py-0.5 text-[11px] text-muted-foreground">
            {report.finishReason}
          </span>
        )}
        {responseValidation && (
          <span
            className={cn(
              "rounded-full px-2 py-0.5 text-[11px] font-medium",
              responseValidation.ok
                ? "bg-[var(--accent-emerald)] text-[var(--dot-emerald)]"
                : "bg-[var(--accent-red)] text-[var(--dot-red)]"
            )}
          >
            {responseValidation.ok ? "contract ok" : "contract failed"}
          </span>
        )}
      </div>

      <div className="mt-3 grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
        <Metric label="Role" value={report.role} />
        <Metric label="Model" value={report.model} mono />
        <Metric label="Total" value={formatMs(latency?.totalDurationMs)} />
        <Metric label="TTFT" value={formatMs(latency?.ttftMs)} />
        <Metric label="First Text" value={formatMs(latency?.firstTextMs)} />
        <Metric label="LLM Calls" value={String(latency?.phases.llmCalls ?? 0)} />
        <Metric label="Tokens" value={usage ? String(usage.totalTokens) : "-"} />
        <Metric label="Delegations" value={String(report.delegationChain.length)} />
      </div>

      {report.delegationChain.length > 0 && (
        <div className="mt-4">
          <div className="text-xs font-medium">Delegation Chain</div>
          <div className="mt-1 text-sm text-muted-foreground">
            {report.delegationChain.join(" -> ")}
          </div>
        </div>
      )}

      {topSubAgents.length > 0 && (
        <div className="mt-4">
          <div className="text-xs font-medium">Sub-agents</div>
          <div className="mt-2 space-y-1.5">
            {topSubAgents.map((agent) => (
              <div
                key={`${agent.invocationId}:${agent.agentName}`}
                className="flex items-center justify-between gap-3 rounded-lg border px-3 py-2 text-sm"
              >
                <div className="font-medium">{agent.agentName}</div>
                <div className="text-muted-foreground">
                  {agent.usage ? `${agent.usage.totalTokens} tokens` : agent.state}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {topTools.length > 0 && (
        <div className="mt-4">
          <div className="text-xs font-medium">Tool Timeline</div>
          <div className="mt-2 space-y-1.5">
            {topTools.map((tool) => (
              <div key={toolKey(tool)} className="rounded-lg border px-3 py-2">
                <div className="flex items-center justify-between gap-3">
                  <div className="text-sm font-medium">{tool.toolName}</div>
                  <div className="text-xs text-muted-foreground">
                    {tool.status}
                    {tool.durationMs !== undefined ? ` · ${formatMs(tool.durationMs)}` : ""}
                  </div>
                </div>
                {tool.progress && tool.progress.phases.length > 0 && (
                  <div className="mt-1 text-xs text-muted-foreground">
                    {tool.progress.phases.map((phase) => phase.label).join(" -> ")}
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {report.error && (
        <div className="mt-4 rounded-lg bg-[var(--accent-red)] px-3 py-2 text-sm text-[var(--dot-red)]">
          {report.error.message}
          {report.error.code ? ` [${report.error.code}]` : ""}
        </div>
      )}

      {responseValidation?.errors && responseValidation.errors.length > 0 && (
        <div className="mt-3 rounded-lg bg-[var(--accent-red)] px-3 py-2 text-sm text-[var(--dot-red)]">
          {responseValidation.errors.join("; ")}
        </div>
      )}

      {report.outputPreview && (
        <div className="mt-4">
          <div className="text-xs font-medium">Output Preview</div>
          <div className="mt-1 whitespace-pre-wrap rounded-lg bg-muted/60 px-3 py-2 text-sm text-muted-foreground">
            {report.outputPreview}
          </div>
        </div>
      )}
    </div>
  );
});

function Metric({
  label,
  value,
  mono = false,
}: {
  readonly label: string;
  readonly value: string;
  readonly mono?: boolean;
}) {
  return (
    <div className="rounded-lg border px-3 py-2">
      <div className="text-[11px] uppercase tracking-[0.08em] text-muted-foreground">{label}</div>
      <div className={cn("mt-1 text-sm font-medium", mono && "font-mono text-[12px] break-all")}>
        {value}
      </div>
    </div>
  );
}
