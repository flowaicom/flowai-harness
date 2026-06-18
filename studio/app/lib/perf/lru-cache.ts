/**
 * LRU (Least Recently Used) Cache with bounded size and statistics.
 *
 * Prevents unbounded memory growth by evicting least recently used entries.
 * O(1) get/set operations using Map's insertion order guarantee.
 *
 * Features:
 * - Hit/miss tracking for cache efficiency analysis
 * - Eviction counting for tuning maxSize
 * - getOrCompute() for lazy initialization pattern
 *
 * @module lib/perf/lru-cache
 */

/**
 * Cache statistics for performance analysis.
 */
export interface CacheStats {
  readonly size: number;
  readonly maxSize: number;
  readonly hits: number;
  readonly misses: number;
  readonly evictions: number;
  /** Hit rate as ratio (0.0 to 1.0), null if no accesses */
  readonly hitRate: number | null;
}

export class LRUCache<K, V> {
  private cache = new Map<K, V>();
  private _hits = 0;
  private _misses = 0;
  private _evictions = 0;

  constructor(private readonly maxSize: number) {
    if (maxSize <= 0) {
      throw new Error("LRU cache maxSize must be positive");
    }
  }

  /**
   * Get a value from the cache, promoting it to most recently used.
   * Tracks hit/miss statistics.
   */
  get(key: K): V | undefined {
    const value = this.cache.get(key);
    if (value !== undefined) {
      this._hits++;
      // Move to end (most recently used) by re-inserting
      this.cache.delete(key);
      this.cache.set(key, value);
      return value;
    }
    this._misses++;
    return undefined;
  }

  /**
   * Get value or compute if missing (lazy initialization pattern).
   *
   * This is the primary API for cache usage - avoids redundant computation.
   */
  getOrCompute(key: K, compute: () => V): V {
    const existing = this.cache.get(key);
    if (existing !== undefined) {
      this._hits++;
      // Promote to MRU
      this.cache.delete(key);
      this.cache.set(key, existing);
      return existing;
    }

    this._misses++;
    const value = compute();
    this.setInternal(key, value);
    return value;
  }

  /**
   * Set a value in the cache, evicting LRU entry if at capacity.
   */
  set(key: K, value: V): void {
    this.setInternal(key, value);
  }

  private setInternal(key: K, value: V): void {
    if (this.cache.has(key)) {
      this.cache.delete(key);
    } else if (this.cache.size >= this.maxSize) {
      // Evict oldest (first entry in Map)
      const oldestKey = this.cache.keys().next().value;
      if (oldestKey !== undefined) {
        this.cache.delete(oldestKey);
        this._evictions++;
      }
    }
    this.cache.set(key, value);
  }

  /**
   * Check if key exists without promoting it or affecting stats.
   */
  has(key: K): boolean {
    return this.cache.has(key);
  }

  /**
   * Delete a key from the cache.
   */
  delete(key: K): boolean {
    return this.cache.delete(key);
  }

  /**
   * Clear all entries and reset statistics.
   */
  clear(): void {
    this.cache.clear();
    this._hits = 0;
    this._misses = 0;
    this._evictions = 0;
  }

  /**
   * Get current size.
   */
  get size(): number {
    return this.cache.size;
  }

  /**
   * Get all keys (from oldest to newest).
   */
  keys(): IterableIterator<K> {
    return this.cache.keys();
  }

  /**
   * Get all values (from oldest to newest).
   */
  values(): IterableIterator<V> {
    return this.cache.values();
  }

  /**
   * Get cache statistics for performance analysis.
   */
  getStats(): CacheStats {
    const totalAccesses = this._hits + this._misses;
    return {
      size: this.cache.size,
      maxSize: this.maxSize,
      hits: this._hits,
      misses: this._misses,
      evictions: this._evictions,
      hitRate: totalAccesses > 0 ? this._hits / totalAccesses : null,
    };
  }

  /**
   * Reset statistics without clearing cache entries.
   */
  resetStats(): void {
    this._hits = 0;
    this._misses = 0;
    this._evictions = 0;
  }
}

// Singleton caches for common use cases
const MESSAGE_CACHE_SIZE = 500;
const TRANSFORM_CACHE_SIZE = 100;

let messageCache: LRUCache<string, unknown> | null = null;
let transformCache: LRUCache<string, unknown> | null = null;

/**
 * Get the shared message cache instance.
 */
export function getMessageCache<V>(): LRUCache<string, V> {
  if (!messageCache) {
    messageCache = new LRUCache<string, unknown>(MESSAGE_CACHE_SIZE);
  }
  return messageCache as LRUCache<string, V>;
}

/**
 * Get the shared transform cache instance.
 */
export function getTransformCache<V>(): LRUCache<string, V> {
  if (!transformCache) {
    transformCache = new LRUCache<string, unknown>(TRANSFORM_CACHE_SIZE);
  }
  return transformCache as LRUCache<string, V>;
}
