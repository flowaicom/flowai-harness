import type {
  AgentSummary,
  StudioEvent,
  ThreadMessage,
  ThreadSummary,
  WorkspaceSummary,
} from "@studio/core/runtime";
import type { UiMessage } from "~/lib/domain/message";
import type { StreamPart } from "~/lib/domain/stream-part";
import { tokenUsageZero } from "~/lib/domain/stream-part";
import type { Thread } from "~/lib/domain/thread";
import type { Workspace } from "~/lib/domain/workspace";

function asRecord(input: unknown): Record<string, unknown> {
  return input && typeof input === "object" && !Array.isArray(input)
    ? (input as Record<string, unknown>)
    : {};
}

function asString(input: unknown, fallback = ""): string {
  return typeof input === "string" ? input : fallback;
}

function asMessageParts(
  input: unknown
): ReadonlyArray<{ readonly type: string; readonly [key: string]: unknown }> | null {
  if (!Array.isArray(input)) return null;
  const parts = input.filter(
    (part): part is { readonly type: string; readonly [key: string]: unknown } =>
      !!part &&
      typeof part === "object" &&
      !Array.isArray(part) &&
      typeof (part as { type?: unknown }).type === "string"
  );
  return parts.length > 0 ? parts : null;
}

function isEmptyRecordValue(input: unknown): boolean {
  return (
    !!input &&
    typeof input === "object" &&
    !Array.isArray(input) &&
    Object.keys(input as Record<string, unknown>).length === 0
  );
}

function mergeLifecyclePart(
  existing: { readonly type: string; readonly [key: string]: unknown },
  incoming: { readonly type: string; readonly [key: string]: unknown }
): { readonly type: string; readonly [key: string]: unknown } {
  if (incoming.type !== "tool-invocation") return { ...existing, ...incoming };

  const incomingArgs = incoming.args;
  const existingArgs = existing.args;
  const keepExistingArgs = isEmptyRecordValue(incomingArgs) && !isEmptyRecordValue(existingArgs);
  return {
    ...existing,
    ...incoming,
    args: keepExistingArgs ? existingArgs : incomingArgs,
  };
}

function dedupeMessageParts(
  parts: ReadonlyArray<{ readonly type: string; readonly [key: string]: unknown }>
): ReadonlyArray<{ readonly type: string; readonly [key: string]: unknown }> {
  const output: Array<{ readonly type: string; readonly [key: string]: unknown }> = [];
  const lifecyclePartIndices = new Map<string, number>();

  for (const part of parts) {
    const toolCallId = typeof part.toolCallId === "string" ? part.toolCallId : null;
    if ((part.type === "tool-invocation" || part.type === "tool-agent") && toolCallId) {
      const key = `${part.type}:${toolCallId}`;
      const existingIdx = lifecyclePartIndices.get(key);
      if (existingIdx !== undefined) {
        output[existingIdx] = mergeLifecyclePart(output[existingIdx], part);
        continue;
      }
      lifecyclePartIndices.set(key, output.length);
    }
    output.push(part);
  }

  return output;
}

export function workspaceSummaryToWorkspace(summary: WorkspaceSummary): Workspace {
  const now = new Date().toISOString();
  return {
    id: summary.workspaceKey,
    displayName: summary.displayName || summary.workspaceKey,
    createdAt: asString(summary.metadata?.createdAt, now),
    updatedAt: asString(summary.metadata?.updatedAt, now),
    databases: [],
    bundle: {
      requiredRoles: [],
      configuredRoles: [],
      missingRoles: [],
      status: "complete",
      complete: true,
    },
  };
}

export function threadSummaryToThread(summary: ThreadSummary, resourceId: string): Thread {
  return {
    id: summary.id,
    title: summary.title,
    resourceId,
    createdAt: summary.updatedAt,
    updatedAt: summary.updatedAt,
  };
}

export function createLocalThread(threadId: string, resourceId: string, title?: string): Thread {
  const now = new Date().toISOString();
  return {
    id: threadId,
    title: title ?? "New Conversation",
    resourceId,
    createdAt: now,
    updatedAt: now,
  };
}

export function threadMessageToUiMessage(message: ThreadMessage): UiMessage {
  const metadataParts = asMessageParts(message.metadata?.parts);
  const parts = metadataParts
    ? dedupeMessageParts(metadataParts)
    : message.content
      ? [{ type: "text", text: message.content }]
      : [];
  return {
    id: message.messageId,
    role:
      message.role === "user" || message.role === "assistant" || message.role === "system"
        ? message.role
        : "system",
    parts,
    createdAt: message.createdAt,
    metadata: {
      ...message.metadata,
      threadId: message.threadId,
    },
  };
}

export function pickChatAgentId(agents: readonly AgentSummary[]): string | null {
  const entrypointCoordinator = agents.find(
    (agent) => agent.entrypoint && agent.role === "coordinator"
  );
  const entrypoint = agents.find((agent) => agent.entrypoint);
  const coordinator = agents.find((agent) => agent.role === "coordinator");
  return (entrypointCoordinator ?? entrypoint ?? coordinator ?? agents[0])?.agentId ?? null;
}

export function studioEventToStreamPart(event: StudioEvent): StreamPart | null {
  const payload = asRecord(event.payload);
  switch (event.kind) {
    case "message.delta": {
      const text = asString(payload.text);
      return text ? { type: "text", text } : null;
    }
    case "tool.call.started":
      return {
        type: "tool-invocation",
        toolInvocationId: asString(payload.toolCallId, `${event.runId}:tool`),
        toolName: asString(payload.toolName, "tool"),
        args: payload.arguments ?? {},
        state: "call",
      };
    case "tool.call.completed":
      return {
        type: "tool-invocation",
        toolInvocationId: asString(payload.toolCallId, `${event.runId}:tool`),
        toolName: asString(payload.toolName, "tool"),
        args: payload.arguments,
        state: "result",
        result: payload.result,
      };
    case "sub_agent.call.started": {
      const agentName = asString(payload.targetAgentId, "sub-agent");
      const toolInvocationId = asString(
        payload.toolCallId ?? payload.toolInvocationId,
        `${event.runId}:${agentName}`
      );
      return {
        type: "tool-agent",
        agentName,
        toolInvocationId,
        state: "call",
      };
    }
    case "sub_agent.call.completed": {
      const agentName = asString(payload.targetAgentId, "sub-agent");
      const toolInvocationId = asString(
        payload.toolCallId ?? payload.toolInvocationId,
        `${event.runId}:${agentName}`
      );
      return {
        type: "tool-agent",
        agentName,
        toolInvocationId,
        state: "result",
      };
    }
    case "approval.required":
      return { type: "custom", name: "approval.required", data: payload };
    case "approval.decision":
      return { type: "custom", name: "approval.decision", data: payload };
    case "run.completed":
    case "run.cancelled":
    case "run.aborted":
      return { type: "finish", finishReason: "stop", usage: tokenUsageZero };
    case "run.failed": {
      const error = asRecord(payload.error);
      return {
        type: "error",
        error: {
          message: asString(error.message, "Runtime stream failed."),
          code: asString(error.code, "runtime.error"),
        },
      };
    }
    case "runtime.event":
      return { type: "custom", name: event.kind, data: payload };
    default:
      return null;
  }
}
