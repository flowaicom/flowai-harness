import { describe, expect, test } from "bun:test";
import * as fc from "fast-check";
import {
  assertNever,
  cn,
  compactRelativeTime,
  debounce,
  deepEqual,
  formatDate,
  formatDateTime,
  formatDuration,
  formatNumber,
  formatRelativeTime,
  formatTime,
  getTimePeriod,
  groupByTimePeriod,
  TIME_PERIOD_LABELS,
  TIME_PERIOD_ORDER,
  throttle,
  truncate,
} from "./utils";

// ============================================================================
// Date/Time Formatting
// ============================================================================

describe("formatDate", () => {
  test("returns empty string for undefined", () => {
    expect(formatDate(undefined)).toBe("");
  });

  test("formats a Date object", () => {
    const d = new Date(2025, 0, 15); // Jan 15, 2025
    const result = formatDate(d);
    // Should contain "Jan" and "15" at minimum
    expect(result).toContain("Jan");
    expect(result).toContain("15");
  });

  test("formats a date string", () => {
    const result = formatDate("2025-06-20T12:00:00Z");
    expect(result).toContain("Jun");
    expect(result).toContain("20");
  });

  test("includes year when date is in a different year", () => {
    const d = new Date(2020, 5, 10); // Jun 10, 2020
    const result = formatDate(d);
    expect(result).toContain("2020");
  });

  test("omits year when date is in the current year", () => {
    const now = new Date();
    const d = new Date(now.getFullYear(), 0, 15); // Jan 15, current year
    const result = formatDate(d);
    expect(result).not.toContain(String(now.getFullYear()));
  });
});

describe("formatTime", () => {
  test("returns empty string for undefined", () => {
    expect(formatTime(undefined)).toBe("");
  });

  test("formats hour and minute", () => {
    const d = new Date(2025, 0, 15, 14, 30, 0);
    const result = formatTime(d);
    // Should contain the hour and minute in some locale-appropriate form
    // e.g. "02:30 PM" or "14:30"
    expect(result).toMatch(/\d{1,2}:\d{2}/);
  });

  test("accepts a date string", () => {
    const result = formatTime("2025-01-15T08:05:00Z");
    expect(result).toMatch(/\d{1,2}:\d{2}/);
  });
});

describe("formatDateTime", () => {
  test("returns empty string for undefined", () => {
    expect(formatDateTime(undefined)).toBe("");
  });

  test("combines date and time with a space", () => {
    const d = new Date(2025, 0, 15, 14, 30);
    const result = formatDateTime(d);
    // Should be the concatenation of formatDate and formatTime
    expect(result).toBe(`${formatDate(d)} ${formatTime(d)}`);
  });
});

describe("formatRelativeTime", () => {
  test("returns empty string for undefined", () => {
    expect(formatRelativeTime(undefined)).toBe("");
  });

  test("returns 'just now' for very recent times", () => {
    const now = new Date();
    expect(formatRelativeTime(now)).toBe("just now");
  });

  test("returns 'just now' for times less than 60 seconds ago", () => {
    const d = new Date(Date.now() - 30_000); // 30 seconds ago
    expect(formatRelativeTime(d)).toBe("just now");
  });

  test("returns minutes for times 1-59 minutes ago", () => {
    const d = new Date(Date.now() - 5 * 60_000); // 5 minutes ago
    expect(formatRelativeTime(d)).toBe("5m ago");
  });

  test("returns hours for times 1-23 hours ago", () => {
    const d = new Date(Date.now() - 3 * 3_600_000); // 3 hours ago
    expect(formatRelativeTime(d)).toBe("3h ago");
  });

  test("returns days for times 1-7 days ago", () => {
    const d = new Date(Date.now() - 2 * 86_400_000); // 2 days ago
    expect(formatRelativeTime(d)).toBe("2d ago");
  });

  test("returns 7d ago for exactly 7 days ago", () => {
    const d = new Date(Date.now() - 7 * 86_400_000); // 7 days ago
    expect(formatRelativeTime(d)).toBe("7d ago");
  });

  test("falls back to formatDate for times older than 7 days", () => {
    const d = new Date(Date.now() - 8 * 86_400_000); // 8 days ago
    expect(formatRelativeTime(d)).toBe(formatDate(d));
  });

  test("boundary: exactly 1 minute ago", () => {
    const d = new Date(Date.now() - 60_000);
    expect(formatRelativeTime(d)).toBe("1m ago");
  });

  test("boundary: exactly 1 hour ago", () => {
    const d = new Date(Date.now() - 3_600_000);
    expect(formatRelativeTime(d)).toBe("1h ago");
  });

  test("boundary: exactly 1 day ago", () => {
    const d = new Date(Date.now() - 86_400_000);
    expect(formatRelativeTime(d)).toBe("1d ago");
  });
});

