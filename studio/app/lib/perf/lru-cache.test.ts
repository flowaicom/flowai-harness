import { describe, expect, test } from "bun:test";
import * as fc from "fast-check";

import { LRUCache } from "./lru-cache";

describe("LRUCache", () => {
  // =========================================================================
  // Construction
  // =========================================================================

  describe("construction", () => {
    test("rejects non-positive maxSize", () => {
      expect(() => new LRUCache(0)).toThrow("maxSize must be positive");
      expect(() => new LRUCache(-1)).toThrow("maxSize must be positive");
    });

    test("creates cache with valid maxSize", () => {
      const cache = new LRUCache<string, number>(10);
      expect(cache.size).toBe(0);
      expect(cache.getStats().maxSize).toBe(10);
    });
  });

  // =========================================================================
  // Empty cache
  // =========================================================================

  describe("empty cache", () => {
    test("get returns undefined", () => {
      const cache = new LRUCache<string, number>(5);
      expect(cache.get("missing")).toBeUndefined();
    });

    test("has returns false", () => {
      const cache = new LRUCache<string, number>(5);
      expect(cache.has("missing")).toBe(false);
    });

    test("delete returns false", () => {
      const cache = new LRUCache<string, number>(5);
      expect(cache.delete("missing")).toBe(false);
    });

    test("size is 0", () => {
      const cache = new LRUCache<string, number>(5);
      expect(cache.size).toBe(0);
    });

    test("stats show zero counters and null hitRate", () => {
      const cache = new LRUCache<string, number>(5);
      const stats = cache.getStats();
      expect(stats.hits).toBe(0);
      expect(stats.misses).toBe(0);
      expect(stats.evictions).toBe(0);
      expect(stats.hitRate).toBeNull();
    });
  });

  // =========================================================================
  // Capacity invariant: cache never exceeds maxSize
  // =========================================================================

  describe("capacity invariant", () => {
    test("cache never exceeds maxSize after inserts", () => {
      const maxSize = 3;
      const cache = new LRUCache<string, number>(maxSize);

      for (let i = 0; i < 100; i++) {
        cache.set(`key-${i}`, i);
        expect(cache.size).toBeLessThanOrEqual(maxSize);
      }
    });

    test("cache stays at maxSize when continuously inserting unique keys", () => {
      const maxSize = 5;
      const cache = new LRUCache<string, number>(maxSize);

      // Fill it up
      for (let i = 0; i < maxSize; i++) {
        cache.set(`key-${i}`, i);
      }
      expect(cache.size).toBe(maxSize);

      // Add more unique keys
      for (let i = maxSize; i < maxSize + 20; i++) {
        cache.set(`key-${i}`, i);
        expect(cache.size).toBe(maxSize);
      }
    });

    test("updating an existing key does not increase size", () => {
      const cache = new LRUCache<string, number>(3);
      cache.set("a", 1);
      cache.set("b", 2);
      cache.set("c", 3);
      expect(cache.size).toBe(3);

      cache.set("b", 99);
      expect(cache.size).toBe(3);
    });
  });

  // =========================================================================
  // Hit/miss accounting
  // =========================================================================

  describe("hit/miss accounting", () => {
    test("getStats reflects actual hits and misses via get()", () => {
      const cache = new LRUCache<string, number>(5);
      cache.set("a", 1);
      cache.set("b", 2);

      cache.get("a"); // hit
      cache.get("b"); // hit
      cache.get("c"); // miss
      cache.get("d"); // miss
      cache.get("a"); // hit

      const stats = cache.getStats();
      expect(stats.hits).toBe(3);
      expect(stats.misses).toBe(2);
      expect(stats.hitRate).toBeCloseTo(3 / 5);
    });

    test("getStats reflects hits and misses via getOrCompute()", () => {
      const cache = new LRUCache<string, number>(5);

      cache.getOrCompute("a", () => 1); // miss + compute
      cache.getOrCompute("a", () => 999); // hit (should return 1, not 999)
      cache.getOrCompute("b", () => 2); // miss + compute

      const stats = cache.getStats();
      expect(stats.hits).toBe(1);
      expect(stats.misses).toBe(2);
    });

    test("set does not affect hit/miss counters", () => {
      const cache = new LRUCache<string, number>(5);
      cache.set("a", 1);
      cache.set("b", 2);
      cache.set("c", 3);

      const stats = cache.getStats();
      expect(stats.hits).toBe(0);
      expect(stats.misses).toBe(0);
    });

    test("has does not affect hit/miss counters", () => {
      const cache = new LRUCache<string, number>(5);
      cache.set("a", 1);
      cache.has("a");
      cache.has("missing");

      const stats = cache.getStats();
      expect(stats.hits).toBe(0);
      expect(stats.misses).toBe(0);
    });

    test("eviction counter tracks evictions", () => {
      const cache = new LRUCache<string, number>(2);
      cache.set("a", 1);
      cache.set("b", 2);
      cache.set("c", 3); // evicts "a"
      cache.set("d", 4); // evicts "b"

      expect(cache.getStats().evictions).toBe(2);
    });
  });

  // =========================================================================
  // Eviction ordering: LRU entry evicted first when full
  // =========================================================================

  describe("eviction ordering", () => {
    test("evicts oldest (least recently inserted) entry first", () => {
      const cache = new LRUCache<string, number>(3);
      cache.set("a", 1);
      cache.set("b", 2);
      cache.set("c", 3);

      // "a" is oldest -> gets evicted
      cache.set("d", 4);
      expect(cache.has("a")).toBe(false);
      expect(cache.has("b")).toBe(true);
      expect(cache.has("c")).toBe(true);
      expect(cache.has("d")).toBe(true);
    });

    test("accessing an entry promotes it, so a different entry is evicted", () => {
      const cache = new LRUCache<string, number>(3);
      cache.set("a", 1);
      cache.set("b", 2);
      cache.set("c", 3);

      // Access "a" to promote it to most recently used
      cache.get("a");

      // Now "b" is the oldest -> gets evicted
      cache.set("d", 4);
      expect(cache.has("a")).toBe(true);
      expect(cache.has("b")).toBe(false);
      expect(cache.has("c")).toBe(true);
      expect(cache.has("d")).toBe(true);
    });

    test("updating an entry promotes it", () => {
      const cache = new LRUCache<string, number>(3);
      cache.set("a", 1);
      cache.set("b", 2);
      cache.set("c", 3);

      // Update "a" value -> promotes it
      cache.set("a", 100);

      // Now "b" is the oldest -> gets evicted
      cache.set("d", 4);
      expect(cache.has("a")).toBe(true);
      expect(cache.get("a")).toBe(100);
      expect(cache.has("b")).toBe(false);
    });

    test("getOrCompute promotes existing entries", () => {
      const cache = new LRUCache<string, number>(3);
      cache.set("a", 1);
      cache.set("b", 2);
      cache.set("c", 3);

      // getOrCompute "a" promotes it
      cache.getOrCompute("a", () => 999);

      // "b" should now be evicted on next insert
      cache.set("d", 4);
      expect(cache.has("a")).toBe(true);
      expect(cache.has("b")).toBe(false);
    });

    test("keys() returns entries from oldest to newest", () => {
      const cache = new LRUCache<string, number>(5);
      cache.set("a", 1);
      cache.set("b", 2);
      cache.set("c", 3);

      // Access "a" to promote it
      cache.get("a");

      const keys = [...cache.keys()];
      expect(keys).toEqual(["b", "c", "a"]);
    });
  });

  // =========================================================================
  // getOrCompute
  // =========================================================================

  describe("getOrCompute", () => {
    test("computes on miss and caches the result", () => {
      const cache = new LRUCache<string, number>(5);
      let computeCount = 0;

      const value = cache.getOrCompute("x", () => {
        computeCount++;
        return 42;
      });

      expect(value).toBe(42);
      expect(computeCount).toBe(1);
      expect(cache.has("x")).toBe(true);
    });

    test("returns cached value on hit without recomputing", () => {
      const cache = new LRUCache<string, number>(5);
      let computeCount = 0;

      cache.getOrCompute("x", () => {
        computeCount++;
        return 42;
      });

      const value = cache.getOrCompute("x", () => {
        computeCount++;
        return 999;
      });

      expect(value).toBe(42);
      expect(computeCount).toBe(1);
    });

    test("computed values respect eviction", () => {
      const cache = new LRUCache<string, number>(2);

      cache.getOrCompute("a", () => 1);
      cache.getOrCompute("b", () => 2);
      cache.getOrCompute("c", () => 3); // evicts "a"

      expect(cache.has("a")).toBe(false);
      expect(cache.getStats().evictions).toBe(1);
    });
  });

  // =========================================================================
  // resetStats
  // =========================================================================

  describe("resetStats", () => {
    test("clears counters without clearing cache entries", () => {
      const cache = new LRUCache<string, number>(5);
      cache.set("a", 1);
      cache.set("b", 2);
      cache.get("a"); // hit
      cache.get("missing"); // miss

      expect(cache.getStats().hits).toBe(1);
      expect(cache.getStats().misses).toBe(1);

      cache.resetStats();

      const stats = cache.getStats();
      expect(stats.hits).toBe(0);
      expect(stats.misses).toBe(0);
      expect(stats.evictions).toBe(0);
      expect(stats.hitRate).toBeNull();

      // Cache entries remain
      expect(cache.size).toBe(2);
      expect(cache.has("a")).toBe(true);
      expect(cache.has("b")).toBe(true);
    });
  });

  // =========================================================================
  // clear
  // =========================================================================

  describe("clear", () => {
    test("removes all entries and resets stats", () => {
      const cache = new LRUCache<string, number>(5);
      cache.set("a", 1);
      cache.set("b", 2);
      cache.get("a");
      cache.get("missing");

      cache.clear();

      expect(cache.size).toBe(0);
      const stats = cache.getStats();
      expect(stats.hits).toBe(0);
      expect(stats.misses).toBe(0);
      expect(stats.evictions).toBe(0);
      expect(stats.size).toBe(0);
    });
  });

  // =========================================================================
  // delete
  // =========================================================================

  describe("delete", () => {
    test("removes the entry and returns true", () => {
      const cache = new LRUCache<string, number>(5);
      cache.set("a", 1);
      expect(cache.delete("a")).toBe(true);
      expect(cache.has("a")).toBe(false);
      expect(cache.size).toBe(0);
    });

    test("returns false for missing key", () => {
      const cache = new LRUCache<string, number>(5);
      expect(cache.delete("missing")).toBe(false);
    });
  });

  // =========================================================================
  // hitRate
  // =========================================================================

  describe("hitRate", () => {
    test("is null when no accesses occurred", () => {
      const cache = new LRUCache<string, number>(5);
      expect(cache.getStats().hitRate).toBeNull();
    });

    test("is 0 when all misses", () => {
      const cache = new LRUCache<string, number>(5);
      cache.get("a");
      cache.get("b");
      expect(cache.getStats().hitRate).toBe(0);
    });

    test("is 1 when all hits", () => {
      const cache = new LRUCache<string, number>(5);
      cache.set("a", 1);
      cache.get("a");
      cache.get("a");
      expect(cache.getStats().hitRate).toBe(1);
    });
  });

  // =========================================================================
  // Edge case: maxSize of 1
  // =========================================================================

  describe("maxSize of 1", () => {
    test("only holds one entry at a time", () => {
      const cache = new LRUCache<string, number>(1);
      cache.set("a", 1);
      expect(cache.size).toBe(1);

      cache.set("b", 2);
      expect(cache.size).toBe(1);
      expect(cache.has("a")).toBe(false);
      expect(cache.has("b")).toBe(true);
    });
  });
});

