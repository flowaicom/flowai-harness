/**
 * Message transformation and deduplication utilities.
 *
 * Multi-level deduplication strategy:
 * - Message-level deduplication by ID
 * - Part-level deduplication (tools, text, UI components)
 * - Early exit optimizations for common cases
 *
 * Design:
 * - Pure functions (referentially transparent)
 * - Single-pass where possible
 * - No side effects
 *
 * @module lib/perf/message-transformer
 */

import type { Message, MessagePart } from "~/lib/domain/message";
import {
  isTextMessagePart,
  isToolAgentMessagePart,
  isToolInvocationMessagePart,
} from "~/lib/domain/message";

// ============================================================================
// Debug Configuration
// ============================================================================

const DEBUG = import.meta.env.DEV && import.meta.env.VITE_DEBUG_MESSAGE_TRANSFORMER === "true";

function debugLog(label: string, data?: unknown): void {
  if (DEBUG) {
    console.log(`%c[MessageTransformer] ${label}`, "color: #9333ea", data);
  }
}

// ============================================================================
// Message Deduplication
// ============================================================================

/**
 * Deduplicate messages by ID, keeping the last occurrence.
 *
 * Handles Mastra's split messages (__split-N suffix).
 * Uses early exit if no duplicates detected (common case).
 */
export function deduplicateMessages(messages: Message[]): Message[] {
  if (messages.length <= 1) return messages;

  // First pass: detect if deduplication needed
  const idSet = new Set<string>();
  let hasDuplicates = false;

  for (const msg of messages) {
    const baseId = getBaseMessageId(msg.id);
    if (idSet.has(baseId)) {
      hasDuplicates = true;
      break;
    }
    idSet.add(baseId);
  }

  // Early exit: no duplicates (common case)
  if (!hasDuplicates) {
    return messages;
  }

  debugLog("Deduplicating messages", { count: messages.length });

  // Keep last occurrence of each ID
  const lastOccurrence = new Map<string, number>();
  for (let i = 0; i < messages.length; i++) {
    const baseId = getBaseMessageId(messages[i].id);
    lastOccurrence.set(baseId, i);
  }

  const result = messages.filter((msg, index) => {
    const baseId = getBaseMessageId(msg.id);
    return lastOccurrence.get(baseId) === index;
  });

  debugLog("Messages after deduplication", { before: messages.length, after: result.length });
  return result;
}

/**
 * Extract base message ID (handles __split-N suffix).
 */
function getBaseMessageId(id: string): string {
  const splitIndex = id.indexOf("__split-");
  return splitIndex === -1 ? id : id.substring(0, splitIndex);
}

// ============================================================================
// Tool Part Deduplication
// ============================================================================

/**
 * Deduplicate tool invocation parts within a message.
 *
 * Strategy:
 * 1. Primary key: toolCallId (most reliable)
 * 2. Prefer parts with results over pending parts
 */
export function deduplicateToolParts(parts: MessagePart[]): MessagePart[] {
  if (parts.length <= 1) return parts;

  // Track tool parts by ID
  const toolPartsByCallId = new Map<
    string,
    { part: MessagePart; index: number; hasResult: boolean }
  >();
  const nonToolParts: Array<{ part: MessagePart; index: number }> = [];

  for (let i = 0; i < parts.length; i++) {
    const part = parts[i];

    if (isToolInvocationMessagePart(part)) {
      const existing = toolPartsByCallId.get(part.toolCallId);

      // Prefer parts with results
      if (existing) {
        const newHasResult = part.state === "result" && part.result !== undefined;
        if (newHasResult && !existing.hasResult) {
          toolPartsByCallId.set(part.toolCallId, {
            part,
            index: i,
            hasResult: true,
          });
        }
        // Otherwise keep existing
      } else {
        toolPartsByCallId.set(part.toolCallId, {
          part,
          index: i,
          hasResult: part.state === "result" && part.result !== undefined,
        });
      }
    } else if (isToolAgentMessagePart(part)) {
      const existing = toolPartsByCallId.get(part.toolCallId);

      if (existing) {
        // Prefer completed state
        if (part.state === "result") {
          toolPartsByCallId.set(part.toolCallId, {
            part,
            index: i,
            hasResult: true,
          });
        }
      } else {
        toolPartsByCallId.set(part.toolCallId, {
          part,
          index: i,
          hasResult: part.state === "result",
        });
      }
    } else {
      nonToolParts.push({ part, index: i });
    }
  }

  // No deduplication needed
  const toolPartsArray = Array.from(toolPartsByCallId.values());
  if (toolPartsArray.length + nonToolParts.length === parts.length) {
    return parts;
  }

  // Reconstruct in original order
  const allParts = [...toolPartsArray, ...nonToolParts].sort((a, b) => a.index - b.index);
  return allParts.map((p) => p.part);
}

// ============================================================================
// Text Part Deduplication
// ============================================================================

