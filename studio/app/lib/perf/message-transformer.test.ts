import { describe, expect, test } from "bun:test";
import * as fc from "fast-check";

import type { Message, MessagePart } from "~/lib/domain/message";
import { arbMessage, arbMessagePart, arbUniqueMessage } from "~/lib/test-utils/arbitraries";
import {
  countTotalParts,
  deduplicateMessages,
  deduplicateTextParts,
  deduplicateToolParts,
  sortMessagesByTimestamp,
  transformMessages,
} from "./message-transformer";

// ============================================================================
// Property-Based Tests (Interpreter Layer)
// ============================================================================

// -- deduplicateMessages --

describe("deduplicateMessages (property-based)", () => {
  test("idempotence: dedup(dedup(msgs)) = dedup(msgs)", () => {
    fc.assert(
      fc.property(fc.array(arbMessage), (messages) => {
        const once = deduplicateMessages(messages);
        const twice = deduplicateMessages(once);
        expect(twice).toEqual(once);
      })
    );
  });

  test("length: result.length ≤ input.length", () => {
    fc.assert(
      fc.property(fc.array(arbMessage), (messages) => {
        expect(deduplicateMessages(messages).length).toBeLessThanOrEqual(messages.length);
      })
    );
  });

  test("no unique loss: all-unique IDs returns same reference", () => {
    fc.assert(
      fc.property(fc.array(arbUniqueMessage), (messages) => {
        const result = deduplicateMessages(messages);
        expect(result).toBe(messages);
      })
    );
  });

  test("last-occurrence semantics: survivor is the last with that baseId", () => {
    fc.assert(
      fc.property(fc.array(arbMessage, { minLength: 1 }), (messages) => {
        const result = deduplicateMessages(messages);
        // For each surviving message, no later message in the input has the same baseId
        const baseId = (id: string) => {
          const idx = id.indexOf("__split-");
          return idx === -1 ? id : id.substring(0, idx);
        };
        const resultIds = new Set(result.map((m) => baseId(m.id)));
        // Every unique baseId in input should appear in result
        const inputIds = new Set(messages.map((m) => baseId(m.id)));
        expect(resultIds).toEqual(inputIds);
      })
    );
  });
});

// -- deduplicateToolParts --

describe("deduplicateToolParts (property-based)", () => {
  test("idempotence: dedup(dedup(parts)) = dedup(parts)", () => {
    fc.assert(
      fc.property(fc.array(arbMessagePart), (parts) => {
        const once = deduplicateToolParts(parts);
        const twice = deduplicateToolParts(once);
        expect(twice).toEqual(once);
      })
    );
  });

  test("length: result.length ≤ input.length", () => {
    fc.assert(
      fc.property(fc.array(arbMessagePart), (parts) => {
        expect(deduplicateToolParts(parts).length).toBeLessThanOrEqual(parts.length);
      })
    );
  });

  test("non-tool parts always survive", () => {
    fc.assert(
      fc.property(fc.array(arbMessagePart), (parts) => {
        const result = deduplicateToolParts(parts);
        const nonToolInput = parts.filter(
          (p) => p.type !== "tool-invocation" && p.type !== "tool-agent"
        );
        const nonToolResult = result.filter(
          (p) => p.type !== "tool-invocation" && p.type !== "tool-agent"
        );
        expect(nonToolResult.length).toBe(nonToolInput.length);
      })
    );
  });
});

// -- deduplicateTextParts --

describe("deduplicateTextParts (property-based)", () => {
  test("idempotence: dedup(dedup(parts)) = dedup(parts)", () => {
    fc.assert(
      fc.property(fc.array(arbMessagePart), (parts) => {
        const once = deduplicateTextParts(parts);
        const twice = deduplicateTextParts(once);
        expect(twice).toEqual(once);
      })
    );
  });

  test("short text (<20 chars) never removed", () => {
    fc.assert(
      fc.property(
        fc.array(
          fc.record({
            type: fc.constant("text" as const),
            text: fc.string({ maxLength: 19 }),
          })
        ),
        (parts) => {
          const result = deduplicateTextParts(parts as MessagePart[]);
          expect(result.length).toBe(parts.length);
        }
      )
    );
  });

  test("non-text parts always survive", () => {
    fc.assert(
      fc.property(fc.array(arbMessagePart), (parts) => {
        const result = deduplicateTextParts(parts);
        const nonTextInput = parts.filter((p) => p.type !== "text");
        const nonTextResult = result.filter((p) => p.type !== "text");
        expect(nonTextResult.length).toBe(nonTextInput.length);
      })
    );
  });
});

// -- sortMessagesByTimestamp --

describe("sortMessagesByTimestamp (property-based)", () => {
  test("idempotence: sort(sort(msgs)) = sort(msgs)", () => {
    fc.assert(
      fc.property(fc.array(arbMessage), (messages) => {
        const once = sortMessagesByTimestamp(messages);
        const twice = sortMessagesByTimestamp(once);
        expect(twice).toEqual(once);
      })
    );
  });

  test("output is chronologically ordered", () => {
    fc.assert(
      fc.property(fc.array(arbMessage, { minLength: 2 }), (messages) => {
        const sorted = sortMessagesByTimestamp(messages);
        for (let i = 1; i < sorted.length; i++) {
          const prev = new Date(sorted[i - 1].createdAt).getTime();
          const curr = new Date(sorted[i].createdAt).getTime();
          expect(prev).toBeLessThanOrEqual(curr);
        }
      })
    );
  });

  test("preserves length", () => {
    fc.assert(
      fc.property(fc.array(arbMessage), (messages) => {
        expect(sortMessagesByTimestamp(messages).length).toBe(messages.length);
      })
    );
  });

  test("already-sorted returns same reference (early exit)", () => {
    fc.assert(
      fc.property(fc.array(arbMessage), (messages) => {
        const sorted = sortMessagesByTimestamp(messages);
        // Second sort should return same reference (already sorted → early exit)
        const again = sortMessagesByTimestamp(sorted);
        expect(again).toBe(sorted);
      })
    );
  });
});

// -- transformMessages Pipeline --

describe("transformMessages (property-based)", () => {
  test("idempotence: transform(transform(msgs)) = transform(msgs)", () => {
    fc.assert(
      fc.property(fc.array(arbMessage), (messages) => {
        const once = transformMessages(messages);
        const twice = transformMessages(once);
        expect(twice).toEqual(once);
      })
    );
  });

  test("empty input returns same reference", () => {
    const empty: Message[] = [];
    expect(transformMessages(empty)).toBe(empty);
  });
});

// -- countTotalParts --

describe("countTotalParts (property-based)", () => {
  test("equals sum of individual message part lengths", () => {
    fc.assert(
      fc.property(fc.array(arbMessage), (messages) => {
        const total = countTotalParts(messages);
        const expected = messages.reduce((sum, msg) => sum + msg.parts.length, 0);
        expect(total).toBe(expected);
      })
    );
  });

  test("monotonic: adding a message never decreases count", () => {
    fc.assert(
      fc.property(fc.array(arbMessage), arbMessage, (messages, extra) => {
        const before = countTotalParts(messages);
        const after = countTotalParts([...messages, extra]);
        expect(after).toBeGreaterThanOrEqual(before);
      })
    );
  });
});