// ============================================================================
// Generator DSL (Description Layer)
//
// Operations over a small key space to maximize eviction + hit contention.
// The operation sequence is a program (description); the replay loop below
// is the interpreter.
// ============================================================================

type CacheOp =
  | { type: "set"; key: string; value: number }
  | { type: "get"; key: string }
  | { type: "has"; key: string }
  | { type: "delete"; key: string }
  | { type: "getOrCompute"; key: string; value: number };

/** Small key space forces eviction and cache hits. */
const arbKey = fc.constantFrom("a", "b", "c", "d", "e", "f", "g", "h");

const arbCacheOp: fc.Arbitrary<CacheOp> = fc.oneof(
  fc.record({
    type: fc.constant("set" as const),
    key: arbKey,
    value: fc.integer(),
  }),
  fc.record({ type: fc.constant("get" as const), key: arbKey }),
  fc.record({ type: fc.constant("has" as const), key: arbKey }),
  fc.record({ type: fc.constant("delete" as const), key: arbKey }),
  fc.record({
    type: fc.constant("getOrCompute" as const),
    key: arbKey,
    value: fc.integer(),
  })
);

// ============================================================================
// LRU Reference Model (Oracle)
//
// A trivially-correct LRU using Map's insertion-order guarantee.
// This is the "known-good reference" for the model test pattern.
// ============================================================================