// ============================================================================
// formatDuration
// ============================================================================

describe("formatDuration", () => {
  test("returns dash for NaN", () => {
    expect(formatDuration(NaN)).toBe("—");
  });

  test("returns dash for null-ish (coerced)", () => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    expect(formatDuration(null as any)).toBe("—");
  });

  test("formats 0ms", () => {
    expect(formatDuration(0)).toBe("0ms");
  });

  test("formats sub-second durations as milliseconds", () => {
    expect(formatDuration(500)).toBe("500ms");
    expect(formatDuration(999)).toBe("999ms");
  });

  test("formats seconds with one decimal", () => {
    expect(formatDuration(1000)).toBe("1.0s");
    expect(formatDuration(1500)).toBe("1.5s");
    expect(formatDuration(59999)).toBe("60.0s");
  });

  test("formats minutes and seconds", () => {
    expect(formatDuration(60_000)).toBe("1m 0s");
    expect(formatDuration(90_000)).toBe("1m 30s");
    expect(formatDuration(125_000)).toBe("2m 5s");
  });

  test("formats large durations", () => {
    expect(formatDuration(3_600_000)).toBe("60m 0s");
  });
});

// ============================================================================
// formatNumber
// ============================================================================

describe("formatNumber", () => {
  test("formats integers with locale separators", () => {
    // The exact format depends on locale, but the result should be a string
    const result = formatNumber(1234567);
    // Should contain the digits
    expect(result.replace(/\D/g, "")).toBe("1234567");
  });

  test("formats zero", () => {
    expect(formatNumber(0)).toBe("0");
  });
});

// ============================================================================
// truncate
// ============================================================================

describe("truncate", () => {
  test("returns text unchanged when within limit", () => {
    expect(truncate("hello", 10)).toBe("hello");
  });

  test("returns text unchanged when exactly at limit", () => {
    expect(truncate("hello", 5)).toBe("hello");
  });

  test("truncates and adds ellipsis when over limit", () => {
    expect(truncate("hello world", 8)).toBe("hello...");
  });

  test("truncation with very short limit", () => {
    expect(truncate("hello", 4)).toBe("h...");
  });

  test("truncation with limit of 3 leaves just ellipsis", () => {
    expect(truncate("hello", 3)).toBe("...");
  });
});

// ============================================================================
// Time Period Classification
// ============================================================================

