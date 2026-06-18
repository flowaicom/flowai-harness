/**
 * Namespaced Storage for multi-tenant isolation.
 *
 * Provides tenant-scoped localStorage/sessionStorage access:
 * - Keys are prefixed with namespace (resourceId)
 * - Safe no-ops until namespace is set
 * - Cross-tab synchronization via storage events
 *
 * @module lib/storage/namespaced-storage
 */

// ============================================================================
// Configuration
// ============================================================================

let currentNamespace: string | null = null;

/**
 * Set the storage namespace (typically resourceId).
 * Must be called before any storage operations.
 */
export function setStorageNamespace(namespace: string): void {
  if (!namespace || namespace.trim() === "") {
    console.warn("[NamespacedStorage] Invalid namespace provided");
    return;
  }
  currentNamespace = namespace.trim();
}

/**
 * Get the current storage namespace.
 */
export function getStorageNamespace(): string | null {
  return currentNamespace;
}

/**
 * Clear the storage namespace (for logout/tenant switch).
 */
export function clearStorageNamespace(): void {
  currentNamespace = null;
}

// ============================================================================
// Namespaced Storage Interface
// ============================================================================

export interface NamespacedStorage {
  /** Get item by key */
  getItem: <T>(key: string) => T | null;
  /** Set item by key */
  setItem: <T>(key: string, value: T) => void;
  /** Remove item by key */
  removeItem: (key: string) => void;
  /** Check if key exists */
  hasItem: (key: string) => boolean;
  /** Clear all items for this namespace */
  clear: () => void;
  /** Get all keys for this namespace */
  keys: () => string[];
  /** Subscribe to changes */
  subscribe: (key: string, callback: (value: unknown) => void) => () => void;
}

// ============================================================================
// Factory
// ============================================================================

/**
 * Create a namespaced storage adapter.
 *
 * @param baseKey - Prefix for all keys (e.g., "settings", "threads")
 * @param storage - Storage backend (localStorage or sessionStorage)
 */
export function createNamespacedStorage(
  baseKey: string,
  storage: Storage | null = typeof window !== "undefined" ? window.localStorage : null
): NamespacedStorage {
  // SSR safety
  if (typeof window === "undefined" || !storage) {
    return createNoopStorage();
  }

  /**
   * Build the full storage key with namespace.
   */
  function buildKey(key: string): string | null {
    if (!currentNamespace) {
      if (import.meta.env.DEV) {
        console.warn(`[NamespacedStorage] Namespace not set, operation skipped for key: ${key}`);
      }
      return null;
    }
    return `${baseKey}-${currentNamespace}-${key}`;
  }

  /**
   * Build the namespace prefix for iteration.
   */
  function getNamespacePrefix(): string | null {
    if (!currentNamespace) return null;
    return `${baseKey}-${currentNamespace}-`;
  }

  // Subscription management
  const subscribers = new Map<string, Set<(value: unknown) => void>>();

  // Cross-tab sync listener
  function handleStorageEvent(e: StorageEvent): void {
    if (!e.key || !e.newValue) return;

    const prefix = getNamespacePrefix();
    if (!prefix || !e.key.startsWith(prefix)) return;

    // Extract the relative key
    const relativeKey = e.key.slice(prefix.length);
    const callbacks = subscribers.get(relativeKey);

    if (callbacks) {
      try {
        const value = JSON.parse(e.newValue);
        for (const callback of callbacks) {
          callback(value);
        }
      } catch {
        // Ignore parse errors
      }
    }
  }

  // Register storage event listener
  window.addEventListener("storage", handleStorageEvent);

  return {
    getItem<T>(key: string): T | null {
      const fullKey = buildKey(key);
      if (!fullKey) return null;

      try {
        const raw = storage.getItem(fullKey);
        if (raw === null) return null;
        return JSON.parse(raw) as T;
      } catch {
        return null;
      }
    },

    setItem<T>(key: string, value: T): void {
      const fullKey = buildKey(key);
      if (!fullKey) return;

      try {
        storage.setItem(fullKey, JSON.stringify(value));

        // Notify local subscribers (storage event only fires for other tabs)
        const callbacks = subscribers.get(key);
        if (callbacks) {
          for (const callback of callbacks) {
            callback(value);
          }
        }
      } catch (e) {
        console.error(`[NamespacedStorage] Failed to set item: ${key}`, e);
      }
    },

    removeItem(key: string): void {
      const fullKey = buildKey(key);
      if (!fullKey) return;

      storage.removeItem(fullKey);

      // Notify subscribers
      const callbacks = subscribers.get(key);
      if (callbacks) {
        for (const callback of callbacks) {
          callback(null);
        }
      }
    },

    hasItem(key: string): boolean {
      const fullKey = buildKey(key);
      if (!fullKey) return false;
      return storage.getItem(fullKey) !== null;
    },

    clear(): void {
      const prefix = getNamespacePrefix();
      if (!prefix) return;

      // Collect keys to remove (can't modify during iteration)
      const keysToRemove: string[] = [];
      for (let i = 0; i < storage.length; i++) {
        const key = storage.key(i);
        if (key?.startsWith(prefix)) {
          keysToRemove.push(key);
        }
      }

      for (const key of keysToRemove) {
        storage.removeItem(key);
      }

      // Notify all subscribers
      for (const [, callbacks] of subscribers) {
        for (const callback of callbacks) {
          callback(null);
        }
      }
    },

    keys(): string[] {
      const prefix = getNamespacePrefix();
      if (!prefix) return [];

      const result: string[] = [];
      for (let i = 0; i < storage.length; i++) {
        const key = storage.key(i);
        if (key?.startsWith(prefix)) {
          result.push(key.slice(prefix.length));
        }
      }
      return result;
    },

    subscribe(key: string, callback: (value: unknown) => void): () => void {
      let callbacks = subscribers.get(key);
      if (!callbacks) {
        callbacks = new Set();
        subscribers.set(key, callbacks);
      }
      callbacks.add(callback);

      // Return unsubscribe function
      return () => {
        callbacks?.delete(callback);
        if (callbacks?.size === 0) {
          subscribers.delete(key);
        }
      };
    },
  };
}

