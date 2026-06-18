export interface NamespacedStorage {
  getItem: <T>(key: string) => T | null;
  setItem: <T>(key: string, value: T) => void;
  removeItem: (key: string) => void;
  hasItem: (key: string) => boolean;
  clear: () => void;
  keys: () => string[];
  subscribe: (key: string, callback: (value: unknown) => void) => () => void;
}

export interface ZustandStorageAdapter {
  getItem: (name: string) => string | null;
  setItem: (name: string, value: string) => void;
  removeItem: (name: string) => void;
}

export interface StorageNamespaceConfig {
  readonly settings: string;
  readonly threads: string;
  readonly cache: string;
}

export interface StorageNamespaceState {
  readonly getNamespace: () => string | null;
  readonly setNamespace: (namespace: string) => void;
  readonly clearNamespace: () => void;
}

export interface NamespacedStorageOptions {
  readonly namespaceState: StorageNamespaceState;
  readonly storage?: Storage | null;
}

export interface StudioStorageAdapters {
  readonly settingsStorage: NamespacedStorage;
  readonly threadsStorage: NamespacedStorage;
  readonly cacheStorage: NamespacedStorage;
  readonly zustandSettingsStorage: ZustandStorageAdapter;
  readonly zustandThreadsStorage: ZustandStorageAdapter;
}

export function createStorageNamespaceState(
  initialNamespace: string | null = null
): StorageNamespaceState {
  let currentNamespace = normalizeNamespace(initialNamespace);

  return {
    getNamespace: () => currentNamespace,
    setNamespace: (namespace: string) => {
      const normalized = normalizeNamespace(namespace);
      if (!normalized) {
        console.warn("[NamespacedStorage] Invalid namespace provided");
        return;
      }
      currentNamespace = normalized;
    },
    clearNamespace: () => {
      currentNamespace = null;
    },
  };
}

export function createNamespacedStorage(
  baseKey: string,
  options: NamespacedStorageOptions
): NamespacedStorage {
  const storage = options.storage ?? (typeof window !== "undefined" ? window.localStorage : null);
  if (!storage) {
    return createNoopStorage();
  }

  function buildKey(key: string): string | null {
    const currentNamespace = options.namespaceState.getNamespace();
    if (!currentNamespace) {
      if (import.meta.env.DEV) {
        console.warn(`[NamespacedStorage] Namespace not set, operation skipped for key: ${key}`);
      }
      return null;
    }
    return `${baseKey}-${currentNamespace}-${key}`;
  }

  function getNamespacePrefix(): string | null {
    const currentNamespace = options.namespaceState.getNamespace();
    if (!currentNamespace) return null;
    return `${baseKey}-${currentNamespace}-`;
  }

  const subscribers = new Map<string, Set<(value: unknown) => void>>();

  function handleStorageEvent(event: StorageEvent): void {
    if (!event.key || !event.newValue) return;

    const prefix = getNamespacePrefix();
    if (!prefix || !event.key.startsWith(prefix)) return;

    const relativeKey = event.key.slice(prefix.length);
    const callbacks = subscribers.get(relativeKey);
    if (!callbacks) return;

    try {
      const value = JSON.parse(event.newValue);
      for (const callback of callbacks) {
        callback(value);
      }
    } catch {
      // Ignore parse errors from corrupted storage rows.
    }
  }

  if (typeof window !== "undefined") {
    window.addEventListener("storage", handleStorageEvent);
  }

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
        const callbacks = subscribers.get(key);
        if (!callbacks) return;
        for (const callback of callbacks) {
          callback(value);
        }
      } catch (error) {
        console.error(`[NamespacedStorage] Failed to set item: ${key}`, error);
      }
    },

    removeItem(key: string): void {
      const fullKey = buildKey(key);
      if (!fullKey) return;

      storage.removeItem(fullKey);
      const callbacks = subscribers.get(key);
      if (!callbacks) return;
      for (const callback of callbacks) {
        callback(null);
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

      const keysToRemove: string[] = [];
      for (let index = 0; index < storage.length; index++) {
        const key = storage.key(index);
        if (key?.startsWith(prefix)) {
          keysToRemove.push(key);
        }
      }

      for (const key of keysToRemove) {
        storage.removeItem(key);
      }

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
      for (let index = 0; index < storage.length; index++) {
        const key = storage.key(index);
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

      return () => {
        callbacks?.delete(callback);
        if (callbacks?.size === 0) {
          subscribers.delete(key);
        }
      };
    },
  };
}

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

export function createZustandStorage(
  baseKey: string,
  options: NamespacedStorageOptions
): ZustandStorageAdapter {
  const storage = options.storage ?? (typeof window !== "undefined" ? window.localStorage : null);

  return {
    getItem(_name: string): string | null {
      if (!storage) return null;
      const currentNamespace = options.namespaceState.getNamespace();
      if (!currentNamespace) {
        if (import.meta.env.DEV) {
          console.warn(`[ZustandStorage] getItem(${baseKey}): namespace not set`);
        }
        return null;
      }
      return storage.getItem(`${baseKey}-${currentNamespace}`);
    },

    setItem(_name: string, value: string): void {
      if (!storage) return;
      const currentNamespace = options.namespaceState.getNamespace();
      if (!currentNamespace) {
        if (import.meta.env.DEV) {
          console.warn(`[ZustandStorage] setItem(${baseKey}): namespace not set, skipping`);
        }
        return;
      }
      storage.setItem(`${baseKey}-${currentNamespace}`, value);
    },

    removeItem(_name: string): void {
      if (!storage) return;
      const currentNamespace = options.namespaceState.getNamespace();
      if (!currentNamespace) return;
      storage.removeItem(`${baseKey}-${currentNamespace}`);
    },
  };
}

export function createStudioStorage(
  config: StorageNamespaceConfig,
  options: NamespacedStorageOptions
): StudioStorageAdapters {
  return {
    settingsStorage: createNamespacedStorage(config.settings, options),
    threadsStorage: createNamespacedStorage(config.threads, options),
    cacheStorage: createNamespacedStorage(config.cache, options),
    zustandSettingsStorage: createZustandStorage(config.settings, options),
    zustandThreadsStorage: createZustandStorage(config.threads, options),
  };
}

function normalizeNamespace(namespace: string | null | undefined): string | null {
  const normalized = namespace?.trim();
  return normalized ? normalized : null;
}