describe("getTimePeriod", () => {
  test("returns 'older' for undefined", () => {
    expect(getTimePeriod(undefined)).toBe("older");
  });

  test("classifies now as 'today'", () => {
    expect(getTimePeriod(new Date())).toBe("today");
  });

  test("classifies midnight today as 'today'", () => {
    const now = new Date();
    const midnight = new Date(now.getFullYear(), now.getMonth(), now.getDate());
    expect(getTimePeriod(midnight)).toBe("today");
  });

  test("classifies yesterday as 'yesterday'", () => {
    const now = new Date();
    const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate());
    yesterday.setDate(yesterday.getDate() - 1);
    // Set to middle of yesterday
    yesterday.setHours(12, 0, 0, 0);
    expect(getTimePeriod(yesterday)).toBe("yesterday");
  });

  test("classifies 3 days ago as 'thisWeek'", () => {
    const d = new Date();
    d.setDate(d.getDate() - 3);
    d.setHours(12, 0, 0, 0);
    expect(getTimePeriod(d)).toBe("thisWeek");
  });

  test("classifies 10 days ago as 'thisMonth'", () => {
    const d = new Date();
    d.setDate(d.getDate() - 10);
    d.setHours(12, 0, 0, 0);
    expect(getTimePeriod(d)).toBe("thisMonth");
  });

  test("classifies 60 days ago as 'older'", () => {
    const d = new Date();
    d.setDate(d.getDate() - 60);
    expect(getTimePeriod(d)).toBe("older");
  });

  test("classifies date string input", () => {
    expect(getTimePeriod(new Date().toISOString())).toBe("today");
  });

  test("boundary: exactly at yesterday start is 'yesterday'", () => {
    const now = new Date();
    const yesterdayStart = new Date(now.getFullYear(), now.getMonth(), now.getDate());
    yesterdayStart.setDate(yesterdayStart.getDate() - 1);
    expect(getTimePeriod(yesterdayStart)).toBe("yesterday");
  });

  test("boundary: 1ms before today start is 'yesterday'", () => {
    const now = new Date();
    const todayStart = new Date(now.getFullYear(), now.getMonth(), now.getDate());
    const justBeforeToday = new Date(todayStart.getTime() - 1);
    expect(getTimePeriod(justBeforeToday)).toBe("yesterday");
  });
});

describe("groupByTimePeriod", () => {
  test("groups items correctly", () => {
    const now = new Date();
    const items = [
      { id: 1, date: now.toISOString() },
      { id: 2, date: new Date(Date.now() - 2 * 86_400_000).toISOString() },
    ];
    const groups = groupByTimePeriod(items, (item) => item.date);
    // Should have at least one group
    expect(groups.length).toBeGreaterThan(0);
    // Each group should have period, label, and items
    for (const g of groups) {
      expect(g).toHaveProperty("period");
      expect(g).toHaveProperty("label");
      expect(g).toHaveProperty("items");
      expect(g.items.length).toBeGreaterThan(0);
    }
  });

  test("maintains order within groups", () => {
    const now = new Date();
    const items = [
      { id: 1, date: new Date(now.getTime() - 1000).toISOString() },
      { id: 2, date: new Date(now.getTime() - 2000).toISOString() },
      { id: 3, date: new Date(now.getTime() - 3000).toISOString() },
    ];
    const groups = groupByTimePeriod(items, (item) => item.date);
    // All should be "today"
    expect(groups.length).toBe(1);
    expect(groups[0].period).toBe("today");
    expect(groups[0].items.map((i) => i.id)).toEqual([1, 2, 3]);
  });

  test("filters empty groups", () => {
    const now = new Date();
    const items = [{ id: 1, date: now.toISOString() }];
    const groups = groupByTimePeriod(items, (item) => item.date);
    // Should only have "today", not empty groups for other periods
    expect(groups.length).toBe(1);
    expect(groups[0].period).toBe("today");
  });

  test("handles empty input", () => {
    const groups = groupByTimePeriod([], () => "");
    expect(groups).toEqual([]);
  });

  test("groups follow TIME_PERIOD_ORDER", () => {
    const now = new Date();
    const items = [
      { id: 1, date: new Date(Date.now() - 60 * 86_400_000).toISOString() }, // older
      { id: 2, date: now.toISOString() }, // today
    ];
    const groups = groupByTimePeriod(items, (item) => item.date);
    // "today" should come before "older"
    const periods = groups.map((g) => g.period);
    expect(periods.indexOf("today")).toBeLessThan(periods.indexOf("older"));
  });

  test("labels match TIME_PERIOD_LABELS", () => {
    const now = new Date();
    const items = [{ id: 1, date: now.toISOString() }];
    const groups = groupByTimePeriod(items, (item) => item.date);
    expect(groups[0].label).toBe(TIME_PERIOD_LABELS[groups[0].period]);
  });
});

// ============================================================================
// compactRelativeTime
// ============================================================================

