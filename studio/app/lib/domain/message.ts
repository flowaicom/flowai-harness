/**
 * Message domain types for the chat interface.
 *
 * Three-level architecture:
 * 1. StreamPart - Raw SSE events from backend
 * 2. MessagePart - Normalized parts for display
 * 3. Message - Complete message with parts array
 *
 * @module domain/message
 */

import type { Brand } from "./brand";
import type { StreamPart, TokenUsage, ToolState } from "./stream-part";

// ============================================================================
// Message Part Types (Display-oriented)
// ============================================================================

/**
 * Text content part.
 */
export interface TextMessagePart {
  readonly type: "text";
  readonly text: string;
}

/**
 * Reasoning/thinking part (collapsible).
 */
export interface ReasoningMessagePart {
  readonly type: "reasoning";
  readonly text: string;
}

/**
 * Tool invocation display state.
 */
export type ToolInvocationState =
  | "partial-call" // Args still streaming
  | "call" // Args complete, awaiting result
  | "result" // Result received
  | "cancelled"; // Stream aborted before result

/**
 * Tool invocation part for display.
 */
export interface ToolInvocationMessagePart {
  readonly type: "tool-invocation";
  readonly toolCallId: string;
  readonly toolName: string;
  readonly args: unknown;
  readonly state: ToolInvocationState;
  readonly result?: unknown;
  /** Absorbed from a preceding tool-progress part when the result arrives. */
  readonly progress?: {
    readonly phases: readonly ToolPhase[];
    readonly currentPhaseIndex: number;
    readonly totalPhases: number;
  };
}

/**
 * Sub-agent part for display.
 */
export interface ToolAgentMessagePart {
  readonly type: "tool-agent";
  readonly toolCallId: string;
  readonly agentName: string;
  readonly state: ToolInvocationState;
  readonly usage?: TokenUsage;
}

/**
 * File attachment part.
 */
export interface FileMessagePart {
  readonly type: "file";
  readonly fileId: string;
  readonly filename: string;
}

/**
 * Pre-computed CommandCard part (approval DSL).
 *
 * Rendered directly by CommandCardRenderer without LLM involvement.
 */
export interface CommandCardPart {
  readonly type: "flow-ui";
  readonly dsl: string;
}

/**
 * Inline approval gate emitted by the harness runtime while a stream is paused.
 */
export interface ApprovalRequiredMessagePart {
  readonly type: "approval-required";
  readonly approvalId: string;
  readonly title: string;
  readonly kind: string;
  readonly status: string;
  readonly payload: Record<string, unknown>;
}

/**
 * A single phase within a tool's progress lifecycle.
 */
export interface ToolPhase {
  readonly index: number;
  readonly label: string;
  readonly milestone?: Record<string, unknown>;
}

/**
 * Accumulated tool progress state for phase rendering.
 *
 * Updated in-place: each new `tool-progress` StreamPart for the same toolName
 * replaces this part rather than appending a new one.
 */
export interface ToolProgressMessagePart {
  readonly type: "tool-progress";
  readonly toolName: string;
  /** Agent name prefix when progress originates from a sub-agent (e.g. "planner/search"). */
  readonly agentName?: string;
  readonly currentPhaseIndex: number;
  readonly totalPhases: number;
  readonly phases: readonly ToolPhase[];
}

/**
 * All possible message parts (closed sum type).
 */
export type MessagePart =
  | TextMessagePart
  | ReasoningMessagePart
  | ToolInvocationMessagePart
  | ToolAgentMessagePart
  | FileMessagePart
  | CommandCardPart
  | ApprovalRequiredMessagePart
  | ToolProgressMessagePart;

// ============================================================================
// Message Types
// ============================================================================

/**
 * Message role.
 */
export type MessageRole = "user" | "assistant" | "system";

/**
 * Unique message identifier.
 */
export type MessageId = Brand<"MessageId", string>;
export const MessageId = (s: string) => s as MessageId;

export interface ResponseContractDescriptor {
  readonly role: string;
  readonly modelRef: string;
  readonly modelName: string;
  readonly sourcePath?: string | null;
  readonly schema?: Record<string, unknown>;
}

export interface ResponseValidation {
  readonly ok: boolean;
  readonly contract: ResponseContractDescriptor;
  readonly parsed?: unknown;
  readonly errors?: readonly string[];
}

/**
 * A complete chat message.
 */
export interface Message {
  readonly id: MessageId;
  readonly role: MessageRole;
  readonly parts: MessagePart[];
  readonly createdAt: string;
  readonly runId?: string;
  readonly responseValidation?: ResponseValidation;
  /** True if message is still being streamed */
  readonly isStreaming?: boolean;
}

/**
 * Backend message format (raw from API).
 *
 * The backend may persist both `content` (plain text or legacy array)
 * and `parts` (typed MessagePart array). The `parts` field takes
 * precedence when present — see `parseBackendMessage`.
 */