/**
 * Deduplicate exact duplicate text parts within a message.
 *
 * Rules:
 * - Skip short text (<20 chars, likely formatting)
 * - Keep first occurrence of each unique text
 */
export function deduplicateTextParts(parts: MessagePart[]): MessagePart[] {
  const seenText = new Set<string>();
  let hasDuplicates = false;

  // First pass: detect duplicates
  for (const part of parts) {
    if (isTextMessagePart(part) && part.text.length >= 20) {
      if (seenText.has(part.text)) {
        hasDuplicates = true;
        break;
      }
      seenText.add(part.text);
    }
  }

  // Early exit: no duplicates
  if (!hasDuplicates) {
    return parts;
  }

  // Second pass: filter duplicates
  seenText.clear();
  return parts.filter((part) => {
    if (isTextMessagePart(part) && part.text.length >= 20) {
      if (seenText.has(part.text)) {
        return false;
      }
      seenText.add(part.text);
    }
    return true;
  });
}

// ============================================================================
// Message Sorting
// ============================================================================

/**
 * Sort messages by timestamp if not already sorted.
 *
 * Uses early exit if already sorted (common case).
 * Handles Mastra's split message timestamp quirks.
 */
export function sortMessagesByTimestamp(messages: Message[]): Message[] {
  if (messages.length <= 1) return messages;

  // Check if already sorted
  let isSorted = true;
  for (let i = 1; i < messages.length; i++) {
    const prevTime = new Date(messages[i - 1].createdAt).getTime();
    const currTime = new Date(messages[i].createdAt).getTime();
    if (prevTime > currTime) {
      isSorted = false;
      break;
    }
  }

  // Early exit: already sorted
  if (isSorted) {
    return messages;
  }

  debugLog("Sorting messages by timestamp", { count: messages.length });

  return [...messages].sort((a, b) => {
    const timeA = new Date(a.createdAt).getTime();
    const timeB = new Date(b.createdAt).getTime();
    return timeA - timeB;
  });
}

// ============================================================================
// Full Transformation Pipeline
// ============================================================================

export interface TransformOptions {
  /** Skip message deduplication */
  skipMessageDedup?: boolean;
  /** Skip tool part deduplication */
  skipToolDedup?: boolean;
  /** Skip text deduplication */
  skipTextDedup?: boolean;
  /** Skip timestamp sorting */
  skipSort?: boolean;
}

/**
 * Full message transformation pipeline.
 *
 * Applies all optimizations in order:
 * 1. Message deduplication
 * 2. Timestamp sorting
 * 3. Part deduplication per message
 */
export function transformMessages(messages: Message[], options: TransformOptions = {}): Message[] {
  if (messages.length === 0) return messages;

  const startTime = DEBUG ? performance.now() : 0;

  // Step 1: Message deduplication
  let result = options.skipMessageDedup ? messages : deduplicateMessages(messages);

  // Step 2: Sort by timestamp
  result = options.skipSort ? result : sortMessagesByTimestamp(result);

  // Step 3: Part deduplication per message
  if (!options.skipToolDedup || !options.skipTextDedup) {
    result = result.map((msg) => {
      let parts = msg.parts;

      if (!options.skipToolDedup) {
        parts = deduplicateToolParts(parts);
      }

      if (!options.skipTextDedup) {
        parts = deduplicateTextParts(parts);
      }

      // Only create new object if parts changed
      if (parts === msg.parts) {
        return msg;
      }

      return { ...msg, parts };
    });
  }

  if (DEBUG) {
    debugLog("Transformation complete", {
      duration: `${(performance.now() - startTime).toFixed(2)}ms`,
      inputCount: messages.length,
      outputCount: result.length,
    });
  }

  return result;
}

// ============================================================================
// Format Detection
// ============================================================================

/**
 * Detect if messages are in UI format (parts array) or backend format (content string).
 */
export function detectMessageFormat(messages: unknown[]): "ui" | "backend" | "unknown" {
  if (messages.length === 0) return "unknown";

  const first = messages[0] as Record<string, unknown>;

  if (Array.isArray(first.parts)) {
    return "ui";
  }

  if (typeof first.content === "string" || Array.isArray(first.content)) {
    return "backend";
  }

  return "unknown";
}

// ============================================================================
// Utility Exports
// ============================================================================

/**
 * Check if any messages have tool invocations.
 */
export function hasAnyToolInvocations(messages: Message[]): boolean {
  return messages.some((msg) => msg.parts.some(isToolInvocationMessagePart));
}

/**
 * Check if any messages have sub-agent calls.
 */
export function hasAnyAgentCalls(messages: Message[]): boolean {
  return messages.some((msg) => msg.parts.some(isToolAgentMessagePart));
}

/**
 * Count total parts across all messages.
 */
export function countTotalParts(messages: Message[]): number {
  return messages.reduce((sum, msg) => sum + msg.parts.length, 0);
}
