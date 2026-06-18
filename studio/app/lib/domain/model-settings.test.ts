import { describe, expect, test } from "bun:test";

import {
  DEFAULT_MODEL_SETTINGS,
  isReasoningEffort,
  modelSettingsToChatFields,
  normalizeModelSettings,
  overrideModelSettings,
} from "./model-settings";

describe("model settings description", () => {
  test("normalizes invalid numeric and effort inputs to safe defaults", () => {
    expect(
      normalizeModelSettings({
        maxTokens: 0,
        thinkingBudgetTokens: -1,
        reasoningEffort: "turbo" as never,
        cacheControl: "yes" as never,
      })
    ).toEqual(DEFAULT_MODEL_SETTINGS);
  });

  test("rounds token settings while preserving max effort", () => {
    const settings = normalizeModelSettings({
      maxTokens: 4096.4,
      thinkingBudgetTokens: 1024.6,
      reasoningEffort: "max",
      cacheControl: false,
    });

    expect(settings).toEqual({
      maxTokens: 4096,
      thinkingBudgetTokens: 1025,
      reasoningEffort: "max",
      cacheControl: false,
    });
    expect(modelSettingsToChatFields(settings)).toEqual(settings);
  });

  test("overrides preserve base values when fields are undefined", () => {
    const base = normalizeModelSettings({
      maxTokens: 8192,
      thinkingBudgetTokens: 0,
      reasoningEffort: "medium",
      cacheControl: true,
    });

    expect(
      overrideModelSettings(base, {
        thinkingBudgetTokens: 2048,
        cacheControl: false,
      })
    ).toEqual({
      maxTokens: 8192,
      thinkingBudgetTokens: 2048,
      reasoningEffort: "medium",
      cacheControl: false,
    });
    expect(overrideModelSettings(base, { reasoningEffort: undefined })).toEqual(base);
  });

  test("effort guard admits only backend-supported values", () => {
    expect(isReasoningEffort("low")).toBe(true);
    expect(isReasoningEffort("medium")).toBe(true);
    expect(isReasoningEffort("high")).toBe(true);
    expect(isReasoningEffort("max")).toBe(true);
    expect(isReasoningEffort("turbo")).toBe(false);
  });
});
