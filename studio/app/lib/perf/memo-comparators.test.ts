import { describe, expect, test } from "bun:test";
import * as fc from "fast-check";

import {
  compareById,
  compareByIdAndState,
  compareByIdStateAnd,
  compareByState,
  compareByText,
  compareMessageProps,
  compareMetricCard,
  compareStreamingParts,
  compareSubAgentRow,
  compareToolAgent,
  compareToolInvocation,
  createPropsComparator,
  withStreamingOverride,
} from "./memo-comparators";

// ============================================================================
// Basic Comparators
// ============================================================================

describe("compareById", () => {
  test("reflexivity: same object returns true", () => {
    const obj = { id: "abc" };
    expect(compareById(obj, obj)).toBe(true);
  });

  test("symmetry: compare(a, b) === compare(b, a)", () => {
    const a = { id: "x" };
    const b = { id: "y" };
    expect(compareById(a, b)).toBe(compareById(b, a));
  });

  test("returns true for equal ids", () => {
    expect(compareById({ id: "1" }, { id: "1" })).toBe(true);
  });

  test("returns false for different ids", () => {
    expect(compareById({ id: "1" }, { id: "2" })).toBe(false);
  });

  test("ignores other props", () => {
    const a = { id: "1", name: "Alice" };
    const b = { id: "1", name: "Bob" };
    expect(compareById(a, b)).toBe(true);
  });
});

describe("compareByText", () => {
  test("reflexivity: same object returns true", () => {
    const obj = { text: "hello" };
    expect(compareByText(obj, obj)).toBe(true);
  });

  test("symmetry: compare(a, b) === compare(b, a)", () => {
    const a = { text: "foo" };
    const b = { text: "bar" };
    expect(compareByText(a, b)).toBe(compareByText(b, a));
  });

  test("returns true for equal text", () => {
    expect(compareByText({ text: "hi" }, { text: "hi" })).toBe(true);
  });

  test("returns false for different text", () => {
    expect(compareByText({ text: "hi" }, { text: "bye" })).toBe(false);
  });

  test("handles empty strings", () => {
    expect(compareByText({ text: "" }, { text: "" })).toBe(true);
    expect(compareByText({ text: "" }, { text: "x" })).toBe(false);
  });
});

describe("compareByState", () => {
  test("reflexivity: same object returns true", () => {
    const obj = { state: "active" as const };
    expect(compareByState(obj, obj)).toBe(true);
  });

  test("symmetry: compare(a, b) === compare(b, a)", () => {
    const a = { state: "active" as const };
    const b = { state: "idle" as const };
    expect(compareByState(a, b)).toBe(compareByState(b, a));
  });

  test("returns true for equal states", () => {
    expect(compareByState({ state: "loading" }, { state: "loading" })).toBe(true);
  });

  test("returns false for different states", () => {
    expect(compareByState({ state: "loading" }, { state: "done" })).toBe(false);
  });
});

// ============================================================================
// Combined Comparators
// ============================================================================

describe("compareByIdAndState", () => {
  test("reflexivity: same object returns true", () => {
    const obj = { id: "1", state: "active" as const };
    expect(compareByIdAndState(obj, obj)).toBe(true);
  });

  test("returns true when both id and state match", () => {
    expect(compareByIdAndState({ id: "1", state: "open" }, { id: "1", state: "open" })).toBe(true);
  });

  test("returns false when id differs", () => {
    expect(compareByIdAndState({ id: "1", state: "open" }, { id: "2", state: "open" })).toBe(false);
  });

  test("returns false when state differs", () => {
    expect(compareByIdAndState({ id: "1", state: "open" }, { id: "1", state: "closed" })).toBe(
      false
    );
  });
});