export interface BackendMessage {
  readonly id?: string;
  readonly messageId?: string;
  readonly role: string;
  readonly content: unknown;
  /** Structured parts from the backend. When present, takes precedence over `content`. */
  readonly parts?: ReadonlyArray<{ readonly type: string; [key: string]: unknown }>;
  readonly createdAt?: string;
  readonly metadata?: {
    readonly parts?: ReadonlyArray<{ readonly type: string; [key: string]: unknown }>;
    readonly runId?: string;
    readonly [key: string]: unknown;
  };
  readonly responseValidation?: unknown;
}

/**
 * UI message format (already transformed by the backend).
 */
export interface UiMessage {
  readonly id: string;
  readonly role: "user" | "assistant" | "system";
  readonly parts: ReadonlyArray<{ readonly type: string; [key: string]: unknown }>;
  readonly createdAt?: string | Date;
  readonly responseValidation?: unknown;
  readonly metadata?: {
    readonly createdAt?: string | Date;
    readonly threadId?: string;
    readonly resourceId?: string;
    readonly runId?: string;
    readonly responseValidation?: unknown;
    readonly [key: string]: unknown;
  };
}

/**
 * Persisted message format discriminator from the thread API.
 */
export type PersistedMessageFormat = "backend" | "ui";

// ============================================================================
// Type Guards for MessagePart
// ============================================================================

export const isTextMessagePart = (part: MessagePart): part is TextMessagePart =>
  part.type === "text";

export const isReasoningMessagePart = (part: MessagePart): part is ReasoningMessagePart =>
  part.type === "reasoning";

export const isToolInvocationMessagePart = (part: MessagePart): part is ToolInvocationMessagePart =>
  part.type === "tool-invocation";

export const isToolAgentMessagePart = (part: MessagePart): part is ToolAgentMessagePart =>
  part.type === "tool-agent";

export const isFileMessagePart = (part: MessagePart): part is FileMessagePart =>
  part.type === "file";

export const isCommandCardPart = (part: MessagePart): part is CommandCardPart =>
  part.type === "flow-ui";

export const isApprovalRequiredMessagePart = (
  part: MessagePart
): part is ApprovalRequiredMessagePart => part.type === "approval-required";

export const isToolProgressMessagePart = (part: MessagePart): part is ToolProgressMessagePart =>
  part.type === "tool-progress";

// ============================================================================
// Backend Message Parsing
// ============================================================================

/** Known message part type discriminators. */
const KNOWN_PART_TYPES = new Set([
  "text",
  "reasoning",
  "tool-invocation",
  "tool-agent",
  "file",
  "flow-ui",
  "approval-required",
  "tool-progress",
]);

const VALID_ROLES = new Set(["user", "assistant", "system"]);
const VALID_TOOL_INVOCATION_STATES = new Set(["partial-call", "call", "result", "cancelled"]);

/** Minimal shape accepted by parseBackendMessage — structurally compatible
 * with the `BackendMessage` interface above. Defined separately so the function
 * signature doesn't depend on a specific nominal type. */
type RawBackendMessage = BackendMessage;
type RawUiMessage = UiMessage;

function normalizeRole(role: unknown): MessageRole {
  return typeof role === "string" && VALID_ROLES.has(role) ? (role as MessageRole) : "assistant";
}

function normalizeCreatedAt(fallback: string, ...candidates: unknown[]): string {
  for (const candidate of candidates) {
    if (candidate instanceof Date) return candidate.toISOString();
    if (typeof candidate === "string" && candidate.length > 0) return candidate;
  }
  return fallback;
}

function normalizeResponseValidation(candidate: unknown): ResponseValidation | undefined {
  if (!candidate || typeof candidate !== "object") return undefined;
  const record = candidate as Record<string, unknown>;
  const contract = record.contract;
  if (typeof record.ok !== "boolean" || !contract || typeof contract !== "object") {
    return undefined;
  }
  const contractRecord = contract as Record<string, unknown>;
  if (
    typeof contractRecord.role !== "string" ||
    typeof contractRecord.modelRef !== "string" ||
    typeof contractRecord.modelName !== "string"
  ) {
    return undefined;
  }

  return {
    ok: record.ok,
    contract: {
      role: contractRecord.role,
      modelRef: contractRecord.modelRef,
      modelName: contractRecord.modelName,
      sourcePath:
        typeof contractRecord.sourcePath === "string" ? contractRecord.sourcePath : undefined,
      schema:
        contractRecord.schema && typeof contractRecord.schema === "object"
          ? (contractRecord.schema as Record<string, unknown>)
          : undefined,
    },
    parsed: record.parsed,
    errors: Array.isArray(record.errors)
      ? record.errors.filter((item): item is string => typeof item === "string")
      : undefined,
  };
}

function normalizePhase(phase: unknown): ToolPhase | null {
  if (!phase || typeof phase !== "object") return null;
  const rec = phase as Record<string, unknown>;
  if (typeof rec.index !== "number" || typeof rec.label !== "string") return null;
  return {
    index: rec.index,
    label: rec.label,
    milestone:
      rec.milestone && typeof rec.milestone === "object"
        ? (rec.milestone as Record<string, unknown>)
        : undefined,
  };
}

function recordValue(candidate: unknown): Record<string, unknown> | null {
  return candidate && typeof candidate === "object" && !Array.isArray(candidate)
    ? (candidate as Record<string, unknown>)
    : null;
}

