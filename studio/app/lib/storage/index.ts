/**
 * Storage module for multi-tenant data persistence.
 *
 * @module lib/storage
 */

export {
  cacheStorage,
  clearStorageNamespace,
  createNamespacedStorage,
  createZustandStorage,
  getStorageNamespace,
  type NamespacedStorage,
  setStorageNamespace,
  settingsStorage,
  threadsStorage,
  type ZustandStorageAdapter,
  zustandSettingsStorage,
  zustandThreadsStorage,
} from "./namespaced-storage";