describe("compactRelativeTime", () => {
  test("returns empty string for undefined", () => {
    expect(compactRelativeTime(undefined)).toBe("");
  });

  test("returns 'now' for very recent times", () => {
    expect(compactRelativeTime(new Date())).toBe("now");
  });

  test("returns minutes for times 1-59m ago", () => {
    const d = new Date(Date.now() - 5 * 60_000);
    expect(compactRelativeTime(d)).toBe("5m");
  });

  test("returns hours for times 1-23h ago", () => {
    const d = new Date(Date.now() - 3 * 3_600_000);
    expect(compactRelativeTime(d)).toBe("3h");
  });

  test("returns days for 1-6 days ago", () => {
    const d = new Date(Date.now() - 2 * 86_400_000);
    expect(compactRelativeTime(d)).toBe("2d");
  });

  test("falls back to short date for 7+ days ago", () => {
    const d = new Date(Date.now() - 10 * 86_400_000);
    const result = compactRelativeTime(d);
    // Should be something like "Jan 15", not a relative time
    expect(result).not.toMatch(/^\d+[mhd]$/);
    expect(result).not.toBe("now");
  });
});

// ============================================================================
// Deep Equality
// ============================================================================

describe("deepEqual", () => {
  test("reference equality shortcut (same object)", () => {
    const obj = { a: 1, b: [2, 3] };
    expect(deepEqual(obj, obj)).toBe(true);
  });

  test("equal primitives", () => {
    expect(deepEqual(1, 1)).toBe(true);
    expect(deepEqual("hello", "hello")).toBe(true);
    expect(deepEqual(true, true)).toBe(true);
    expect(deepEqual(null, null)).toBe(true);
  });

  test("unequal primitives", () => {
    expect(deepEqual(1, 2)).toBe(false);
    expect(deepEqual("a", "b")).toBe(false);
    expect(deepEqual(true, false)).toBe(false);
  });

  test("objects with same keys in different order are equal", () => {
    expect(deepEqual({ a: 1, b: 2 }, { b: 2, a: 1 })).toBe(true);
  });

  test("objects with different values are not equal", () => {
    expect(deepEqual({ a: 1 }, { a: 2 })).toBe(false);
  });

  test("objects with different key counts are not equal", () => {
    expect(deepEqual({ a: 1 }, { a: 1, b: 2 })).toBe(false);
  });

  test("arrays with same elements in same order are equal", () => {
    expect(deepEqual([1, 2, 3], [1, 2, 3])).toBe(true);
  });

  test("arrays with same elements in different order are NOT equal", () => {
    expect(deepEqual([1, 2, 3], [3, 2, 1])).toBe(false);
  });

  test("arrays of different lengths are not equal", () => {
    expect(deepEqual([1, 2], [1, 2, 3])).toBe(false);
  });

  test("nested objects", () => {
    const a = { x: { y: { z: 1 } }, w: [1, { a: 2 }] };
    const b = { w: [1, { a: 2 }], x: { y: { z: 1 } } };
    expect(deepEqual(a, b)).toBe(true);
  });

  test("nested objects with differences", () => {
    const a = { x: { y: { z: 1 } } };
    const b = { x: { y: { z: 2 } } };
    expect(deepEqual(a, b)).toBe(false);
  });

  test("null vs undefined", () => {
    expect(deepEqual(null, undefined)).toBe(false);
  });

  test("null vs object", () => {
    expect(deepEqual(null, {})).toBe(false);
    expect(deepEqual({}, null)).toBe(false);
  });

  test("0 vs false (different types)", () => {
    expect(deepEqual(0, false)).toBe(false);
  });

  test("empty string vs null", () => {
    expect(deepEqual("", null)).toBe(false);
  });

  test("empty string vs 0 (different types)", () => {
    expect(deepEqual("", 0)).toBe(false);
  });

  test("array vs object", () => {
    expect(deepEqual([1, 2], { 0: 1, 1: 2 })).toBe(false);
  });

  test("empty objects are equal", () => {
    expect(deepEqual({}, {})).toBe(true);
  });

  test("empty arrays are equal", () => {
    expect(deepEqual([], [])).toBe(true);
  });

  test("prototype differences: plain object vs class instance", () => {
    class Foo {
      x: number;
      constructor(x: number) {
        this.x = x;
      }
    }
    // deepEqual checks structural equality (own keys), so a class instance
    // with the same own properties will be structurally equal to a plain object
    expect(deepEqual(new Foo(1), { x: 1 })).toBe(true);
  });

  test("deeply nested arrays in objects", () => {
    expect(deepEqual({ a: [[1, 2], [3]] }, { a: [[1, 2], [3]] })).toBe(true);
    expect(deepEqual({ a: [[1, 2], [3]] }, { a: [[1, 2], [4]] })).toBe(false);
  });
});

