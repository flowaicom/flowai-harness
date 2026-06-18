import { describe, expect, test } from "bun:test";
import * as fc from "fast-check";

import { arbISODate } from "~/lib/test-utils/arbitraries";
import {
  findThread,
  generateTitle,
  isEvalThread,
  removeThreadFromList,
  sortFilesByRecent,
  sortThreadsByRecent,
  type Thread,
  type ThreadFile,
  touchThread,
  updateThreadInList,
} from "./thread";

// ============================================================================
// Helpers
// ============================================================================

const mkThread = (id: string, updatedAt: string, title = "t"): Thread => ({
  id,
  title,
  resourceId: "r1",
  createdAt: "2024-01-01T00:00:00Z",
  updatedAt,
});

// ============================================================================
// sortThreadsByRecent
// ============================================================================

describe("sortThreadsByRecent", () => {
  test("sorts by updatedAt descending", () => {
    const threads = [
      mkThread("a", "2024-01-01T00:00:00Z"),
      mkThread("b", "2024-01-03T00:00:00Z"),
      mkThread("c", "2024-01-02T00:00:00Z"),
    ];
    const sorted = sortThreadsByRecent(threads);
    expect(sorted.map((t) => t.id)).toEqual(["b", "c", "a"]);
  });

  test("does not mutate original array", () => {
    const threads = [mkThread("a", "2024-01-02T00:00:00Z"), mkThread("b", "2024-01-01T00:00:00Z")];
    const original = [...threads];
    sortThreadsByRecent(threads);
    expect(threads).toEqual(original);
  });

  test("empty array returns empty", () => {
    expect(sortThreadsByRecent([])).toEqual([]);
  });

  test("single element returns same", () => {
    const threads = [mkThread("a", "2024-01-01T00:00:00Z")];
    expect(sortThreadsByRecent(threads)).toEqual(threads);
  });
});

// ============================================================================
// findThread
// ============================================================================

describe("findThread", () => {
  test("finds existing thread", () => {
    const threads = [mkThread("a", "t1"), mkThread("b", "t2")];
    expect(findThread(threads, "b")?.id).toBe("b");
  });

  test("returns undefined for missing thread", () => {
    expect(findThread([mkThread("a", "t1")], "missing")).toBeUndefined();
  });

  test("returns undefined for empty list", () => {
    expect(findThread([], "a")).toBeUndefined();
  });
});

// ============================================================================
// touchThread
// ============================================================================

describe("touchThread", () => {
  test("returns new object (immutability)", () => {
    const thread = mkThread("a", "2024-01-01T00:00:00Z");
    const touched = touchThread(thread);
    expect(touched).not.toBe(thread);
    expect(touched.id).toBe(thread.id);
    expect(touched.title).toBe(thread.title);
  });

  test("updatedAt changes", () => {
    const thread = mkThread("a", "2024-01-01T00:00:00Z");
    const touched = touchThread(thread);
    expect(touched.updatedAt).not.toBe(thread.updatedAt);
  });
});

// ============================================================================
// updateThreadInList
// ============================================================================

describe("updateThreadInList", () => {
  test("updates matching thread", () => {
    const threads = [mkThread("a", "t1"), mkThread("b", "t2")];
    const updated = updateThreadInList(threads, "b", { title: "New Title" });
    expect(updated[1].title).toBe("New Title");
    expect(updated[0].title).toBe("t"); // unchanged
  });

  test("returns same-shaped list for missing id", () => {
    const threads = [mkThread("a", "t1")];
    const updated = updateThreadInList(threads, "missing", { title: "X" });
    expect(updated).toEqual(threads);
  });

  test("does not mutate original", () => {
    const threads = [mkThread("a", "t1")];
    const original = threads[0];
    updateThreadInList(threads, "a", { title: "Changed" });
    expect(original.title).toBe("t");
  });
});

// ============================================================================
// removeThreadFromList
// ============================================================================

describe("removeThreadFromList", () => {
  test("removes matching thread", () => {
    const threads = [mkThread("a", "t1"), mkThread("b", "t2")];
    const result = removeThreadFromList(threads, "a");
    expect(result.length).toBe(1);
    expect(result[0].id).toBe("b");
  });

  test("returns same list for missing id", () => {
    const threads = [mkThread("a", "t1")];
    const result = removeThreadFromList(threads, "missing");
    expect(result.length).toBe(1);
  });

  test("does not mutate original", () => {
    const threads = [mkThread("a", "t1"), mkThread("b", "t2")];
    removeThreadFromList(threads, "a");
    expect(threads.length).toBe(2);
  });
});

// ============================================================================
// generateTitle
// ============================================================================

describe("generateTitle", () => {
  test("short content returned as-is", () => {
    expect(generateTitle("Hello world")).toBe("Hello world");
  });

  test("long content truncated with ellipsis", () => {
    const long = "a".repeat(100);
    const title = generateTitle(long, 50);
    expect(title.length).toBe(53); // 50 + "..."
    expect(title.endsWith("...")).toBe(true);
  });

  test("whitespace normalized", () => {
    expect(generateTitle("  hello   world  ")).toBe("hello world");
  });

  test("empty content returns fallback", () => {
    expect(generateTitle("")).toBe("New Conversation");
  });

  test("whitespace-only returns fallback", () => {
    expect(generateTitle("   ")).toBe("New Conversation");
  });
});