function stringValue(candidate: unknown): string | null {
  return typeof candidate === "string" && candidate.length > 0 ? candidate : null;
}

function normalizeApprovalPart(p: Record<string, unknown>): ApprovalRequiredMessagePart | null {
  const approvalId = stringValue(p.approvalId);
  if (!approvalId) return null;
  const payload = recordValue(p.payload) ?? recordValue(p.raw) ?? {};
  return {
    type: "approval-required",
    approvalId,
    title: stringValue(p.title) ?? stringValue(payload.title) ?? "Approval required",
    kind: stringValue(p.kind) ?? stringValue(payload.kind) ?? "custom",
    status: stringValue(p.status) ?? "pending",
    payload,
  };
}

function normalizeToolProgressPart(p: Record<string, unknown>): ToolProgressMessagePart {
  const currentPhaseIndex =
    typeof p.currentPhaseIndex === "number"
      ? p.currentPhaseIndex
      : typeof p.phaseIndex === "number"
        ? p.phaseIndex
        : 0;

  const normalizedPhases = Array.isArray(p.phases)
    ? p.phases.map(normalizePhase).filter((phase): phase is ToolPhase => phase !== null)
    : [];

  if (
    normalizedPhases.length === 0 &&
    typeof p.phaseIndex === "number" &&
    typeof p.label === "string"
  ) {
    normalizedPhases.push({
      index: p.phaseIndex,
      label: p.label,
      milestone:
        p.milestone && typeof p.milestone === "object"
          ? (p.milestone as Record<string, unknown>)
          : undefined,
    });
  }

  const totalPhases =
    typeof p.totalPhases === "number"
      ? p.totalPhases
      : normalizedPhases.length > 0
        ? normalizedPhases.length
        : currentPhaseIndex + 1;

  return {
    type: "tool-progress",
    toolName: p.toolName as string,
    agentName: typeof p.agentName === "string" ? p.agentName : undefined,
    currentPhaseIndex,
    totalPhases,
    phases: normalizedPhases,
  };
}

function parseStructuredPart(candidate: unknown): MessagePart | null {
  if (!candidate || typeof candidate !== "object") return null;
  const p = candidate as Record<string, unknown>;
  if (typeof p.type !== "string" || !KNOWN_PART_TYPES.has(p.type)) return null;

  switch (p.type) {
    case "text":
    case "reasoning":
      return typeof p.text === "string" ? ({ type: p.type, text: p.text } as MessagePart) : null;
    case "tool-invocation":
      if (
        typeof p.toolCallId === "string" &&
        typeof p.toolName === "string" &&
        typeof p.state === "string" &&
        VALID_TOOL_INVOCATION_STATES.has(p.state)
      ) {
        return {
          type: "tool-invocation",
          toolCallId: p.toolCallId,
          toolName: p.toolName,
          args: p.args,
          state: p.state as ToolInvocationState,
          result: p.result,
          progress:
            p.progress && typeof p.progress === "object"
              ? (p.progress as ToolInvocationMessagePart["progress"])
              : undefined,
        };
      }
      return null;
    case "tool-agent":
      if (
        typeof p.toolCallId === "string" &&
        typeof p.agentName === "string" &&
        typeof p.state === "string" &&
        VALID_TOOL_INVOCATION_STATES.has(p.state)
      ) {
        return {
          type: "tool-agent",
          toolCallId: p.toolCallId,
          agentName: p.agentName,
          state: p.state as ToolInvocationState,
          usage: p.usage && typeof p.usage === "object" ? (p.usage as TokenUsage) : undefined,
        };
      }
      return null;
    case "file":
      return typeof p.fileId === "string" && typeof p.filename === "string"
        ? {
            type: "file",
            fileId: p.fileId,
            filename: p.filename,
          }
        : null;
    case "flow-ui":
      return typeof p.dsl === "string" ? { type: "flow-ui", dsl: p.dsl } : null;
    case "approval-required":
      return normalizeApprovalPart(p);
    case "tool-progress":
      return typeof p.toolName === "string" &&
        (typeof p.currentPhaseIndex === "number" || typeof p.phaseIndex === "number")
        ? normalizeToolProgressPart(p)
        : null;
    default:
      return null;
  }
}

function parseStructuredParts(parts: ReadonlyArray<unknown>): MessagePart[] {
  return parts.map(parseStructuredPart).filter((part): part is MessagePart => part !== null);
}

/**
 * Parse a raw backend message into a typed `Message`.
 *
 * Runtime-validates the `type` discriminant of each part and the `role` field,
 * filtering out unknown types rather than silently accepting them. This is
 * the system boundary between untyped JSON and our closed sum type.
 */