describe("compareByIdStateAnd", () => {
  test("calls predicate when id and state match", () => {
    let predicateCalled = false;
    const comparator = compareByIdStateAnd<{
      id: string;
      state: string;
      extra: number;
    }>((prev, next) => {
      predicateCalled = true;
      return prev.extra === next.extra;
    });

    comparator({ id: "1", state: "a", extra: 10 }, { id: "1", state: "a", extra: 10 });
    expect(predicateCalled).toBe(true);
  });

  test("short-circuits on id mismatch without calling predicate", () => {
    let predicateCalled = false;
    const comparator = compareByIdStateAnd<{
      id: string;
      state: string;
    }>(() => {
      predicateCalled = true;
      return true;
    });

    const result = comparator({ id: "1", state: "a" }, { id: "2", state: "a" });
    expect(result).toBe(false);
    expect(predicateCalled).toBe(false);
  });

  test("short-circuits on state mismatch without calling predicate", () => {
    let predicateCalled = false;
    const comparator = compareByIdStateAnd<{
      id: string;
      state: string;
    }>(() => {
      predicateCalled = true;
      return true;
    });

    const result = comparator({ id: "1", state: "a" }, { id: "1", state: "b" });
    expect(result).toBe(false);
    expect(predicateCalled).toBe(false);
  });
});

// ============================================================================
// Message Comparators
// ============================================================================

describe("compareMessageProps", () => {
  test("returns true for identical stable messages", () => {
    const parts = [{ type: "text", text: "hello" }];
    const a = { id: "1", isStreaming: false, parts };
    expect(compareMessageProps(a, a)).toBe(true);
  });

  test("returns false when ids differ", () => {
    const parts = [{ type: "text" }];
    expect(
      compareMessageProps(
        { id: "1", isStreaming: false, parts },
        { id: "2", isStreaming: false, parts }
      )
    ).toBe(false);
  });

  test("returns false when streaming state changes", () => {
    const parts = [{ type: "text" }];
    expect(
      compareMessageProps(
        { id: "1", isStreaming: true, parts },
        { id: "1", isStreaming: false, parts }
      )
    ).toBe(false);
  });

  test("always returns false for streaming messages", () => {
    const parts = [{ type: "text" }];
    const msg = { id: "1", isStreaming: true, parts };
    // Even the same object returns false when streaming
    expect(compareMessageProps(msg, msg)).toBe(false);
  });

  test("compares parts by reference for stable messages", () => {
    const parts1 = [{ type: "text" }];
    const parts2 = [{ type: "text" }]; // same content, different reference
    expect(
      compareMessageProps(
        { id: "1", isStreaming: false, parts: parts1 },
        { id: "1", isStreaming: false, parts: parts2 }
      )
    ).toBe(false);
  });

  test("handles undefined isStreaming", () => {
    const parts = [{ type: "text" }];
    expect(compareMessageProps({ id: "1", parts }, { id: "1", parts })).toBe(true);
  });
});

describe("compareStreamingParts", () => {
  test("different lengths returns false", () => {
    expect(
      compareStreamingParts(
        { parts: [{ type: "text" }] },
        { parts: [{ type: "text" }, { type: "text" }] }
      )
    ).toBe(false);
  });

  test("empty arrays are equal", () => {
    expect(compareStreamingParts({ parts: [] }, { parts: [] })).toBe(true);
  });

  test("compares last part type", () => {
    expect(
      compareStreamingParts(
        { parts: [{ type: "text", text: "a" }] },
        { parts: [{ type: "tool-call" }] }
      )
    ).toBe(false);
  });

  test("compares last part text content for text parts", () => {
    expect(
      compareStreamingParts(
        { parts: [{ type: "text", text: "hello" }] },
        { parts: [{ type: "text", text: "hello" }] }
      )
    ).toBe(true);

    expect(
      compareStreamingParts(
        { parts: [{ type: "text", text: "hello" }] },
        { parts: [{ type: "text", text: "world" }] }
      )
    ).toBe(false);
  });

  test("non-text parts use reference equality", () => {
    const part = { type: "tool-call" };
    expect(compareStreamingParts({ parts: [part] }, { parts: [part] })).toBe(true);

    expect(
      compareStreamingParts({ parts: [{ type: "tool-call" }] }, { parts: [{ type: "tool-call" }] })
    ).toBe(false);
  });
});

// ============================================================================
// Tool Display Comparators
// ============================================================================