// ============================================================================
// isEvalThread
// ============================================================================

describe("isEvalThread", () => {
  test("eval thread id detected", () => {
    expect(isEvalThread("eval-run-123")).toBe(true);
  });

  test("regular thread id rejected", () => {
    expect(isEvalThread("thread-123")).toBe(false);
  });

  test("undefined returns false", () => {
    expect(isEvalThread(undefined)).toBe(false);
  });
});

// ============================================================================
// sortFilesByRecent
// ============================================================================

describe("sortFilesByRecent", () => {
  test("sorts by createdAt descending", () => {
    const files: ThreadFile[] = [
      { fileId: "1", filename: "a.txt", threadId: "t1", createdAt: "2024-01-01T00:00:00Z" },
      { fileId: "2", filename: "b.txt", threadId: "t1", createdAt: "2024-01-03T00:00:00Z" },
      { fileId: "3", filename: "c.txt", threadId: "t1", createdAt: "2024-01-02T00:00:00Z" },
    ];
    const sorted = sortFilesByRecent(files);
    expect(sorted.map((f) => f.fileId)).toEqual(["2", "3", "1"]);
  });

  test("does not mutate original", () => {
    const files: ThreadFile[] = [
      { fileId: "1", filename: "a.txt", threadId: "t1", createdAt: "2024-01-02T00:00:00Z" },
      { fileId: "2", filename: "b.txt", threadId: "t1", createdAt: "2024-01-01T00:00:00Z" },
    ];
    const original = [...files];
    sortFilesByRecent(files);
    expect(files).toEqual(original);
  });
});

// ============================================================================
// Generator DSL
// ============================================================================

const arbThread: fc.Arbitrary<Thread> = fc.record({
  id: fc.string({ minLength: 1 }),
  title: fc.string(),
  resourceId: fc.string(),
  createdAt: arbISODate,
  updatedAt: arbISODate,
});

// ============================================================================
// Property-Based Tests
// ============================================================================

describe("sortThreadsByRecent (property-based)", () => {
  test("idempotence: sort(sort(ts)) = sort(ts)", () => {
    fc.assert(
      fc.property(fc.array(arbThread), (threads) => {
        const once = sortThreadsByRecent(threads);
        const twice = sortThreadsByRecent(once);
        expect(twice).toEqual(once);
      })
    );
  });

  test("preserves length", () => {
    fc.assert(
      fc.property(fc.array(arbThread), (threads) => {
        expect(sortThreadsByRecent(threads).length).toBe(threads.length);
      })
    );
  });

  test("does not mutate original", () => {
    fc.assert(
      fc.property(fc.array(arbThread), (threads) => {
        const snapshot = threads.map((t) => t.id);
        sortThreadsByRecent(threads);
        expect(threads.map((t) => t.id)).toEqual(snapshot);
      })
    );
  });
});

describe("updateThreadInList (property-based)", () => {
  test("preserves length", () => {
    fc.assert(
      fc.property(fc.array(arbThread), fc.string(), (threads, newTitle) => {
        if (threads.length === 0) return;
        const id = threads[0].id;
        const updated = updateThreadInList(threads, id, { title: newTitle });
        expect(updated.length).toBe(threads.length);
      })
    );
  });

  test("non-targeted threads unchanged", () => {
    fc.assert(
      fc.property(fc.array(arbThread, { minLength: 2 }), fc.string(), (threads, newTitle) => {
        const targetId = threads[0].id;
        const updated = updateThreadInList(threads, targetId, {
          title: newTitle,
        });
        for (let i = 1; i < threads.length; i++) {
          if (threads[i].id !== targetId) {
            expect(updated[i]).toEqual(threads[i]);
          }
        }
      })
    );
  });
});

describe("removeThreadFromList (property-based)", () => {
  test("length decreases by at most 1", () => {
    fc.assert(
      fc.property(fc.array(arbThread), fc.string(), (threads, id) => {
        const result = removeThreadFromList(threads, id);
        expect(result.length).toBeGreaterThanOrEqual(threads.length - 1);
        expect(result.length).toBeLessThanOrEqual(threads.length);
      })
    );
  });

  test("removed id no longer present", () => {
    fc.assert(
      fc.property(fc.array(arbThread, { minLength: 1 }), (threads) => {
        const id = threads[0].id;
        const result = removeThreadFromList(threads, id);
        expect(result.find((t) => t.id === id)).toBeUndefined();
      })
    );
  });

  test("does not mutate original", () => {
    fc.assert(
      fc.property(fc.array(arbThread), fc.string(), (threads, id) => {
        const len = threads.length;
        removeThreadFromList(threads, id);
        expect(threads.length).toBe(len);
      })
    );
  });
});

describe("generateTitle (property-based)", () => {
  test("non-empty content respects maxLength + 3", () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 1 }).filter((s) => /\S/.test(s)),
        fc.integer({ min: 1, max: 200 }),
        (content, maxLength) => {
          const title = generateTitle(content, maxLength);
          expect(title.length).toBeLessThanOrEqual(maxLength + 3);
        }
      )
    );
  });

  test("whitespace-only input returns fallback", () => {
    fc.assert(
      fc.property(
        fc.array(fc.constantFrom(" ", "\t", "\n", "\r")).map((a) => a.join("")),
        (whitespace) => {
          expect(generateTitle(whitespace)).toBe("New Conversation");
        }
      )
    );
  });
});
