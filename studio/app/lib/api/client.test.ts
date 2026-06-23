import { afterEach, describe, expect, test } from "bun:test";
import { getApiConfig, getApiRequestHeaders, setApiConfig } from "./client";

const originalConfig = getApiConfig();
const originalWindow = globalThis.window;

afterEach(() => {
  setApiConfig(originalConfig);
  if (originalWindow === undefined) {
    Reflect.deleteProperty(globalThis, "window");
  } else {
    Object.defineProperty(globalThis, "window", {
      value: originalWindow,
      configurable: true,
    });
  }
});

describe("API client headers", () => {
  test("adds Studio auth token from bootstrap config", () => {
    Object.defineProperty(globalThis, "window", {
      value: { __FLOWAI__: { studioAuthToken: "token-123" } },
      configurable: true,
    });
    setApiConfig({
      baseUrl: "/api",
      timeout: 30_000,
      headers: { "Content-Type": "application/json" },
    });

    expect(getApiRequestHeaders({ Accept: "text/event-stream" })).toEqual({
      "Content-Type": "application/json",
      "X-FlowAI-Studio-Token": "token-123",
      accept: "text/event-stream",
    });
  });
});