export function parseBackendMessage(raw: RawBackendMessage, now?: string): Message {
  const role = normalizeRole(raw.role);

  let parts: MessagePart[];
  if (Array.isArray(raw.parts) && (raw.parts as unknown[]).length > 0) {
    parts = parseStructuredParts(raw.parts);
  } else if (Array.isArray(raw.metadata?.parts) && raw.metadata.parts.length > 0) {
    parts = parseStructuredParts(raw.metadata.parts);
  } else if (typeof raw.content === "string") {
    parts = [{ type: "text" as const, text: raw.content }];
  } else if (Array.isArray(raw.content)) {
    parts = (raw.content as Array<{ type: string; text?: string }>).filter(
      (p): p is TextMessagePart => p.type === "text" && typeof p.text === "string"
    );
  } else {
    parts = [];
  }

  const rawId = typeof raw.id === "string" ? raw.id : raw.messageId;
  return {
    id: MessageId(typeof rawId === "string" ? rawId : "message"),
    role,
    parts,
    createdAt: normalizeCreatedAt(now ?? new Date().toISOString(), raw.createdAt),
    runId: typeof raw.metadata?.runId === "string" ? raw.metadata.runId : undefined,
    responseValidation: normalizeResponseValidation(raw.responseValidation),
  };
}

/**
 * Parse a UI-formatted persisted message into a typed `Message`.
 *
 * UI messages already use display-oriented part shapes, but they still cross
 * the JSON boundary and therefore must be normalized through the same parser
 * as backend messages.
 */
export function parseUiMessage(raw: RawUiMessage, now?: string): Message {
  const role = normalizeRole(raw.role);
  const createdAt = normalizeCreatedAt(
    now ?? new Date().toISOString(),
    raw.createdAt,
    raw.metadata?.createdAt
  );

  return {
    id: MessageId(raw.id),
    role,
    parts: Array.isArray(raw.parts) ? parseStructuredParts(raw.parts) : [],
    createdAt,
    runId: typeof raw.metadata?.runId === "string" ? raw.metadata.runId : undefined,
    responseValidation: normalizeResponseValidation(
      raw.responseValidation ?? raw.metadata?.responseValidation
    ),
  };
}

/**
 * Parse persisted thread messages from the API using the declared format.
 */
export function parsePersistedMessages(
  messages: readonly unknown[],
  format: PersistedMessageFormat,
  now?: string
): Message[] {
  return messages.map((message) =>
    format === "ui"
      ? parseUiMessage(message as RawUiMessage, now)
      : parseBackendMessage(message as RawBackendMessage, now)
  );
}

// ============================================================================
// Constructors
// ============================================================================

/**
 * Create a new user message.
 */
export const createUserMessage = (id: MessageId, content: string, now: string): Message => ({
  id,
  role: "user",
  parts: [{ type: "text", text: content }],
  createdAt: now,
});

/**
 * Create a new assistant message (empty, for streaming).
 */
export const createAssistantMessage = (id: MessageId, now: string): Message => ({
  id,
  role: "assistant",
  parts: [],
  createdAt: now,
  isStreaming: true,
});

// ============================================================================
// Message Accumulator (for streaming)
// ============================================================================

/**
 * Accumulator state for building a message from stream parts.
 */
export interface MessageAccumulator {
  readonly id: MessageId;
  readonly textBuffer: string;
  readonly parts: MessagePart[];
  readonly pendingTools: Map<string, ToolInvocationMessagePart>;
  readonly pendingAgents: Map<string, ToolAgentMessagePart>;
  /** Maps toolName → index in parts[] for update-in-place progress tracking. */
  readonly progressIndices: Map<string, number>;
  /** True after any tool-invocation or tool-agent event. Prevents reasoning
   *  merges across tool call boundaries (boundary correctness). */
  readonly hadToolActivity: boolean;
}

export interface FinalizeAccumulatorOptions {
  readonly pendingState?: "cancelled";
}

/**
 * Create a new accumulator.
 */
export const createAccumulator = (id: MessageId): MessageAccumulator => ({
  id,
  textBuffer: "",
  parts: [],
  pendingTools: new Map(),
  pendingAgents: new Map(),
  progressIndices: new Map(),
  hadToolActivity: false,
});

/**
 * Map ToolState from stream to display state.
 */
const mapToolState = (state: ToolState): ToolInvocationState =>
  state === "call" ? "call" : "result";

/**
 * Parse a potentially scoped tool name ("agentName/toolName" or plain "toolName").
 *
 * Scoped names are emitted by TeeEventSink when sub-agent progress is forwarded
 * to the parent stream. The slash delimiter separates the originating agent name
 * from the actual tool name.
 */
const parseScopedToolName = (raw: string): { agentName: string | undefined; toolName: string } => {
  const idx = raw.indexOf("/");
  if (idx > 0 && idx < raw.length - 1) {
    return { agentName: raw.slice(0, idx), toolName: raw.slice(idx + 1) };
  }
  return { agentName: undefined, toolName: raw };
};

