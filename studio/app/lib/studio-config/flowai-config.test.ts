import { describe, expect, test } from "bun:test";
import {
  DEFAULT_FLOWAI_STUDIO_CONFIG,
  type FlowAIStudioConfig,
  normalizeFlowAIStudioConfig,
} from "./flowai-config";

describe("FlowAI Studio bootstrap config", () => {
  test("normalizes current /__flowai_config.js shape", () => {
    const config = normalizeFlowAIStudioConfig({
      appName: "Acme pricing",
      apiBaseUrl: "/api",
      studioApiVersion: "harness-studio/v1",
      defaultWorkspaceKey: "local-dev",
      streamTransport: "sse",
    });

    expect(config).toEqual({
      appName: "Acme pricing",
      apiBaseUrl: "/api",
      studioApiVersion: "harness-studio/v1",
      defaultWorkspaceKey: "local-dev",
      streamTransport: "sse",
    } satisfies FlowAIStudioConfig);
  });

  test("falls back deterministically for tests and unpackaged static views", () => {
    expect(normalizeFlowAIStudioConfig(undefined)).toEqual(DEFAULT_FLOWAI_STUDIO_CONFIG);
    expect(normalizeFlowAIStudioConfig({ appName: "" }).appName).toBe("FlowAI Studio");
  });
});
