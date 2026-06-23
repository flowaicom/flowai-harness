import { describe, expect, test } from "bun:test";
import {
  type ModelConfig,
  sanitizeSettingsPersistence,
  selectModelSettings,
  useAgentConfig,
} from "./settings-store";

describe("settings store selectors", () => {
  test("model settings selector returns a stable reference for unchanged inputs", () => {
    const state = useAgentConfig.getState();
    const first = selectModelSettings(state);
    const second = selectModelSettings(state);

    expect(first).toBe(second);
  });
});

describe("settings store persistence", () => {
  test("removes provider and Neon API keys before browser persistence", () => {
    const persisted = sanitizeSettingsPersistence({
      providerSettings: {
        openai: {
          apiKey: "sk-provider",
          organization: "org-123",
        },
        bedrock: {
          region: "us-east-1",
          accessKey: "aws-access-key",
          secretAccessKey: "aws-secret-key",
        },
      },
      neondbApiKey: "neon-api-key",
      neondbProjectId: "project-ref",
      theme: "slate",
    });

    expect(persisted).toEqual({
      providerSettings: {
        openai: {
          organization: "org-123",
        },
        bedrock: {
          region: "us-east-1",
        },
      },
      neondbProjectId: "project-ref",
      theme: "slate",
    });
  });

  test("removes dynamically declared secret provider settings", () => {
    const availableModels: ModelConfig[] = [
      {
        key: "custom-provider",
        model: "model",
        displayName: "Custom Provider",
        description: "",
        available: true,
        endpointSettings: [
          {
            key: "tenant",
            displayName: "Tenant",
            description: "",
            kind: "text",
            options: [],
          },
        ],
        settings: [
          {
            key: "deploymentId",
            displayName: "Deployment ID",
            description: "",
            kind: "secret",
            options: [],
          },
          {
            key: "region",
            displayName: "Region",
            description: "",
            kind: "text",
            options: [],
          },
        ],
      },
    ];

    const persisted = sanitizeSettingsPersistence(
      {
        providerSettings: {
          "custom-provider": {
            tenant: "tenant-a",
            deploymentId: "secret-deployment",
            region: "eu-west-1",
          },
        },
      },
      availableModels
    );

    expect(persisted).toEqual({
      providerSettings: {
        "custom-provider": {
          tenant: "tenant-a",
          region: "eu-west-1",
        },
      },
    });
  });
});