const upsertToolInvocationPart = (
  parts: readonly MessagePart[],
  toolPart: ToolInvocationMessagePart
): MessagePart[] => {
  const existingIdx = parts.findIndex(
    (part) => part.type === "tool-invocation" && part.toolCallId === toolPart.toolCallId
  );
  if (existingIdx === -1) return [...parts, toolPart];

  const existingPart = parts[existingIdx] as ToolInvocationMessagePart;
  const incomingArgsAreEmptyRecord =
    toolPart.args !== null &&
    typeof toolPart.args === "object" &&
    !Array.isArray(toolPart.args) &&
    Object.keys(toolPart.args as Record<string, unknown>).length === 0;
  const existingArgsAreEmptyRecord =
    existingPart.args !== null &&
    typeof existingPart.args === "object" &&
    !Array.isArray(existingPart.args) &&
    Object.keys(existingPart.args as Record<string, unknown>).length === 0;
  const args =
    incomingArgsAreEmptyRecord && !existingArgsAreEmptyRecord ? existingPart.args : toolPart.args;
  const result = toolPart.result !== undefined ? toolPart.result : existingPart.result;
  const progress = toolPart.progress ?? existingPart.progress;
  const next = [...parts];
  next[existingIdx] = {
    type: "tool-invocation",
    toolCallId: toolPart.toolCallId,
    toolName: toolPart.toolName,
    args,
    state: toolPart.state,
    ...(result !== undefined ? { result } : {}),
    ...(progress !== undefined ? { progress } : {}),
  };
  return next;
};

const appendApprovalPart = (
  parts: readonly MessagePart[],
  part: ApprovalRequiredMessagePart
): MessagePart[] => {
  const existingIdx = parts.findIndex(
    (candidate) =>
      candidate.type === "approval-required" && candidate.approvalId === part.approvalId
  );
  if (existingIdx === -1) return [...parts, part];

  const next = [...parts];
  next[existingIdx] = { ...(next[existingIdx] as ApprovalRequiredMessagePart), ...part };
  return next;
};

const updateApprovalStatus = (
  parts: readonly MessagePart[],
  approvalId: string,
  status: string
): MessagePart[] => {
  const existingIdx = parts.findIndex(
    (candidate) => candidate.type === "approval-required" && candidate.approvalId === approvalId
  );
  if (existingIdx === -1) return [...parts];

  const next = [...parts];
  const existing = next[existingIdx] as ApprovalRequiredMessagePart;
  next[existingIdx] = { ...existing, status };
  return next;
};

/**
 * Accumulate a stream part into the message builder.
 *
 * This is a pure function - returns new state without mutation.
 */