describe("compareToolInvocation", () => {
  test("reflexivity: same object returns true", () => {
    const obj = {
      part: { toolCallId: "t1", toolName: "search", state: "done" },
    };
    expect(compareToolInvocation(obj, obj)).toBe(true);
  });

  test("returns true for matching fields", () => {
    expect(
      compareToolInvocation(
        { part: { toolCallId: "t1", toolName: "search", state: "done" } },
        { part: { toolCallId: "t1", toolName: "search", state: "done" } }
      )
    ).toBe(true);
  });

  test("returns false when state differs", () => {
    expect(
      compareToolInvocation(
        { part: { toolCallId: "t1", toolName: "search", state: "pending" } },
        { part: { toolCallId: "t1", toolName: "search", state: "done" } }
      )
    ).toBe(false);
  });

  test("returns false when toolCallId differs", () => {
    expect(
      compareToolInvocation(
        { part: { toolCallId: "t1", toolName: "search", state: "done" } },
        { part: { toolCallId: "t2", toolName: "search", state: "done" } }
      )
    ).toBe(false);
  });

  test("returns false when toolName differs", () => {
    expect(
      compareToolInvocation(
        { part: { toolCallId: "t1", toolName: "search", state: "done" } },
        { part: { toolCallId: "t1", toolName: "query", state: "done" } }
      )
    ).toBe(false);
  });

  test("ignores result field", () => {
    expect(
      compareToolInvocation(
        {
          part: {
            toolCallId: "t1",
            toolName: "search",
            state: "done",
            result: { data: 1 },
          },
        },
        {
          part: {
            toolCallId: "t1",
            toolName: "search",
            state: "done",
            result: { data: 2 },
          },
        }
      )
    ).toBe(true);
  });
});

describe("compareToolAgent", () => {
  test("reflexivity: same object returns true", () => {
    const obj = {
      part: { toolCallId: "t1", agentName: "researcher", state: "active" },
    };
    expect(compareToolAgent(obj, obj)).toBe(true);
  });

  test("returns true for matching fields", () => {
    expect(
      compareToolAgent(
        { part: { toolCallId: "t1", agentName: "researcher", state: "active" } },
        { part: { toolCallId: "t1", agentName: "researcher", state: "active" } }
      )
    ).toBe(true);
  });

  test("returns false when agentName differs", () => {
    expect(
      compareToolAgent(
        { part: { toolCallId: "t1", agentName: "researcher", state: "active" } },
        { part: { toolCallId: "t1", agentName: "planner", state: "active" } }
      )
    ).toBe(false);
  });
});

// ============================================================================
// Latency Panel Comparators
// ============================================================================

describe("compareMetricCard", () => {
  test("reflexivity: same object returns true", () => {
    const obj = { label: "TTFB", value: 120 };
    expect(compareMetricCard(obj, obj)).toBe(true);
  });

  test("returns true for equal label and value", () => {
    expect(
      compareMetricCard({ label: "TTFB", value: "120ms" }, { label: "TTFB", value: "120ms" })
    ).toBe(true);
  });

  test("returns false for different values", () => {
    expect(compareMetricCard({ label: "TTFB", value: 100 }, { label: "TTFB", value: 200 })).toBe(
      false
    );
  });

  test("handles null and undefined values", () => {
    expect(compareMetricCard({ label: "TTFB", value: null }, { label: "TTFB", value: null })).toBe(
      true
    );

    expect(
      compareMetricCard({ label: "TTFB", value: undefined }, { label: "TTFB", value: undefined })
    ).toBe(true);

    expect(
      compareMetricCard({ label: "TTFB", value: null }, { label: "TTFB", value: undefined })
    ).toBe(false);
  });
});

describe("compareSubAgentRow", () => {
  test("reflexivity: same object returns true", () => {
    const obj = { agentName: "sql", durationMs: 500, status: "done" };
    expect(compareSubAgentRow(obj, obj)).toBe(true);
  });

  test("returns true for matching fields", () => {
    expect(
      compareSubAgentRow(
        { agentName: "sql", durationMs: 500, status: "done" },
        { agentName: "sql", durationMs: 500, status: "done" }
      )
    ).toBe(true);
  });

  test("returns false when durationMs differs", () => {
    expect(
      compareSubAgentRow(
        { agentName: "sql", durationMs: 500, status: "done" },
        { agentName: "sql", durationMs: 600, status: "done" }
      )
    ).toBe(false);
  });

  test("handles undefined optional fields", () => {
    expect(compareSubAgentRow({ agentName: "sql" }, { agentName: "sql" })).toBe(true);

    expect(
      compareSubAgentRow({ agentName: "sql", durationMs: undefined }, { agentName: "sql" })
    ).toBe(true);
  });
});

// ============================================================================
// Generic Comparators
// ============================================================================