class LRUModel {
  private map = new Map<string, number>();
  constructor(private maxSize: number) {}

  set(key: string, value: number): void {
    if (this.map.has(key)) {
      this.map.delete(key);
    } else if (this.map.size >= this.maxSize) {
      const oldest = this.map.keys().next().value!;
      this.map.delete(oldest);
    }
    this.map.set(key, value);
  }

  get(key: string): number | undefined {
    const value = this.map.get(key);
    if (value !== undefined) {
      this.map.delete(key);
      this.map.set(key, value);
      return value;
    }
    return undefined;
  }

  getOrCompute(key: string, value: number): number {
    const existing = this.map.get(key);
    if (existing !== undefined) {
      this.map.delete(key);
      this.map.set(key, existing);
      return existing;
    }
    if (this.map.size >= this.maxSize) {
      const oldest = this.map.keys().next().value!;
      this.map.delete(oldest);
    }
    this.map.set(key, value);
    return value;
  }

  has(key: string): boolean {
    return this.map.has(key);
  }

  delete(key: string): boolean {
    return this.map.delete(key);
  }

  get size(): number {
    return this.map.size;
  }
}

// ============================================================================
// Property-Based Tests (Interpreter Layer)
// ============================================================================

describe("LRU model test (property-based)", () => {
  test("agrees with Map oracle on every operation", () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 1, max: 20 }),
        fc.array(arbCacheOp, { minLength: 1, maxLength: 200 }),
        (maxSize, ops) => {
          const cache = new LRUCache<string, number>(maxSize);
          const model = new LRUModel(maxSize);

          for (const op of ops) {
            switch (op.type) {
              case "set":
                cache.set(op.key, op.value);
                model.set(op.key, op.value);
                break;
              case "get":
                expect(cache.get(op.key)).toBe(model.get(op.key));
                break;
              case "has":
                expect(cache.has(op.key)).toBe(model.has(op.key));
                break;
              case "delete":
                expect(cache.delete(op.key)).toBe(model.delete(op.key));
                break;
              case "getOrCompute": {
                const rv = cache.getOrCompute(op.key, () => op.value);
                const mv = model.getOrCompute(op.key, op.value);
                expect(rv).toBe(mv);
                break;
              }
            }
            expect(cache.size).toBe(model.size);
          }
        }
      )
    );
  });
});