export const accumulatePart = (acc: MessageAccumulator, part: StreamPart): MessageAccumulator => {
  switch (part.type) {
    case "text": {
      return {
        ...acc,
        textBuffer: acc.textBuffer + part.text,
      };
    }

    case "reasoning": {
      // Merge consecutive reasoning deltas into a single part —
      // but NOT across tool call boundaries (hadToolActivity guard).
      // Without this guard, reasoning after a tool call could silently
      // merge into the reasoning block from before the tool call.
      if (!acc.textBuffer && !acc.hadToolActivity && acc.parts.length > 0) {
        const lastPart = acc.parts[acc.parts.length - 1];
        if (lastPart.type === "reasoning") {
          const newParts = [...acc.parts];
          newParts[newParts.length - 1] = {
            type: "reasoning" as const,
            text: lastPart.text + part.text,
          };
          return { ...acc, parts: newParts };
        }
      }

      // Flush text buffer if needed, then start new reasoning part.
      // Reset hadToolActivity — this new reasoning block is a fresh boundary.
      const parts = acc.textBuffer
        ? [...acc.parts, { type: "text" as const, text: acc.textBuffer }]
        : acc.parts;

      return {
        ...acc,
        textBuffer: "",
        parts: [...parts, { type: "reasoning" as const, text: part.text }],
        hadToolActivity: false,
      };
    }

    case "tool-invocation": {
      const toolPart: ToolInvocationMessagePart = {
        type: "tool-invocation",
        toolCallId: part.toolInvocationId,
        toolName: part.toolName,
        args: part.args ?? {},
        state: mapToolState(part.state),
        result: part.result,
      };

      if (part.state === "call") {
        // Store pending tool call — mark boundary for reasoning merge guard
        const pendingTools = new Map(acc.pendingTools);
        pendingTools.set(part.toolInvocationId, toolPart);
        const parts = acc.textBuffer
          ? [...acc.parts, { type: "text" as const, text: acc.textBuffer }]
          : acc.parts;
        return { ...acc, textBuffer: "", parts, pendingTools, hadToolActivity: true };
      }
      // Complete the tool call — capture pending progress before deletion
      const pendingToolPart = acc.pendingTools.get(part.toolInvocationId);
      const pendingProgress = pendingToolPart?.progress;
      const completedToolPart: ToolInvocationMessagePart = {
        ...toolPart,
        args: part.args ?? pendingToolPart?.args ?? {},
      };
      const pendingTools = new Map(acc.pendingTools);
      pendingTools.delete(part.toolInvocationId);

      // Flush text buffer if needed
      const parts = acc.textBuffer
        ? [...acc.parts, { type: "text" as const, text: acc.textBuffer }]
        : acc.parts;

      // Absorb progress: prefer standalone part in parts[], fallback to pending tool's progress
      const progressIdx = acc.progressIndices.get(part.toolName);
      if (progressIdx !== undefined) {
        const progressPart = parts[progressIdx] as ToolProgressMessagePart;
        const enrichedToolPart: ToolInvocationMessagePart = {
          ...completedToolPart,
          progress: {
            phases: progressPart.phases,
            currentPhaseIndex: progressPart.currentPhaseIndex,
            totalPhases: progressPart.totalPhases,
          },
        };
        // Remove standalone progress part, append enriched tool invocation
        const filteredParts = parts.filter((_, i) => i !== progressIdx);
        const newProgressIndices = new Map(acc.progressIndices);
        newProgressIndices.delete(part.toolName);
        for (const [name, idx] of newProgressIndices) {
          if (idx > progressIdx) {
            newProgressIndices.set(name, idx - 1);
          }
        }
        return {
          ...acc,
          textBuffer: "",
          parts: upsertToolInvocationPart(filteredParts, enrichedToolPart),
          pendingTools,
          progressIndices: newProgressIndices,
        };
      }

      // Transfer progress from pending tool (absorbed during streaming)
      if (pendingProgress) {
        const enrichedToolPart: ToolInvocationMessagePart = {
          ...completedToolPart,
          progress: pendingProgress,
        };
        return {
          ...acc,
          textBuffer: "",
          parts: upsertToolInvocationPart(parts, enrichedToolPart),
          pendingTools,
        };
      }

      return {
        ...acc,
        textBuffer: "",
        parts: upsertToolInvocationPart(parts, completedToolPart),
        pendingTools,
      };
    }

    case "tool-agent": {
      const agentPart: ToolAgentMessagePart = {
        type: "tool-agent",
        toolCallId: part.toolInvocationId,
        agentName: part.agentName,
        state: mapToolState(part.state),
      };

      if (part.state === "call") {
        const pendingAgents = new Map(acc.pendingAgents);
        pendingAgents.set(part.toolInvocationId, agentPart);
        const parts = acc.textBuffer
          ? [...acc.parts, { type: "text" as const, text: acc.textBuffer }]
          : acc.parts;
        return { ...acc, textBuffer: "", parts, pendingAgents, hadToolActivity: true };
      }
      const pendingAgents = new Map(acc.pendingAgents);
      pendingAgents.delete(part.toolInvocationId);

      const parts = acc.textBuffer
        ? [...acc.parts, { type: "text" as const, text: acc.textBuffer }]
        : acc.parts;

      return {
        ...acc,
        textBuffer: "",
        parts: [...parts, agentPart],
        pendingAgents,
      };
    }

    case "data-file-registered": {
      const filePart: FileMessagePart = {
        type: "file",
        fileId: part.data.fileId,
        filename: part.data.filename,
      };

      const parts = acc.textBuffer
        ? [...acc.parts, { type: "text" as const, text: acc.textBuffer }]
        : acc.parts;

      return {
        ...acc,
        textBuffer: "",
        parts: [...parts, filePart],
      };
    }

    case "tool-progress": {
      // Parse scoped tool names from sub-agent TeeEventSink ("agent/tool" → { agentName, toolName }).
      const { agentName: scopedAgent, toolName: bareToolName } = parseScopedToolName(part.toolName);
      const phase: ToolPhase = {
        index: part.phaseIndex,
        label: part.label,
        milestone: part.milestone,
      };

      // Prefer absorbing progress into the pending tool call (shown inline on the tool card).
      // When toolCallId is present, use direct Map lookup (O(1), precise).
      // Fallback to toolName scan for backward compatibility (matches bare or full scoped name).
      let pendingToolId: string | undefined;
      if (part.toolCallId && acc.pendingTools.has(part.toolCallId)) {
        pendingToolId = part.toolCallId;
      } else {
        for (const [id, tool] of acc.pendingTools) {
          if (tool.toolName === bareToolName || tool.toolName === part.toolName) {
            pendingToolId = id;
            break;
          }
        }
      }

      if (pendingToolId) {
        const existingTool = acc.pendingTools.get(pendingToolId)!;
        const existingPhases = existingTool.progress?.phases ?? [];
        const updatedTool: ToolInvocationMessagePart = {
          ...existingTool,
          progress: {
            phases: [...existingPhases, phase],
            currentPhaseIndex: part.phaseIndex,
            totalPhases: part.totalPhases,
          },
        };
        const pendingTools = new Map(acc.pendingTools);
        pendingTools.set(pendingToolId, updatedTool);
        return { ...acc, pendingTools };
      }

      // Fallback: standalone progress part (no pending tool).
      // Use the raw toolName as index key (preserves scoped name for uniqueness).
      const existingIdx = acc.progressIndices.get(part.toolName);

      if (existingIdx !== undefined) {
        // Update in place — replace the existing progress part
        const prevPart = acc.parts[existingIdx] as ToolProgressMessagePart;
        const updatedPart: ToolProgressMessagePart = {
          ...prevPart,
          currentPhaseIndex: part.phaseIndex,
          phases: [...prevPart.phases, phase],
        };
        const newParts = [...acc.parts];
        newParts[existingIdx] = updatedPart;
        return { ...acc, parts: newParts };
      }

      // First phase for this tool — flush text buffer, add new progress part
      const parts = acc.textBuffer
        ? [...acc.parts, { type: "text" as const, text: acc.textBuffer }]
        : acc.parts;

      const progressPart: ToolProgressMessagePart = {
        type: "tool-progress",
        toolName: bareToolName,
        agentName: scopedAgent,
        currentPhaseIndex: part.phaseIndex,
        totalPhases: part.totalPhases,
        phases: [phase],
      };

      const progressIndices = new Map(acc.progressIndices);
      progressIndices.set(part.toolName, parts.length);

      return {
        ...acc,
        textBuffer: "",
        parts: [...parts, progressPart],
        progressIndices,
      };
    }

    case "data-flow-ui": {
      const cardPart: CommandCardPart = {
        type: "flow-ui",
        dsl: part.data.dsl,
      };

      // Discard text buffer (displaySummary) — the Command Card supersedes it.
      // The backend still emits displaySummary as Text for thread replay persistence,
      // but during streaming the Card already contains all the same info.
      return {
        ...acc,
        textBuffer: "",
        parts: [...acc.parts, cardPart],
      };
    }

    case "custom": {
      if (part.name === "approval.required") {
        const approval = normalizeApprovalPart(recordValue(part.data) ?? {});
        if (!approval) return acc;

        const parts = acc.textBuffer
          ? [...acc.parts, { type: "text" as const, text: acc.textBuffer }]
          : acc.parts;
        return {
          ...acc,
          textBuffer: "",
          parts: appendApprovalPart(parts, approval),
          hadToolActivity: true,
        };
      }

      if (part.name === "approval.decision") {
        const payload = recordValue(part.data) ?? {};
        const approvalId = stringValue(payload.approvalId);
        const status = stringValue(payload.status);
        if (!approvalId || !status) return acc;
        return {
          ...acc,
          parts: updateApprovalStatus(acc.parts, approvalId, status),
        };
      }

      return acc;
    }

    case "data-tool-agent": {
      // Update agent with usage data.
      // pendingAgents is keyed by toolCallId, so we search by agentName value.
      const pendingAgents = new Map(acc.pendingAgents);
      let found = false;
      for (const [id, agent] of pendingAgents) {
        if (agent.agentName === part.data.agentName) {
          pendingAgents.set(id, { ...agent, usage: part.data.usage });
          found = true;
          break;
        }
      }
      return found ? { ...acc, pendingAgents } : acc;
    }

    case "error": {
      // Surface stream errors as visible text so the user sees them.
      // The error message from the backend is rendered as a text part
      // with an "[Error]" prefix for visual distinction.
      const errorText = `[Error] ${part.error.message}${part.error.code ? ` (${part.error.code})` : ""}`;
      const parts = acc.textBuffer
        ? [...acc.parts, { type: "text" as const, text: acc.textBuffer }]
        : acc.parts;
      return {
        ...acc,
        textBuffer: "",
        parts: [...parts, { type: "text" as const, text: errorText }],
      };
    }

    // These don't affect message content
    case "step-start":
    case "data-cost-summary":
    case "data-latency-summary":
    case "finish":
      return acc;
  }
};