// ============================================================================
// Debounce
// ============================================================================

describe("debounce", () => {
  test("returns a callable function", () => {
    const fn = debounce(() => {}, 100);
    expect(typeof fn).toBe("function");
  });

  test("fires after delay", async () => {
    let called = 0;
    const fn = debounce(() => {
      called++;
    }, 50);
    fn();
    expect(called).toBe(0);
    await new Promise<void>((r) => setTimeout(r, 80));
    expect(called).toBe(1);
  });

  test("resets timer on repeated calls (only fires once)", async () => {
    let called = 0;
    const fn = debounce(() => {
      called++;
    }, 50);
    fn();
    await new Promise<void>((r) => setTimeout(r, 20));
    fn(); // reset the timer
    await new Promise<void>((r) => setTimeout(r, 20));
    fn(); // reset again
    await new Promise<void>((r) => setTimeout(r, 80));
    expect(called).toBe(1);
  });

  test("passes arguments to the underlying function", async () => {
    let receivedArgs: unknown[] = [];
    const fn = debounce((...args: unknown[]) => {
      receivedArgs = args;
    }, 50);
    fn("a", "b");
    await new Promise<void>((r) => setTimeout(r, 80));
    expect(receivedArgs).toEqual(["a", "b"]);
  });

  test("passes last call arguments when debounced", async () => {
    let receivedArg: unknown;
    const fn = debounce((x: unknown) => {
      receivedArg = x;
    }, 50);
    fn("first");
    fn("second");
    fn("third");
    await new Promise<void>((r) => setTimeout(r, 80));
    expect(receivedArg).toBe("third");
  });
});

// ============================================================================
// Throttle
// ============================================================================

describe("throttle", () => {
  test("returns a callable function", () => {
    const fn = throttle(() => {}, 100);
    expect(typeof fn).toBe("function");
  });

  test("fires immediately on first call", () => {
    let called = 0;
    const fn = throttle(() => {
      called++;
    }, 100);
    fn();
    expect(called).toBe(1);
  });

  test("blocks during cooldown period", () => {
    let called = 0;
    const fn = throttle(() => {
      called++;
    }, 100);
    fn(); // fires immediately
    fn(); // blocked
    fn(); // blocked
    expect(called).toBe(1);
  });

  test("fires again after cooldown expires", async () => {
    let called = 0;
    const fn = throttle(() => {
      called++;
    }, 50);
    fn(); // fires immediately
    expect(called).toBe(1);
    await new Promise<void>((r) => setTimeout(r, 80));
    fn(); // cooldown expired, fires again
    expect(called).toBe(2);
  });

  test("passes arguments to the underlying function", () => {
    let receivedArgs: unknown[] = [];
    const fn = throttle((...args: unknown[]) => {
      receivedArgs = args;
    }, 100);
    fn("a", "b");
    expect(receivedArgs).toEqual(["a", "b"]);
  });
});

// ============================================================================
// cn (class name merger)
// ============================================================================

describe("cn", () => {
  test("merges class names", () => {
    expect(cn("foo", "bar")).toBe("foo bar");
  });

  test("handles conditional classes", () => {
    const result = cn("base", false && "hidden", "visible");
    expect(result).toBe("base visible");
  });

  test("merges conflicting Tailwind classes (last wins)", () => {
    // twMerge deduplicates conflicting Tailwind utility classes
    const result = cn("p-2", "p-4");
    expect(result).toBe("p-4");
  });

  test("handles empty input", () => {
    expect(cn()).toBe("");
  });

  test("handles undefined and null values", () => {
    expect(cn("a", undefined, null, "b")).toBe("a b");
  });
});