/**
 * Create a no-op storage for SSR.
 */
function createNoopStorage(): NamespacedStorage {
  return {
    getItem: () => null,
    setItem: () => {},
    removeItem: () => {},
    hasItem: () => false,
    clear: () => {},
    keys: () => [],
    subscribe: () => () => {},
  };
}

// ============================================================================
// Zustand Persist-Compatible Adapter
// ============================================================================

/**
 * Zustand-compatible storage interface.
 * Used with createJSONStorage() from zustand/middleware.
 */
export interface ZustandStorageAdapter {
  getItem: (name: string) => string | null;
  setItem: (name: string, value: string) => void;
  removeItem: (name: string) => void;
}

/**
 * Create a Zustand-compatible storage adapter.
 *
 * Unlike createNamespacedStorage, this returns the raw string interface
 * that Zustand's persist middleware expects. The storage key becomes:
 * `{baseKey}-{namespace}` (namespace is set via setStorageNamespace).
 *
 * @param baseKey - Base storage key (e.g., "settings", "threads")
 */
export function createZustandStorage(baseKey: string): ZustandStorageAdapter {
  return {
    getItem(_name: string): string | null {
      if (typeof window === "undefined") return null;
      if (!currentNamespace) {
        if (import.meta.env.DEV) {
          console.warn(`[ZustandStorage] getItem(${baseKey}): namespace not set`);
        }
        return null;
      }
      const key = `${baseKey}-${currentNamespace}`;
      return localStorage.getItem(key);
    },

    setItem(_name: string, value: string): void {
      if (typeof window === "undefined") return;
      if (!currentNamespace) {
        if (import.meta.env.DEV) {
          console.warn(`[ZustandStorage] setItem(${baseKey}): namespace not set, skipping`);
        }
        return;
      }
      const key = `${baseKey}-${currentNamespace}`;
      localStorage.setItem(key, value);
    },

    removeItem(_name: string): void {
      if (typeof window === "undefined") return;
      if (!currentNamespace) {
        return;
      }
      const key = `${baseKey}-${currentNamespace}`;
      localStorage.removeItem(key);
    },
  };
}

// ============================================================================
// Pre-created Adapters
// ============================================================================

/**
 * Settings storage (persists user preferences).
 */
export const settingsStorage = createNamespacedStorage("studio-settings");

/**
 * Threads storage (persists thread metadata).
 */
export const threadsStorage = createNamespacedStorage("studio-threads");

/**
 * Cache storage (temporary data with potential TTL).
 */
export const cacheStorage = createNamespacedStorage("studio-cache");

/**
 * Zustand-compatible storage for settings store.
 * Usage: createJSONStorage(() => zustandSettingsStorage)
 */
export const zustandSettingsStorage = createZustandStorage("studio-settings");

/**
 * Zustand-compatible storage for threads store.
 */
export const zustandThreadsStorage = createZustandStorage("studio-threads");