/**
 * Finalize the accumulator into a message.
 */
export const finalizeAccumulator = (
  acc: MessageAccumulator,
  now?: string,
  runId?: string,
  options?: FinalizeAccumulatorOptions
): Message => {
  // Flush remaining text buffer
  let parts = acc.parts;
  if (acc.textBuffer) {
    parts = [...parts, { type: "text" as const, text: acc.textBuffer }];
  }

  // Add any pending tools/agents (shouldn't happen normally)
  for (const tool of acc.pendingTools.values()) {
    parts = [
      ...parts,
      options?.pendingState === "cancelled" ? { ...tool, state: "cancelled" as const } : tool,
    ];
  }
  for (const agent of acc.pendingAgents.values()) {
    parts = [
      ...parts,
      options?.pendingState === "cancelled" ? { ...agent, state: "cancelled" as const } : agent,
    ];
  }

  return {
    id: acc.id,
    role: "assistant",
    parts,
    createdAt: now ?? new Date().toISOString(),
    runId,
    isStreaming: false,
  };
};

// ============================================================================
// Message Utilities
// ============================================================================

/**
 * Extract plain text from message parts (total function — safe on any part type).
 */
export const extractTextContent = (parts: readonly MessagePart[]): string =>
  parts
    .filter(isTextMessagePart)
    .map((p) => p.text)
    .join("");

/**
 * Extract plain text content from a message.
 */
export const getMessageText = (message: Message): string => extractTextContent(message.parts);

/**
 * Check if message has any tool invocations.
 */
export const hasToolInvocations = (message: Message): boolean =>
  message.parts.some(isToolInvocationMessagePart);

/**
 * Check if message has any sub-agent calls.
 */
export const hasAgentCalls = (message: Message): boolean =>
  message.parts.some(isToolAgentMessagePart);

/**
 * Get all tool invocation parts from a message.
 */
export const getToolInvocations = (message: Message): ToolInvocationMessagePart[] =>
  message.parts.filter(isToolInvocationMessagePart);

/**
 * Get all file parts from a message.
 */
export const getFiles = (message: Message): FileMessagePart[] =>
  message.parts.filter(isFileMessagePart);

// ============================================================================
// Display Part Grouping (View Layer)
// ============================================================================