// ============================================================================
// assertNever
// ============================================================================

describe("assertNever", () => {
  test("throws with the unexpected value", () => {
    expect(() => assertNever("unexpected" as never)).toThrow("Unexpected discriminant");
  });

  test("includes the value in the error message", () => {
    expect(() => assertNever(42 as never)).toThrow("42");
  });
});

// ============================================================================
// Constants
// ============================================================================

describe("TIME_PERIOD_ORDER", () => {
  test("has the expected order", () => {
    expect(TIME_PERIOD_ORDER).toEqual(["today", "yesterday", "thisWeek", "thisMonth", "older"]);
  });
});

describe("TIME_PERIOD_LABELS", () => {
  test("has labels for every period", () => {
    for (const p of TIME_PERIOD_ORDER) {
      expect(typeof TIME_PERIOD_LABELS[p]).toBe("string");
      expect(TIME_PERIOD_LABELS[p].length).toBeGreaterThan(0);
    }
  });
});

// ============================================================================
// Property-Based Tests (Interpreter Layer)
// ============================================================================

describe("deepEqual equivalence relation (property-based)", () => {
  /** JSON-serializable values — the domain deepEqual is designed for. */
  const arbJsonValue = fc.jsonValue();

  test("reflexivity: deepEqual(x, x) = true", () => {
    fc.assert(
      fc.property(arbJsonValue, (x) => {
        expect(deepEqual(x, x)).toBe(true);
      })
    );
  });

  test("symmetry: deepEqual(a, b) = deepEqual(b, a)", () => {
    fc.assert(
      fc.property(arbJsonValue, arbJsonValue, (a, b) => {
        expect(deepEqual(a, b)).toBe(deepEqual(b, a));
      })
    );
  });

  test("structural: deepEqual(x, JSON.parse(JSON.stringify(x)))", () => {
    fc.assert(
      fc.property(arbJsonValue, (x) => {
        const clone = JSON.parse(JSON.stringify(x));
        expect(deepEqual(x, clone)).toBe(true);
      })
    );
  });

  test("type discrimination: never confuses types", () => {
    fc.assert(
      fc.property(fc.integer(), fc.string(), (n, s) => {
        expect(deepEqual(n, s)).toBe(false);
      })
    );
  });
});

describe("truncate (property-based)", () => {
  test("output length never exceeds maxLength", () => {
    fc.assert(
      fc.property(fc.string(), fc.integer({ min: 4, max: 1000 }), (text, maxLength) => {
        expect(truncate(text, maxLength).length).toBeLessThanOrEqual(maxLength);
      })
    );
  });

  test("short text returned unchanged", () => {
    fc.assert(
      fc.property(
        fc.string({ maxLength: 10 }),
        fc.integer({ min: 10, max: 100 }),
        (text, maxLength) => {
          if (text.length <= maxLength) {
            expect(truncate(text, maxLength)).toBe(text);
          }
        }
      )
    );
  });

  test("truncated text ends with '...'", () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 20 }),
        fc.integer({ min: 4, max: 19 }),
        (text, maxLength) => {
          expect(truncate(text, maxLength)).toMatch(/\.\.\.$/);
        }
      )
    );
  });
});

describe("formatDuration (property-based)", () => {
  test("never throws for any non-negative number", () => {
    fc.assert(
      fc.property(fc.nat(), (ms) => {
        const result = formatDuration(ms);
        expect(typeof result).toBe("string");
        expect(result.length).toBeGreaterThan(0);
      })
    );
  });

  test("returns '—' for NaN", () => {
    expect(formatDuration(NaN)).toBe("—");
  });
});

describe("formatNumber (property-based)", () => {
  test("never throws for any integer", () => {
    fc.assert(
      fc.property(fc.integer(), (n) => {
        const result = formatNumber(n);
        expect(typeof result).toBe("string");
        expect(result.length).toBeGreaterThan(0);
      })
    );
  });
});