describe("createPropsComparator", () => {
  test("compares only specified keys", () => {
    const comparator = createPropsComparator<{
      id: string;
      name: string;
      age: number;
    }>(["id", "name"]);

    expect(
      comparator({ id: "1", name: "Alice", age: 30 }, { id: "1", name: "Alice", age: 99 })
    ).toBe(true);
  });

  test("returns false when a specified key differs", () => {
    const comparator = createPropsComparator<{
      id: string;
      name: string;
    }>(["id", "name"]);

    expect(comparator({ id: "1", name: "Alice" }, { id: "1", name: "Bob" })).toBe(false);
  });

  test("uses Object.is semantics (NaN equals NaN)", () => {
    const comparator = createPropsComparator<{ value: number }>(["value"]);

    expect(comparator({ value: NaN }, { value: NaN })).toBe(true);
    expect(comparator({ value: 0 }, { value: -0 })).toBe(false);
  });

  test("reflexivity: same object returns true", () => {
    const comparator = createPropsComparator<{ a: string; b: number }>(["a", "b"]);
    const obj = { a: "x", b: 1 };
    expect(comparator(obj, obj)).toBe(true);
  });

  test("empty keys array always returns true", () => {
    const comparator = createPropsComparator<{ x: number }>([]);
    expect(comparator({ x: 1 }, { x: 2 })).toBe(true);
  });
});

describe("withStreamingOverride", () => {
  const baseComparator = (
    prev: { isStreaming?: boolean; id: string },
    next: { isStreaming?: boolean; id: string }
  ) => prev.id === next.id;

  test("returns false when next is streaming", () => {
    const comparator = withStreamingOverride(baseComparator);
    expect(comparator({ id: "1", isStreaming: false }, { id: "1", isStreaming: true })).toBe(false);
  });

  test("returns false when streaming state changes", () => {
    const comparator = withStreamingOverride(baseComparator);
    expect(comparator({ id: "1", isStreaming: true }, { id: "1", isStreaming: false })).toBe(false);
  });

  test("delegates to base comparator when not streaming", () => {
    const comparator = withStreamingOverride(baseComparator);
    expect(comparator({ id: "1", isStreaming: false }, { id: "1", isStreaming: false })).toBe(true);

    expect(comparator({ id: "1", isStreaming: false }, { id: "2", isStreaming: false })).toBe(
      false
    );
  });

  test("handles undefined isStreaming as not streaming", () => {
    const comparator = withStreamingOverride(baseComparator);
    expect(comparator({ id: "1" }, { id: "1" })).toBe(true);
  });
});

// ============================================================================
// Generator DSL (Description Layer)
// ============================================================================

const arbId = fc.record({ id: fc.string() });
const arbText = fc.record({ text: fc.string() });
const arbState = fc.record({ state: fc.string() });
const arbIdState = fc.record({ id: fc.string(), state: fc.string() });

const arbToolInvProps = fc.record({
  part: fc.record({
    toolCallId: fc.string(),
    toolName: fc.string(),
    state: fc.string(),
    result: fc.option(fc.anything(), { nil: undefined }),
  }),
});

const arbToolAgentProps = fc.record({
  part: fc.record({
    toolCallId: fc.string(),
    agentName: fc.string(),
    state: fc.string(),
  }),
});

const arbMetricCard = fc.record({
  label: fc.string(),
  value: fc.oneof(fc.string(), fc.integer(), fc.constant(null), fc.constant(undefined)),
});

const arbSubAgentRow = fc.record({
  agentName: fc.string(),
  durationMs: fc.option(fc.nat(), { nil: undefined }),
  status: fc.option(fc.string(), { nil: undefined }),
});

// ============================================================================
// Property-Based Tests (Interpreter Layer)
// ============================================================================

