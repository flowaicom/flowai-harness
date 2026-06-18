import { describe, expect, test } from "bun:test";
import {
  createNamespacedStorage,
  createStorageNamespaceState,
  createStudioStorage,
  createZustandStorage,
} from "./namespaced-storage";

function createMemoryStorage(): Storage {
  const map = new Map<string, string>();

  return {
    get length() {
      return map.size;
    },
    clear() {
      map.clear();
    },
    getItem(key: string) {
      return map.get(key) ?? null;
    },
    key(index: number) {
      return Array.from(map.keys())[index] ?? null;
    },
    removeItem(key: string) {
      map.delete(key);
    },
    setItem(key: string, value: string) {
      map.set(key, value);
    },
  };
}

describe("namespaced storage substrate", () => {
  test("isolates values by namespace state", () => {
    const namespaceState = createStorageNamespaceState();
    const storage = createMemoryStorage();
    const namespacedStorage = createNamespacedStorage("studio-settings", {
      namespaceState,
      storage,
    });

    namespaceState.setNamespace("workspace-a");
    namespacedStorage.setItem("theme", { mode: "dark" });

    namespaceState.setNamespace("workspace-b");
    expect(namespacedStorage.getItem("theme")).toBeNull();

    namespacedStorage.setItem("theme", { mode: "light" });

    namespaceState.setNamespace("workspace-a");
    expect(namespacedStorage.getItem<{ mode: string }>("theme")).toEqual({ mode: "dark" });

    namespaceState.setNamespace("workspace-b");
    expect(namespacedStorage.getItem<{ mode: string }>("theme")).toEqual({ mode: "light" });
  });

  test("createStudioStorage shares namespace state across all adapters", () => {
    const namespaceState = createStorageNamespaceState("workspace-a");
    const storage = createMemoryStorage();
    const studioStorage = createStudioStorage(
      {
        settings: "studio-settings",
        threads: "studio-threads",
        cache: "studio-cache",
      },
      {
        namespaceState,
        storage,
      }
    );

    studioStorage.settingsStorage.setItem("theme", { mode: "dark" });
    studioStorage.threadsStorage.setItem("current", { id: "thread-1" });
    studioStorage.zustandSettingsStorage.setItem("zustand", JSON.stringify({ tab: "general" }));

    namespaceState.setNamespace("workspace-b");
    expect(studioStorage.settingsStorage.getItem("theme")).toBeNull();
    expect(studioStorage.threadsStorage.getItem("current")).toBeNull();
    expect(studioStorage.zustandSettingsStorage.getItem("zustand")).toBeNull();

    namespaceState.setNamespace("workspace-a");
    expect(studioStorage.settingsStorage.getItem<{ mode: string }>("theme")).toEqual({
      mode: "dark",
    });
    expect(studioStorage.threadsStorage.getItem<{ id: string }>("current")).toEqual({
      id: "thread-1",
    });
    expect(studioStorage.zustandSettingsStorage.getItem("zustand")).toBe(
      JSON.stringify({ tab: "general" })
    );
  });

  test("createZustandStorage uses the provided namespace controller", () => {
    const namespaceState = createStorageNamespaceState("workspace-a");
    const storage = createMemoryStorage();
    const zustandStorage = createZustandStorage("studio-settings", {
      namespaceState,
      storage,
    });

    zustandStorage.setItem("ignored", '{"theme":"dark"}');
    expect(zustandStorage.getItem("ignored")).toBe('{"theme":"dark"}');

    namespaceState.setNamespace("workspace-b");
    expect(zustandStorage.getItem("ignored")).toBeNull();
  });
});
