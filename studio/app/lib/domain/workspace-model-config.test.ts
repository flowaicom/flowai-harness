import { describe, expect, test } from "bun:test";

import {
  defaultWorkspaceModelConfigState,
  deriveWorkspaceModelConfigState,
  modelConfigSyncPayload,
  stripWorkspaceModelConfigPersistence,
} from "./workspace-model-config";

describe("workspace model configuration description", () => {
  test("derives provider selection and provider-local model IDs from backend config", () => {
    const state = deriveWorkspaceModelConfigState({
      defaultModels: {
        coordinator: "anthropic",
        planner: "bedrock",
      },
      selectedModels: {
        coordinator: "anthropic/claude-opus-4-6",
        planner: "bedrock/anthropic.claude-sonnet-4-5-20250929-v1:0",
      },
      modelSettings: {
        maxTokens: 8192,
        thinkingBudgetTokens: 0,
        reasoningEffort: "max",
        cacheControl: false,
      },
    });

    expect(state).toEqual({
      agentModels: {
        coordinator: "anthropic",
        planner: "bedrock",
      },
      agentSelectedModels: {
        coordinator: "claude-opus-4-6",
        planner: "anthropic.claude-sonnet-4-5-20250929-v1:0",
      },
      maxTokens: 8192,
      thinkingBudgetTokens: 0,
      reasoningEffort: "max",
      cacheControl: false,
    });
  });

  test("preserves bare custom model IDs and defaults missing runtime settings", () => {
    expect(
      deriveWorkspaceModelConfigState({
        defaultModels: { coordinator: "anthropic" },
        selectedModels: { coordinator: "claude-opus-4-6" },
      })
    ).toEqual({
      agentModels: { coordinator: "anthropic" },
      agentSelectedModels: { coordinator: "claude-opus-4-6" },
      maxTokens: 16384,
      thinkingBudgetTokens: 0,
      reasoningEffort: "high",
      cacheControl: true,
    });
  });

  test("falls back to a transient default role when backend exposes no roles", () => {
    expect(
      deriveWorkspaceModelConfigState({
        defaultModels: {},
        selectedModels: {},
        modelSettings: null,
      })
    ).toEqual(defaultWorkspaceModelConfigState());
  });

  test("builds backend payloads without double-prefixing already-qualified model IDs", () => {
    expect(
      modelConfigSyncPayload({
        agentModels: {
          coordinator: "anthropic",
          planner: "bedrock",
        },
        agentSelectedModels: {
          coordinator: "anthropic/claude-opus-4-6",
          planner: "anthropic.claude-sonnet-4-5-20250929-v1:0",
        },
        maxTokens: 4096,
        thinkingBudgetTokens: 2048,
        reasoningEffort: "medium",
        cacheControl: true,
      })
    ).toEqual({
      roleModels: {
        coordinator: "anthropic/claude-opus-4-6",
        planner: "bedrock/anthropic.claude-sonnet-4-5-20250929-v1:0",
      },
      maxTokens: 4096,
      thinkingBudgetTokens: 2048,
      reasoningEffort: "medium",
      cacheControl: true,
    });
  });

  test("strips stale globally persisted model config fields during store migration", () => {
    expect(
      stripWorkspaceModelConfigPersistence({
        agentModels: { coordinator: "anthropic" },
        agentSelectedModels: { coordinator: "claude-opus-4-6" },
        agentCustomBaseUrls: { coordinator: "http://localhost:4000" },
        agentCustomModelNames: { coordinator: "custom-model" },
        agentCustomApiKeys: { coordinator: "sk-old" },
        agentCustomEndpoints: { coordinator: { settings: { baseUrl: "http://localhost:4000" } } },
        maxTokens: 4096,
        thinkingBudgetTokens: 2048,
        reasoningEffort: "max",
        cacheControl: false,
        providerSettings: { anthropic: { apiKey: "sk-provider" } },
        featureFlags: { piiScramble: true },
        theme: "slate",
      })
    ).toEqual({
      providerSettings: { anthropic: { apiKey: "sk-provider" } },
      featureFlags: { piiScramble: true },
      theme: "slate",
    });
  });
});