describe("LRU invariants (property-based)", () => {
  test("capacity: size ≤ maxSize after any operation sequence", () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 1, max: 50 }),
        fc.array(arbCacheOp, { maxLength: 300 }),
        (maxSize, ops) => {
          const cache = new LRUCache<string, number>(maxSize);
          for (const op of ops) {
            switch (op.type) {
              case "set":
                cache.set(op.key, op.value);
                break;
              case "get":
                cache.get(op.key);
                break;
              case "has":
                cache.has(op.key);
                break;
              case "delete":
                cache.delete(op.key);
                break;
              case "getOrCompute":
                cache.getOrCompute(op.key, () => op.value);
                break;
            }
            expect(cache.size).toBeLessThanOrEqual(maxSize);
          }
        }
      )
    );
  });

  test("hit rate ∈ [0,1] or null, and hits + misses = total accesses", () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 1, max: 20 }),
        fc.array(arbCacheOp, { maxLength: 100 }),
        (maxSize, ops) => {
          const cache = new LRUCache<string, number>(maxSize);
          let accessCount = 0;

          for (const op of ops) {
            switch (op.type) {
              case "set":
                cache.set(op.key, op.value);
                break;
              case "get":
                cache.get(op.key);
                accessCount++;
                break;
              case "has":
                cache.has(op.key);
                break;
              case "delete":
                cache.delete(op.key);
                break;
              case "getOrCompute":
                cache.getOrCompute(op.key, () => op.value);
                accessCount++;
                break;
            }
          }

          const stats = cache.getStats();
          expect(stats.hits + stats.misses).toBe(accessCount);
          if (accessCount > 0) {
            expect(stats.hitRate).not.toBeNull();
            expect(stats.hitRate!).toBeGreaterThanOrEqual(0);
            expect(stats.hitRate!).toBeLessThanOrEqual(1);
          } else {
            expect(stats.hitRate).toBeNull();
          }
        }
      )
    );
  });
});
