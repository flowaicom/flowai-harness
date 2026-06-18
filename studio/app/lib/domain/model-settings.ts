/**
 * Pure model runtime settings description.
 *
 * UI stores and API clients interpret this value; they should not each invent
 * separate token/thinking normalization rules.
 */

export const REASONING_EFFORTS = ["low", "medium", "high", "max"] as const;

export type ReasoningEffort = (typeof REASONING_EFFORTS)[number];

export interface ModelSettings {
  readonly maxTokens: number;
  readonly thinkingBudgetTokens: number;
  readonly reasoningEffort: ReasoningEffort;
  readonly cacheControl: boolean;
}

export const DEFAULT_MODEL_SETTINGS: ModelSettings = Object.freeze({
  maxTokens: 16384,
  thinkingBudgetTokens: 0,
  reasoningEffort: "high",
  cacheControl: true,
});

export function isReasoningEffort(value: unknown): value is ReasoningEffort {
  return typeof value === "string" && REASONING_EFFORTS.includes(value as ReasoningEffort);
}

export function normalizeModelSettings(
  value: Partial<ModelSettings> | null | undefined
): ModelSettings {
  return {
    maxTokens: normalizePositiveInteger(value?.maxTokens, DEFAULT_MODEL_SETTINGS.maxTokens),
    thinkingBudgetTokens: normalizeNonNegativeInteger(
      value?.thinkingBudgetTokens,
      DEFAULT_MODEL_SETTINGS.thinkingBudgetTokens
    ),
    reasoningEffort: isReasoningEffort(value?.reasoningEffort)
      ? value.reasoningEffort
      : DEFAULT_MODEL_SETTINGS.reasoningEffort,
    cacheControl:
      typeof value?.cacheControl === "boolean"
        ? value.cacheControl
        : DEFAULT_MODEL_SETTINGS.cacheControl,
  };
}

export function overrideModelSettings(
  base: ModelSettings,
  overrides: Partial<ModelSettings> | null | undefined
): ModelSettings {
  return normalizeModelSettings({
    ...base,
    ...Object.fromEntries(
      Object.entries(overrides ?? {}).filter(([, value]) => value !== undefined)
    ),
  });
}

export function modelSettingsToChatFields(settings: ModelSettings): {
  readonly maxTokens: number;
  readonly thinkingBudgetTokens: number;
  readonly reasoningEffort: ReasoningEffort;
  readonly cacheControl: boolean;
} {
  const normalized = normalizeModelSettings(settings);
  return {
    maxTokens: normalized.maxTokens,
    thinkingBudgetTokens: normalized.thinkingBudgetTokens,
    reasoningEffort: normalized.reasoningEffort,
    cacheControl: normalized.cacheControl,
  };
}

function normalizePositiveInteger(value: unknown, fallback: number): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
    return fallback;
  }
  return Math.max(1, Math.round(value));
}

function normalizeNonNegativeInteger(value: unknown, fallback: number): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    return fallback;
  }
  return Math.max(0, Math.round(value));
}
