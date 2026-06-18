import type { RunEventEnvelope, RunSummary } from "~/lib/api/runs";

export type RunTimelineItemKind =
  | "tool"
  | "subAgent"
  | "approval"
  | "lifecycle"
  | "data"
  | "unknown";
export type RunTimelineStatus = "started" | "completed" | "failed" | "pending";

export interface RunTimelineItem {
  readonly id: string;
  readonly kind: RunTimelineItemKind;
  readonly eventKind: string;
  readonly label: string;
  readonly status: RunTimelineStatus;
  readonly seq: number;
  readonly completedSeq?: number;
  readonly input?: unknown;
  readonly output?: unknown;
  readonly rawEvents: readonly RunEventEnvelope[];
}

const TERMINAL_RUN_STATUSES = new Set(["completed", "failed", "cancelled", "canceled"]);
export const STALE_RUNNING_RUN_MS = 5 * 60 * 1000;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function stringValue(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function eventPayload(event: RunEventEnvelope): Record<string, unknown> {
  const normalized = isRecord(event.event) ? event.event : {};
  return isRecord(normalized.payload) ? normalized.payload : normalized;
}

function eventRaw(event: RunEventEnvelope): Record<string, unknown> {
  return isRecord(event.raw) ? event.raw : {};
}

function eventInput(event: RunEventEnvelope): unknown {
  const payload = eventPayload(event);
  const raw = eventRaw(event);
  return payload.arguments ?? payload.input ?? raw.args ?? raw.arguments;
}

function eventOutput(event: RunEventEnvelope): unknown {
  const payload = eventPayload(event);
  const raw = eventRaw(event);
  return payload.result ?? payload.output ?? raw.result ?? raw.output ?? payload.error;
}

function toolCallId(event: RunEventEnvelope): string | undefined {
  const payload = eventPayload(event);
  const raw = eventRaw(event);
  return (
    stringValue(payload.toolCallId) ??
    stringValue(raw.toolCallId) ??
    stringValue(raw.toolInvocationId)
  );
}

function toolName(event: RunEventEnvelope): string {
  const payload = eventPayload(event);
  const raw = eventRaw(event);
  return (
    stringValue(payload.toolName) ??
    stringValue(raw.toolName) ??
    stringValue(raw.tool_name) ??
    "tool"
  );
}

function subAgentName(event: RunEventEnvelope): string {
  const payload = eventPayload(event);
  const raw = eventRaw(event);
  return (
    stringValue(payload.targetAgentId) ??
    stringValue(raw.agentName) ??
    stringValue(raw.agent_name) ??
    "sub-agent"
  );
}

function subAgentInput(event: RunEventEnvelope): unknown {
  const payload = eventPayload(event);
  const raw = eventRaw(event);
  return payload.message ?? raw.prompt ?? raw.message;
}

function standaloneStatus(kind: string): RunTimelineStatus {
  const lower = kind.toLowerCase();
  if (lower.includes("failed") || lower.includes("error")) return "failed";
  if (lower.includes("completed") || lower.includes("finish")) return "completed";
  if (lower.includes("required") || lower.includes("pending")) return "pending";
  return "started";
}

function itemTypeForKind(kind: string): RunTimelineItemKind {
  if (kind.startsWith("data.")) return "data";
  if (kind.startsWith("runtime.") || kind.startsWith("eval") || kind.startsWith("testCase")) {
    return "lifecycle";
  }
  return "unknown";
}

function labelForStandalone(event: RunEventEnvelope): string {
  const payload = eventPayload(event);
  if (event.kind === "approval.required") {
    return stringValue(payload.title) ?? stringValue(payload.kind) ?? "Approval required";
  }
  if (event.kind === "runtime.finish") return "Runtime finished";
  if (event.kind === "run.failed") return "Run failed";
  return event.kind;
}

function updateStatus(current: RunTimelineStatus, next: RunTimelineStatus): RunTimelineStatus {
  if (next === "failed" || current === "failed") return "failed";
  if (next === "completed") return "completed";
  return next;
}

export function projectRunTimeline(events: readonly RunEventEnvelope[]): RunTimelineItem[] {
  const byPairKey = new Map<string, RunTimelineItem>();
  const items: RunTimelineItem[] = [];

  const sorted = [...events].sort((a, b) => a.seq - b.seq);

  for (const event of sorted) {
    if (event.kind === "tool.call.started") {
      const id = toolCallId(event) ?? `seq-${event.seq}`;
      const key = `tool:${id}`;
      const existing = byPairKey.get(key);
      const next: RunTimelineItem = {
        id: key,
        kind: "tool",
        eventKind: event.kind,
        label: toolName(event),
        status: existing?.status ?? "pending",
        seq: existing?.seq ?? event.seq,
        completedSeq: existing?.completedSeq,
        input: eventInput(event),
        output: existing?.output,
        rawEvents: [...(existing?.rawEvents ?? []), event],
      };
      byPairKey.set(key, next);
      if (!existing) items.push(next);
      else items[items.indexOf(existing)] = next;
      continue;
    }

    if (event.kind === "tool.call.completed") {
      const id = toolCallId(event) ?? `seq-${event.seq}`;
      const key = `tool:${id}`;
      const existing = byPairKey.get(key);
      const next: RunTimelineItem = {
        id: key,
        kind: "tool",
        eventKind: event.kind,
        label: toolName(event),
        status: updateStatus(existing?.status ?? "pending", "completed"),
        seq: existing?.seq ?? event.seq,
        completedSeq: event.seq,
        input: existing?.input,
        output: eventOutput(event),
        rawEvents: [...(existing?.rawEvents ?? []), event],
      };
      byPairKey.set(key, next);
      if (!existing) items.push(next);
      else items[items.indexOf(existing)] = next;
      continue;
    }

    if (event.kind === "sub_agent.call.started") {
      const id = toolCallId(event) ?? `seq-${event.seq}`;
      const key = `subAgent:${id}`;
      const existing = byPairKey.get(key);
      const next: RunTimelineItem = {
        id: key,
        kind: "subAgent",
        eventKind: event.kind,
        label: subAgentName(event),
        status: existing?.status ?? "pending",
        seq: existing?.seq ?? event.seq,
        completedSeq: existing?.completedSeq,
        input: subAgentInput(event),
        output: existing?.output,
        rawEvents: [...(existing?.rawEvents ?? []), event],
      };
      byPairKey.set(key, next);
      if (!existing) items.push(next);
      else items[items.indexOf(existing)] = next;
      continue;
    }

    if (event.kind === "sub_agent.call.completed") {
      const id = toolCallId(event) ?? `seq-${event.seq}`;
      const key = `subAgent:${id}`;
      const existing = byPairKey.get(key);
      const next: RunTimelineItem = {
        id: key,
        kind: "subAgent",
        eventKind: event.kind,
        label: subAgentName(event),
        status: updateStatus(existing?.status ?? "pending", "completed"),
        seq: existing?.seq ?? event.seq,
        completedSeq: event.seq,
        input: existing?.input,
        output: eventOutput(event),
        rawEvents: [...(existing?.rawEvents ?? []), event],
      };
      byPairKey.set(key, next);
      if (!existing) items.push(next);
      else items[items.indexOf(existing)] = next;
      continue;
    }

    const item: RunTimelineItem = {
      id: `${event.kind}:${event.seq}`,
      kind: event.kind === "approval.required" ? "approval" : itemTypeForKind(event.kind),
      eventKind: event.kind,
      label: labelForStandalone(event),
      status: standaloneStatus(event.kind),
      seq: event.seq,
      input: event.kind === "approval.required" ? eventPayload(event).raw : undefined,
      output: event.kind === "run.failed" ? eventOutput(event) : undefined,
      rawEvents: [event],
    };
    items.push(item);
  }

  return items.sort((a, b) => a.seq - b.seq);
}

export function isTerminalRunStatus(status: string | null | undefined): boolean {
  return status ? TERMINAL_RUN_STATUSES.has(status.toLowerCase()) : false;
}

export function isStaleRunningRun(
  run: Pick<RunSummary, "status" | "updatedAt"> | null | undefined,
  nowMs = Date.now(),
  staleAfterMs = STALE_RUNNING_RUN_MS
): boolean {
  if (!run || run.status.toLowerCase() !== "running") return false;
  const updatedMs = Date.parse(run.updatedAt);
  if (!Number.isFinite(updatedMs)) return false;
  return nowMs - updatedMs > staleAfterMs;
}

export function isEffectivelyTerminalRun(
  run: Pick<RunSummary, "status" | "updatedAt"> | null | undefined,
  nowMs = Date.now()
): boolean {
  return isTerminalRunStatus(run?.status) || isStaleRunningRun(run, nowMs);
}

export function runHasTerminalEvent(events: readonly RunEventEnvelope[]): boolean {
  return events.some((event) => event.kind === "runtime.finish" || event.kind === "run.failed");
}

export function latestChatRunForThread(
  runs: readonly RunSummary[],
  threadId: string
): RunSummary | null {
  const matches = runs.filter((run) => run.operation === "chat" && run.threadId === threadId);
  matches.sort((a, b) => Date.parse(b.updatedAt) - Date.parse(a.updatedAt));
  return matches[0] ?? null;
}