describe("comparator reflexivity (property-based)", () => {
  test("compareById: reflexive for any id", () => {
    fc.assert(fc.property(arbId, (x) => expect(compareById(x, x)).toBe(true)));
  });

  test("compareByText: reflexive for any text", () => {
    fc.assert(fc.property(arbText, (x) => expect(compareByText(x, x)).toBe(true)));
  });

  test("compareByState: reflexive for any state", () => {
    fc.assert(fc.property(arbState, (x) => expect(compareByState(x, x)).toBe(true)));
  });

  test("compareByIdAndState: reflexive", () => {
    fc.assert(fc.property(arbIdState, (x) => expect(compareByIdAndState(x, x)).toBe(true)));
  });

  test("compareToolInvocation: reflexive", () => {
    fc.assert(fc.property(arbToolInvProps, (x) => expect(compareToolInvocation(x, x)).toBe(true)));
  });

  test("compareToolAgent: reflexive", () => {
    fc.assert(fc.property(arbToolAgentProps, (x) => expect(compareToolAgent(x, x)).toBe(true)));
  });

  test("compareMetricCard: reflexive", () => {
    fc.assert(fc.property(arbMetricCard, (x) => expect(compareMetricCard(x, x)).toBe(true)));
  });

  test("compareSubAgentRow: reflexive", () => {
    fc.assert(fc.property(arbSubAgentRow, (x) => expect(compareSubAgentRow(x, x)).toBe(true)));
  });
});

describe("comparator symmetry (property-based)", () => {
  test("compareById: symmetric", () => {
    fc.assert(
      fc.property(arbId, arbId, (a, b) => expect(compareById(a, b)).toBe(compareById(b, a)))
    );
  });

  test("compareByText: symmetric", () => {
    fc.assert(
      fc.property(arbText, arbText, (a, b) => expect(compareByText(a, b)).toBe(compareByText(b, a)))
    );
  });

  test("compareByIdAndState: symmetric", () => {
    fc.assert(
      fc.property(arbIdState, arbIdState, (a, b) =>
        expect(compareByIdAndState(a, b)).toBe(compareByIdAndState(b, a))
      )
    );
  });

  test("compareToolInvocation: symmetric", () => {
    fc.assert(
      fc.property(arbToolInvProps, arbToolInvProps, (a, b) =>
        expect(compareToolInvocation(a, b)).toBe(compareToolInvocation(b, a))
      )
    );
  });

  test("compareMetricCard: symmetric", () => {
    fc.assert(
      fc.property(arbMetricCard, arbMetricCard, (a, b) =>
        expect(compareMetricCard(a, b)).toBe(compareMetricCard(b, a))
      )
    );
  });
});

describe("createPropsComparator (property-based)", () => {
  test("reflexive for any key set", () => {
    fc.assert(
      fc.property(fc.record({ a: fc.string(), b: fc.integer(), c: fc.boolean() }), (obj) => {
        const cmp = createPropsComparator<typeof obj>(["a", "b", "c"]);
        expect(cmp(obj, obj)).toBe(true);
      })
    );
  });

  test("only checks named keys — extra keys ignored", () => {
    fc.assert(
      fc.property(fc.string(), fc.string(), fc.string(), (id, extra1, extra2) => {
        const cmp = createPropsComparator<Record<string, unknown>>(["id"]);
        const a = { id, extra: extra1 };
        const b = { id, extra: extra2 };
        expect(cmp(a, b)).toBe(true);
      })
    );
  });
});

describe("withStreamingOverride (property-based)", () => {
  test("delegates to base when neither prev nor next is streaming", () => {
    fc.assert(
      fc.property(fc.string(), fc.string(), (idA, idB) => {
        const base = (
          prev: { isStreaming?: boolean; id: string },
          next: { isStreaming?: boolean; id: string }
        ) => prev.id === next.id;
        const wrapped = withStreamingOverride(base);
        const prev = { id: idA, isStreaming: false };
        const next = { id: idB, isStreaming: false };
        expect(wrapped(prev, next)).toBe(base(prev, next));
      })
    );
  });

  test("always returns false when next.isStreaming", () => {
    fc.assert(
      fc.property(fc.string(), fc.string(), (idA, idB) => {
        const base = (
          prev: { isStreaming?: boolean; id: string },
          next: { isStreaming?: boolean; id: string }
        ) => prev.id === next.id;
        const wrapped = withStreamingOverride(base);
        expect(wrapped({ id: idA, isStreaming: false }, { id: idB, isStreaming: true })).toBe(
          false
        );
      })
    );
  });
});

describe("compareMessageProps (property-based)", () => {
  test("reflexive when not streaming", () => {
    fc.assert(
      fc.property(
        fc.record({
          id: fc.string(),
          isStreaming: fc.constant(false),
          parts: fc.array(fc.anything()),
        }),
        (props) => {
          expect(compareMessageProps(props, props)).toBe(true);
        }
      )
    );
  });
});