/**
 * A group of consecutive tool invocations/agents, collapsed into one card.
 *
 * Pure view-layer concept — the data model stays as individual parts,
 * but the rendering groups them for visual compactness.
 */
export interface ToolGroupDisplayPart {
  readonly type: "tool-group";
  readonly parts: ReadonlyArray<ToolInvocationMessagePart>;
}

/**
 * A display-only sub-agent delegation row.
 *
 * May be backed by a sub-agent lifecycle part, a `call_agent` tool invocation,
 * or both. Keeping the source parts lets view transforms stay lossless while
 * presenting delegation outside the ordinary tool-call list.
 */
export interface SubAgentInvocationDisplayPart {
  readonly type: "sub-agent-invocation";
  readonly toolCallId: string;
  readonly agentName: string;
  readonly state: ToolInvocationState;
  readonly parts: ReadonlyArray<ToolAgentMessagePart | ToolInvocationMessagePart>;
}

/**
 * Union of message parts and display-only grouping parts.
 */
export type DisplayPart = MessagePart | ToolGroupDisplayPart | SubAgentInvocationDisplayPart;

type ToolRunPart = ToolInvocationMessagePart | ToolAgentMessagePart;

interface SubAgentDisplayDraft {
  toolCallId: string;
  agentName: string;
  state: ToolInvocationState;
  parts: ToolRunPart[];
}

function stringValueFromRecord(record: unknown, keys: readonly string[]): string | null {
  if (!record || typeof record !== "object") return null;
  const obj = record as Record<string, unknown>;
  for (const key of keys) {
    const value = obj[key];
    if (typeof value === "string" && value.trim().length > 0) {
      return value;
    }
  }
  return null;
}

function agentNameFromCallAgent(part: ToolInvocationMessagePart): string | null {
  if (part.toolName !== "call_agent") return null;
  return (
    stringValueFromRecord(part.args, [
      "agent",
      "agentName",
      "agentId",
      "targetAgent",
      "targetAgentId",
      "target_agent",
      "target_agent_id",
    ]) ??
    stringValueFromRecord(part.result, [
      "agent",
      "agentName",
      "agentId",
      "targetAgent",
      "targetAgentId",
      "target_agent",
      "target_agent_id",
    ]) ??
    "sub-agent"
  );
}

function subAgentState(
  parts: readonly ToolRunPart[],
  fallback: ToolInvocationState
): ToolInvocationState {
  const lifecycle = parts.find((part): part is ToolAgentMessagePart => part.type === "tool-agent");
  if (lifecycle) return lifecycle.state;
  return parts.some((part) => part.state === "result") ? "result" : fallback;
}

function pushSubAgentDraft(
  drafts: SubAgentDisplayDraft[],
  part: ToolRunPart,
  agentName: string
): void {
  const existing = drafts.find((draft) => draft.agentName === agentName);
  if (existing) {
    existing.parts.push(part);
    if (part.type === "tool-agent") {
      existing.toolCallId = part.toolCallId;
      existing.state = subAgentState(existing.parts, part.state);
    } else {
      existing.state = subAgentState(existing.parts, existing.state);
    }
    return;
  }

  drafts.push({
    toolCallId: part.toolCallId,
    agentName,
    state: part.state,
    parts: [part],
  });
}

function pushToolRun(result: DisplayPart[], toolParts: readonly ToolInvocationMessagePart[]): void {
  if (toolParts.length === 0) return;
  if (toolParts.length === 1) {
    result.push(toolParts[0]);
    return;
  }
  result.push({ type: "tool-group", parts: toolParts });
}

/**
 * Group consecutive tool-invocation parts into collapsible groups and render
 * sub-agent delegations before those tool groups.
 *
 * Pure function — no side effects. Single tool calls pass through ungrouped.
 * Non-tool parts (text, reasoning, flow-ui, file) act as group boundaries.
 * Tool-progress parts pass through individually (they're absorbed into tool cards).
 */
export function groupParts(parts: readonly MessagePart[]): DisplayPart[] {
  const result: DisplayPart[] = [];
  let toolBuffer: ToolRunPart[] = [];

  const flush = () => {
    if (toolBuffer.length === 0) return;

    const subAgents: SubAgentDisplayDraft[] = [];
    const toolParts: ToolInvocationMessagePart[] = [];

    for (const toolPart of toolBuffer) {
      if (toolPart.type === "tool-agent") {
        pushSubAgentDraft(subAgents, toolPart, toolPart.agentName);
        continue;
      }

      const agentName = agentNameFromCallAgent(toolPart);
      if (agentName) {
        pushSubAgentDraft(subAgents, toolPart, agentName);
      } else {
        toolParts.push(toolPart);
      }
    }

    for (const subAgent of subAgents) {
      result.push({ type: "sub-agent-invocation", ...subAgent });
    }
    pushToolRun(result, toolParts);
    toolBuffer = [];
  };

  for (const part of parts) {
    if (part.type === "tool-invocation" || part.type === "tool-agent") {
      toolBuffer.push(part);
    } else if (part.type === "tool-progress") {
      // Progress parts are standalone fallbacks; don't break tool grouping
      result.push(part);
    } else {
      flush();
      result.push(part);
    }
  }
  flush();
  return result;
}
