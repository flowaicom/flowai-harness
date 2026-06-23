/**
 * Settings Store for Per-Agent Model Configuration
 *
 * Manages LLM model selection for each agent role defined by the project.
 * Agent roles are dynamic strings loaded from the backend (via /api/model-config
 * or /api/studio/project). Before config loads, the store uses a transient
 * single "default" role in memory only.
 *
 * Fetches available models from the backend and persists selection.
 *
 * @module stores/settings-store
 */

import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";
import { useShallow } from "zustand/react/shallow";
import { apiFetch } from "~/lib/api/client";
import {
  isReasoningEffort,
  type ModelSettings,
  normalizeModelSettings,
  type ReasoningEffort,
} from "~/lib/domain/model-settings";
import {
  defaultWorkspaceModelConfigState,
  deriveWorkspaceModelConfigState,
  modelConfigSyncPayload,
  stripWorkspaceModelConfigPersistence,
  type WorkspaceModelConfigState,
} from "~/lib/domain/workspace-model-config";
import { zustandSettingsStorage } from "~/lib/storage";
import { useWorkspace } from "./workspace-store";

// ============================================================================
// Types
// ============================================================================

/** Provider/model selection key — dynamic string loaded from backend config. */
export type ModelKey = string;
/** Agent role identifier — dynamic string loaded from project config. */
export type AgentRole = string;
/** Provider identifier — dynamic string loaded from backend config. */
export type ProviderKey = string;

/** UI theme mode */
export type ThemeMode = "system" | "light" | "dark" | "slate";

export type { ModelSettings, ReasoningEffort };

/** Per-agent model selection (keys are dynamic role names). */
export type AgentModelSelection = Record<string, ModelKey>;

export interface ProviderSettingOption {
  key: string;
  displayName: string;
  description: string;
}

export type ProviderSettingKind = "secret" | "text" | "select";

export interface ProviderSetting {
  key: string;
  displayName: string;
  description: string;
  kind: ProviderSettingKind;
  required?: boolean;
  defaultValue?: string | null;
  options: ProviderSettingOption[];
}

export interface ModelConfig {
  key: ModelKey;
  model: string;
  displayName: string;
  description: string;
  available: boolean;
  endpointTransport?: string | null;
  endpointTransportDisplayName?: string | null;
  endpointTransportDescription?: string | null;
  endpointSettings: ProviderSetting[];
  settings: ProviderSetting[];
}

/** Custom endpoint override for an agent role. */
export interface AgentCustomEndpoint {
  settings: Record<string, string>;
  targetModel?: string;
}

/** Custom endpoint overrides per agent role. */
export type AgentCustomEndpoints = Partial<Record<AgentRole, AgentCustomEndpoint>>;

/** Selected specific model ID per agent (within the chosen provider) */
export type AgentSelectedModels = Partial<Record<AgentRole, string>>;

/** Generic user-provided provider settings, keyed by provider then setting key. */
export type ProviderSettings = Partial<Record<ProviderKey, Record<string, string>>>;

type LegacyProviderApiKeys = Partial<Record<ProviderKey, string>>;

const SENSITIVE_PROVIDER_SETTING_MARKERS = [
  "apikey",
  "token",
  "secret",
  "password",
  "credential",
  "accesskey",
  "privatekey",
];

/** Model pricing info */
export interface ModelPricing {
  inputPerMTok: number;
  outputPerMTok: number;
}

/** Provider model from list-provider-models endpoint */
export interface ProviderModel {
  id: string;
  name?: string;
  displayName: string;
  provider: ProviderKey;
  description?: string;
  pricing?: ModelPricing;
  pricingSource: "gateway" | "aws-pricing" | "direct" | "none";
  capabilities?: {
    streaming?: boolean;
    maxContextLength?: number;
    maxOutputTokens?: number;
  };
}

export interface AgentInfo {
  role: AgentRole;
  displayName: string;
  description: string;
}

export interface ModelConfigResponse {
  models: ModelConfig[];
  agents: AgentInfo[];
  defaultModels: AgentModelSelection;
  selectedModels?: AgentModelSelection;
  modelSettings?: Partial<ModelSettings> | null;
  workspaceModelConfig?: {
    workspaceId: string;
    defaultModel: string;
    subAgentModel: string;
    roleModels: AgentModelSelection;
    modelSettings: ModelSettings;
    inheritsDefault: boolean;
    sourceWorkspaceId: string;
  };
}

/** Feature flags for experimental features */
export interface FeatureFlags {
  /** Export all rows in CSV vs only affected rows */
  fullCsvExport: boolean;
  /** Show latency metrics panel for debugging streaming performance */
  showLatencyPanel: boolean;
  /** Scramble business data text for demo presentations (visual only, no functional impact) */
  piiScramble: boolean;
}

// ============================================================================
// Constants
// ============================================================================

/** TTL for provider models cache (1 hour in milliseconds) */
const PROVIDER_MODELS_TTL_MS = 60 * 60 * 1000;

/** Default feature flags */
const DEFAULT_FEATURE_FLAGS: FeatureFlags = {
  fullCsvExport: true,
  showLatencyPanel: false,
  piiScramble: false,
};

let _isApplyingBackendModelConfig = false;

const normalizeStoredString = (value: string | null | undefined): string | undefined => {
  const normalized = value?.trim();
  return normalized ? normalized : undefined;
};

const normalizeCustomEndpoint = (
  endpoint: Partial<AgentCustomEndpoint> | null | undefined
): AgentCustomEndpoint | undefined => {
  const settings = Object.fromEntries(
    Object.entries(endpoint?.settings || {})
      .map(([key, value]) => [key, normalizeStoredString(value)])
      .filter((entry): entry is [string, string] => !!entry[1])
  );
  const targetModel = normalizeStoredString(endpoint?.targetModel);

  if (Object.keys(settings).length === 0) {
    return undefined;
  }

  return {
    settings,
    ...(targetModel ? { targetModel } : {}),
  };
};

const combineCustomEndpoints = (
  baseUrls?: Partial<Record<AgentRole, string>>,
  modelNames?: Partial<Record<AgentRole, string>>,
  apiKeys?: Partial<Record<AgentRole, string>>
): AgentCustomEndpoints => {
  const roles = new Set<AgentRole>([
    ...Object.keys(baseUrls || {}),
    ...Object.keys(modelNames || {}),
    ...Object.keys(apiKeys || {}),
  ]);

  const endpoints: AgentCustomEndpoints = {};
  for (const role of roles) {
    const endpoint = normalizeCustomEndpoint({
      settings: {
        ...(baseUrls?.[role] ? { baseUrl: baseUrls[role] } : {}),
        ...(apiKeys?.[role] ? { apiKey: apiKeys[role] } : {}),
      },
      targetModel: modelNames?.[role],
    });
    if (endpoint) {
      endpoints[role] = endpoint;
    }
  }

  return endpoints;
};

const upgradeCustomEndpointState = (
  state: Record<string, unknown>
): Record<string, unknown> & {
  agentCustomEndpoints: AgentCustomEndpoints;
} => {
  const {
    agentCustomBaseUrls,
    agentCustomModelNames,
    agentCustomApiKeys,
    agentCustomEndpoints,
    ...rest
  } = state as {
    agentCustomBaseUrls?: Partial<Record<AgentRole, string>>;
    agentCustomModelNames?: Partial<Record<AgentRole, string>>;
    agentCustomApiKeys?: Partial<Record<AgentRole, string>>;
    agentCustomEndpoints?: AgentCustomEndpoints;
  } & Record<string, unknown>;

  return {
    ...rest,
    agentCustomEndpoints: agentCustomEndpoints
      ? Object.fromEntries(
          Object.entries(agentCustomEndpoints)
            .map(([role, endpoint]) => [role, normalizeCustomEndpoint(endpoint)])
            .filter((entry): entry is [string, AgentCustomEndpoint] => !!entry[1])
        )
      : combineCustomEndpoints(agentCustomBaseUrls, agentCustomModelNames, agentCustomApiKeys),
  };
};

const upgradeEndpointTargetModelField = (
  state: Record<string, unknown>
): Record<string, unknown> & {
  agentCustomEndpoints: AgentCustomEndpoints;
} => {
  const currentEndpoints = (state.agentCustomEndpoints || {}) as Partial<
    Record<
      AgentRole,
      {
        settings?: Record<string, string>;
        targetModel?: string;
        modelName?: string;
      }
    >
  >;

  return {
    ...state,
    agentCustomEndpoints: Object.fromEntries(
      Object.entries(currentEndpoints)
        .map(([role, endpoint]) => [
          role,
          normalizeCustomEndpoint({
            settings: endpoint?.settings || {},
            targetModel: endpoint?.targetModel || endpoint?.modelName,
          }),
        ])
        .filter((entry): entry is [string, AgentCustomEndpoint] => !!entry[1])
    ),
  };
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isSensitiveProviderSettingKey(settingKey: string): boolean {
  const normalized = settingKey.replace(/[^a-z0-9]/gi, "").toLowerCase();
  return SENSITIVE_PROVIDER_SETTING_MARKERS.some((marker) => normalized.includes(marker));
}

function secretProviderSettingKeys(availableModels: ModelConfig[]): Map<ProviderKey, Set<string>> {
  const keysByProvider = new Map<ProviderKey, Set<string>>();

  for (const model of availableModels) {
    for (const setting of [...model.endpointSettings, ...model.settings]) {
      if (setting.kind !== "secret") continue;
      const keys = keysByProvider.get(model.key) || new Set<string>();
      keys.add(setting.key);
      keysByProvider.set(model.key, keys);
    }
  }

  return keysByProvider;
}

function sanitizeProviderSettings(
  providerSettings: unknown,
  availableModels: ModelConfig[] = []
): ProviderSettings {
  if (!isRecord(providerSettings)) return {};

  const secretKeysByProvider = secretProviderSettingKeys(availableModels);
  const sanitized: ProviderSettings = {};

  for (const [provider, settings] of Object.entries(providerSettings)) {
    if (!isRecord(settings)) continue;

    const providerSecretKeys = secretKeysByProvider.get(provider);
    const safeSettings: Record<string, string> = {};
    for (const [settingKey, value] of Object.entries(settings)) {
      if (typeof value !== "string") continue;
      if (providerSecretKeys?.has(settingKey) || isSensitiveProviderSettingKey(settingKey)) {
        continue;
      }
      safeSettings[settingKey] = value;
    }

    if (Object.keys(safeSettings).length > 0) {
      sanitized[provider] = safeSettings;
    }
  }

  return sanitized;
}

export function sanitizeSettingsPersistence(
  state: Record<string, unknown>,
  availableModels: ModelConfig[] = []
): Record<string, unknown> {
  const { neondbApiKey: _neondbApiKey, providerSettings, ...rest } = state;
  return {
    ...rest,
    providerSettings: sanitizeProviderSettings(providerSettings, availableModels),
  };
}

const stripSettingsPersistence = (state: Record<string, unknown>): Record<string, unknown> =>
  sanitizeSettingsPersistence(stripWorkspaceModelConfigPersistence(state));

// ============================================================================
// Store Interface
// ============================================================================

export interface SettingsStore {
  // State
  agentModels: AgentModelSelection;
  agentSelectedModels: AgentSelectedModels;
  agentCustomEndpoints: AgentCustomEndpoints;
  providerSettings: ProviderSettings;
  availableModels: ModelConfig[];
  providerModels: ProviderModel[];
  /** Timestamp when each provider's models were fetched (for TTL-based cache invalidation) */
  providerModelsFetchedAt: Partial<Record<ProviderKey, number>>;
  agents: AgentInfo[];
  maxTokens: number;
  thinkingBudgetTokens: number;
  reasoningEffort: ReasoningEffort;
  cacheControl: boolean;
  activeModelConfigWorkspaceId: string;
  featureFlags: FeatureFlags;
  neondbApiKey: string | undefined;
  neondbProjectId: string | undefined;
  theme: ThemeMode;
  /** Global settings dialog open state (driven by keyboard shortcut or button). */
  settingsDialogOpen: boolean;
  isHydrated: boolean;
  isLoading: boolean;
  isLoaded: boolean;
  isLoadingProviderModels: Set<string>;
  providerModelsError: string | null;
  error: string | null;

  // Actions
  setAgentModel: (role: AgentRole, model: ModelKey) => void;
  setAgentSelectedModel: (role: AgentRole, modelId: string | undefined) => void;
  setAllAgentModels: (model: ModelKey) => void;
  setAgentCustomEndpoint: (
    role: AgentRole,
    endpoint: Partial<AgentCustomEndpoint> | undefined
  ) => void;
  setProviderSetting: (
    provider: ProviderKey,
    settingKey: string,
    value: string | undefined
  ) => void;
  setMaxTokens: (tokens: number) => void;
  setThinkingBudgetTokens: (tokens: number) => void;
  setReasoningEffort: (effort: ReasoningEffort) => void;
  setCacheControl: (enabled: boolean) => void;
  setFeatureFlag: <K extends keyof FeatureFlags>(flag: K, value: FeatureFlags[K]) => void;
  setNeondbApiKey: (apiKey: string | undefined) => void;
  setNeondbProjectId: (projectId: string | undefined) => void;
  setTheme: (theme: ThemeMode) => void;
  openSettingsDialog: () => void;
  closeSettingsDialog: () => void;
  toggleSettingsDialog: () => void;
  loadModelConfig: () => Promise<void>;
  loadProviderModels: (provider?: ProviderKey, force?: boolean, region?: string) => Promise<void>;
  getAgentModelConfig: (role: AgentRole) => ModelConfig | null;
  getProviderSettings: (provider: ProviderKey) => Record<string, string>;
  getModelsForProvider: (provider: ProviderKey) => ProviderModel[];
}

type WorkspaceModelConfigStoreFields = Pick<
  SettingsStore,
  | "agentModels"
  | "agentSelectedModels"
  | "agentCustomEndpoints"
  | "maxTokens"
  | "thinkingBudgetTokens"
  | "reasoningEffort"
  | "cacheControl"
>;

const createWorkspaceModelConfigStoreFields = (): WorkspaceModelConfigStoreFields => {
  const defaults = defaultWorkspaceModelConfigState();
  return {
    agentModels: { ...defaults.agentModels },
    agentSelectedModels: { ...defaults.agentSelectedModels },
    agentCustomEndpoints: {},
    maxTokens: defaults.maxTokens,
    thinkingBudgetTokens: defaults.thinkingBudgetTokens,
    reasoningEffort: defaults.reasoningEffort,
    cacheControl: defaults.cacheControl,
  };
};

const selectWorkspaceModelConfigState = (state: SettingsStore): WorkspaceModelConfigState => ({
  agentModels: state.agentModels,
  agentSelectedModels: state.agentSelectedModels,
  maxTokens: state.maxTokens,
  thinkingBudgetTokens: state.thinkingBudgetTokens,
  reasoningEffort: state.reasoningEffort,
  cacheControl: state.cacheControl,
});

const workspaceModelConfigSyncKey = (state: SettingsStore, workspaceId: string): string =>
  JSON.stringify({
    workspaceId,
    payload: modelConfigSyncPayload(selectWorkspaceModelConfigState(state)),
  });

let cachedModelSettingsInput: {
  maxTokens: number;
  thinkingBudgetTokens: number;
  reasoningEffort: ReasoningEffort;
  cacheControl: boolean;
} | null = null;
let cachedModelSettings: ModelSettings | null = null;

function selectStableModelSettings(state: SettingsStore): ModelSettings {
  const input = {
    maxTokens: state.maxTokens,
    thinkingBudgetTokens: state.thinkingBudgetTokens,
    reasoningEffort: state.reasoningEffort,
    cacheControl: state.cacheControl,
  };

  if (
    cachedModelSettingsInput &&
    cachedModelSettings &&
    cachedModelSettingsInput.maxTokens === input.maxTokens &&
    cachedModelSettingsInput.thinkingBudgetTokens === input.thinkingBudgetTokens &&
    cachedModelSettingsInput.reasoningEffort === input.reasoningEffort &&
    cachedModelSettingsInput.cacheControl === input.cacheControl
  ) {
    return cachedModelSettings;
  }

  cachedModelSettingsInput = input;
  cachedModelSettings = normalizeModelSettings(input);
  return cachedModelSettings;
}

// ============================================================================
// Store Implementation
// ============================================================================

export const useAgentConfig = create<SettingsStore>()(
  persist(
    (set, get) => ({
      ...createWorkspaceModelConfigStoreFields(),
      providerSettings: {},
      availableModels: [],
      providerModels: [],
      providerModelsFetchedAt: {},
      agents: [],
      activeModelConfigWorkspaceId: useWorkspace.getState().activeWorkspaceId,
      featureFlags: { ...DEFAULT_FEATURE_FLAGS },
      neondbApiKey: undefined,
      neondbProjectId: undefined,
      theme: "slate" as ThemeMode,
      settingsDialogOpen: false,
      isHydrated: false,
      isLoading: false,
      isLoaded: false,
      isLoadingProviderModels: new Set<string>(),
      providerModelsError: null,
      error: null,

      /**
       * Set model for a specific agent role.
       * Requires the provider to be marked available by the backend config.
       */
      setAgentModel: (role: AgentRole, model: ModelKey) => {
        const { availableModels, agentModels, agentSelectedModels } = get();
        const modelConfig = availableModels.find((m) => m.key === model);

        // Selection is allowed only when the backend marks the provider available.
        if (!modelConfig || modelConfig.available) {
          const { [role]: _, ...restSelectedModels } = agentSelectedModels;
          set({
            agentModels: { ...agentModels, [role]: model },
            agentSelectedModels: restSelectedModels,
            error: null,
          });
        } else {
          set({
            error: `${modelConfig.displayName} is not available (credentials not configured)`,
          });
        }
      },

      /**
       * Set specific model ID for an agent (within the selected provider).
       * Clears custom endpoint settings (mutually exclusive).
       */
      setAgentSelectedModel: (role: AgentRole, modelId: string | undefined) => {
        const { agentSelectedModels, agentModels, providerModels, agentCustomEndpoints } = get();

        if (modelId) {
          const selectedProvider = agentModels[role];
          const modelsForProvider = providerModels.filter((m) => m.provider === selectedProvider);
          const modelExists = modelsForProvider.some((m) => m.id === modelId);

          if (!modelExists && modelsForProvider.length > 0) {
            console.warn(
              `[setAgentSelectedModel] Model "${modelId}" not found in provider "${selectedProvider}".`
            );
            set({
              error: `Model "${modelId}" is not available for provider "${selectedProvider}"`,
            });
            return;
          }

          const { [role]: _customEndpoint, ...restCustomEndpoints } = agentCustomEndpoints;

          set({
            agentSelectedModels: { ...agentSelectedModels, [role]: modelId },
            agentCustomEndpoints: restCustomEndpoints,
            error: null,
          });
        } else {
          const { [role]: _, ...rest } = agentSelectedModels;
          set({ agentSelectedModels: rest, error: null });
        }
      },

      /**
       * Set all agents to the same model.
       */
      setAllAgentModels: (model: ModelKey) => {
        const { availableModels, agentModels } = get();
        const modelConfig = availableModels.find((m) => m.key === model);

        if (!modelConfig || modelConfig.available) {
          const updated: AgentModelSelection = {};
          for (const role of Object.keys(agentModels)) {
            updated[role] = model;
          }
          set({
            agentModels: updated,
            agentSelectedModels: {},
            error: null,
          });
        } else {
          set({
            error: `${modelConfig.displayName} is not available (credentials not configured)`,
          });
        }
      },

      /**
       * Set custom endpoint settings for an agent.
       * Clears selected model ID when an endpoint is configured.
       */
      setAgentCustomEndpoint: (
        role: AgentRole,
        endpoint: Partial<AgentCustomEndpoint> | undefined
      ) => {
        const { agentCustomEndpoints, agentSelectedModels } = get();
        const normalized = normalizeCustomEndpoint(endpoint);

        if (normalized) {
          const { [role]: _selectedModel, ...restSelectedModels } = agentSelectedModels;
          set({
            agentCustomEndpoints: { ...agentCustomEndpoints, [role]: normalized },
            agentSelectedModels: restSelectedModels,
          });
        } else {
          const { [role]: _customEndpoint, ...restCustomEndpoints } = agentCustomEndpoints;
          set({ agentCustomEndpoints: restCustomEndpoints });
        }
      },

      /**
       * Set a provider setting value.
       * Invalidates only that provider's cached model list and selected model IDs.
       */
      setProviderSetting: (
        provider: ProviderKey,
        settingKey: string,
        value: string | undefined
      ) => {
        const {
          providerModelsFetchedAt,
          agentModels,
          agentSelectedModels,
          providerModels,
          providerSettings,
        } = get();
        const { [provider]: _timestamp, ...restTimestamps } = providerModelsFetchedAt;

        const clearedSelectedModels = { ...agentSelectedModels };
        for (const role of Object.keys(agentModels)) {
          if (agentModels[role] === provider) {
            delete clearedSelectedModels[role];
          }
        }

        const remainingModels = providerModels.filter((model) => model.provider !== provider);
        const nextProviderSettings = { ...providerSettings };
        const currentProviderSettings = { ...(nextProviderSettings[provider] || {}) };

        if (value) {
          currentProviderSettings[settingKey] = value;
          nextProviderSettings[provider] = currentProviderSettings;
        } else {
          delete currentProviderSettings[settingKey];
          if (Object.keys(currentProviderSettings).length === 0) {
            delete nextProviderSettings[provider];
          } else {
            nextProviderSettings[provider] = currentProviderSettings;
          }
        }

        set({
          providerSettings: nextProviderSettings,
          providerModelsFetchedAt: restTimestamps,
          agentSelectedModels: clearedSelectedModels,
          providerModels: remainingModels,
        });
      },

      /**
       * Get persisted settings for a provider.
       */
      getProviderSettings: (provider: ProviderKey) => {
        return get().providerSettings[provider] || {};
      },

      /**
       * Set max response tokens.
       */
      setMaxTokens: (tokens: number) => {
        set((state) => ({
          maxTokens: normalizeModelSettings({ ...selectModelSettings(state), maxTokens: tokens })
            .maxTokens,
        }));
      },

      /**
       * Set thinking budget tokens.
       */
      setThinkingBudgetTokens: (tokens: number) => {
        set((state) => ({
          thinkingBudgetTokens: normalizeModelSettings({
            ...selectModelSettings(state),
            thinkingBudgetTokens: tokens,
          }).thinkingBudgetTokens,
        }));
      },

      /**
       * Set Anthropic reasoning effort.
       */
      setReasoningEffort: (effort: ReasoningEffort) => {
        if (isReasoningEffort(effort)) {
          set({ reasoningEffort: effort });
        }
      },

      /**
       * Set provider prompt-caching preference where supported.
       */
      setCacheControl: (enabled: boolean) => {
        set({ cacheControl: enabled });
      },

      /**
       * Set a feature flag value.
       *
       * Pure state transition only — backend sync for `piiScramble` is handled
       * by a module-level subscriber (see `_unsubscribePiiSync` below).
       */
      setFeatureFlag: <K extends keyof FeatureFlags>(flag: K, value: FeatureFlags[K]) => {
        const { featureFlags } = get();
        set({ featureFlags: { ...featureFlags, [flag]: value } });
      },

      /**
       * Set NeonDB API key.
       */
      setNeondbApiKey: (apiKey: string | undefined) => {
        set({ neondbApiKey: apiKey || undefined });
      },

      /**
       * Set NeonDB project ID.
       */
      setNeondbProjectId: (projectId: string | undefined) => {
        set({ neondbProjectId: projectId || undefined });
      },

      setTheme: (theme: ThemeMode) => {
        set({ theme });
      },

      openSettingsDialog: () => set({ settingsDialogOpen: true }),
      closeSettingsDialog: () => set({ settingsDialogOpen: false }),
      toggleSettingsDialog: () => set((s) => ({ settingsDialogOpen: !s.settingsDialogOpen })),

      /**
       * Load model configuration from backend.
       * Failure stays explicit — no stale built-in model fallback.
       */
      loadModelConfig: async () => {
        const { isLoading } = get();
        if (isLoading) return;
        const workspaceId = useWorkspace.getState().activeWorkspaceId;

        set({ activeModelConfigWorkspaceId: workspaceId, isLoading: true, error: null });

        try {
          const { apiFetch } = await import("~/lib/api/client");
          const result = await apiFetch<ModelConfigResponse>("/model-config");
          if (result._tag === "Err") {
            throw new Error(result.error.message);
          }
          const data = result.value;
          const fallbackProvider =
            data.models.find((m) => m.available)?.key ?? data.models[0]?.key ?? "anthropic";
          const workspaceModelConfig = deriveWorkspaceModelConfigState(data, fallbackProvider);

          if (workspaceId !== useWorkspace.getState().activeWorkspaceId) {
            return;
          }

          _isApplyingBackendModelConfig = true;
          try {
            set({
              availableModels: data.models,
              agents: data.agents,
              ...workspaceModelConfig,
              activeModelConfigWorkspaceId: workspaceId,
              isLoaded: true,
              isLoading: false,
              error: null,
            });
          } finally {
            _isApplyingBackendModelConfig = false;
          }
        } catch (_error) {
          if (workspaceId !== useWorkspace.getState().activeWorkspaceId) {
            return;
          }
          // The harness does not serve /model-config (settings module not wired).
          // Degrade gracefully to "loaded, empty" so dependent screens (e.g. the
          // New Evaluation form) render instead of blocking on a skeleton.
          set({
            availableModels: [],
            agents: [],
            isLoading: false,
            isLoaded: true,
            error: null,
          });
        }
      },

      /**
       * Load provider models with pricing from backend.
       * Uses TTL-based caching.
       */
      loadProviderModels: async (
        provider?: ProviderKey,
        force?: boolean,
        regionOverride?: string
        // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: store initialization
      ) => {
        const {
          isLoadingProviderModels,
          providerModels: existingModels,
          providerModelsFetchedAt,
        } = get();
        const loadKey = provider ?? "__all__";
        if (isLoadingProviderModels.has(loadKey)) return;

        if (!force && provider) {
          const fetchedAt = providerModelsFetchedAt[provider];
          if (fetchedAt) {
            const age = Date.now() - fetchedAt;
            if (age < PROVIDER_MODELS_TTL_MS) {
              const hasModels = existingModels.some((m) => m.provider === provider);
              if (hasModels) return;
            }
          }
        }

        set({
          isLoadingProviderModels: new Set([...get().isLoadingProviderModels, loadKey]),
          providerModelsError: null,
        });

        try {
          const { providerSettings, availableModels } = get();
          const body: Record<string, unknown> = {};

          if (provider) {
            body.provider = provider;
            const providerConfig = availableModels.find((model) => model.key === provider);
            const mergedSettings: Record<string, string> = {
              ...(providerSettings[provider] || {}),
            };
            if (providerConfig) {
              for (const setting of providerConfig.settings) {
                if (
                  mergedSettings[setting.key] == null &&
                  setting.defaultValue != null &&
                  setting.defaultValue !== ""
                ) {
                  mergedSettings[setting.key] = setting.defaultValue;
                }
              }
            }
            if (regionOverride) {
              mergedSettings.region = regionOverride;
            }
            if (Object.keys(mergedSettings).length > 0) {
              body.settings = mergedSettings;
            }
          }

          const { apiFetch } = await import("~/lib/api/client");
          const result = await apiFetch<{ success: boolean; models: ProviderModel[] }>(
            "/list-provider-models",
            {
              method: "POST",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify(body),
            }
          );

          if (result._tag === "Err") {
            throw new Error(result.error.message);
          }

          const data = result.value;

          if (data.success) {
            let updatedModels: ProviderModel[];
            const now = Date.now();

            if (provider) {
              const otherModels = existingModels.filter((m) => m.provider !== provider);
              updatedModels = [...otherModels, ...data.models];
              const updated1 = new Set(get().isLoadingProviderModels);
              updated1.delete(loadKey);
              set({
                providerModels: updatedModels,
                providerModelsFetchedAt: { ...providerModelsFetchedAt, [provider]: now },
                isLoadingProviderModels: updated1,
                providerModelsError: null,
              });
            } else {
              updatedModels = data.models;
              const providers = [...new Set(data.models.map((m: ProviderModel) => m.provider))];
              const newTimestamps: Partial<Record<ProviderKey, number>> = {};
              for (const p of providers) {
                newTimestamps[p] = now;
              }
              const updated2 = new Set(get().isLoadingProviderModels);
              updated2.delete(loadKey);
              set({
                providerModels: updatedModels,
                providerModelsFetchedAt: newTimestamps,
                isLoadingProviderModels: updated2,
                providerModelsError: null,
              });
            }
          } else {
            const errorMsg =
              ((data as Record<string, unknown>).error as string) ||
              "Provider models response was not successful";
            const updated3 = new Set(get().isLoadingProviderModels);
            updated3.delete(loadKey);
            set({ isLoadingProviderModels: updated3, providerModelsError: errorMsg });
          }
        } catch (error) {
          const updated4 = new Set(get().isLoadingProviderModels);
          updated4.delete(loadKey);
          set({
            isLoadingProviderModels: updated4,
            providerModelsError:
              error instanceof Error ? error.message : "Failed to load provider models",
          });
        }
      },

      /**
       * Get the full config for an agent's model.
       */
      getAgentModelConfig: (role: AgentRole) => {
        const { agentModels, availableModels } = get();
        return availableModels.find((m) => m.key === agentModels[role]) || null;
      },

      /**
       * Get available models for a specific provider.
       */
      getModelsForProvider: (provider: ProviderKey) => {
        return get().providerModels.filter((m) => m.provider === provider);
      },
    }),
    {
      name: "studio-settings",
      version: 29,
      storage: createJSONStorage(() => zustandSettingsStorage),
      skipHydration: true,
      partialize: (state) =>
        sanitizeSettingsPersistence(
          {
            providerSettings: state.providerSettings,
            featureFlags: state.featureFlags,
            neondbProjectId: state.neondbProjectId,
            theme: state.theme,
          },
          state.availableModels
        ),
      // Migrations
      migrate: (persistedState: unknown, version: number) => {
        const migrateModelKey = (key: string): ModelKey => {
          const mapping: Record<string, ModelKey> = {
            claude: "anthropic",
            gpt5: "openai",
            gpt4: "openai",
            "bedrock-claude": "bedrock",
            "bedrock-nova": "bedrock",
            anthropic: "anthropic",
            openai: "openai",
            cerebras: "cerebras",
            bedrock: "bedrock",
            groq: "groq",
            deepinfra: "deepinfra",
            google: "google",
            azure: "azure",
          };
          return mapping[key] || "anthropic";
        };

        const migrateAgentModels = (models: Record<string, string>): AgentModelSelection => {
          const result: AgentModelSelection = {};
          for (const [role, key] of Object.entries(models)) {
            result[role] = migrateModelKey(key || "anthropic");
          }
          // Ensure at least one role exists
          if (Object.keys(result).length === 0) {
            result.default = "anthropic";
          }
          return result;
        };

        // v1 (legacy version) → v15: full structure migration
        if (version === 1) {
          const oldState = persistedState as {
            agentModels?: Record<string, string>;
            selectedModel?: string;
            agentSelectedModels?: AgentSelectedModels;
            providerApiKeys?: LegacyProviderApiKeys;
            featureFlags?: Record<string, boolean>;
          };

          // Handle both old single-model and per-agent formats
          let agentModels: AgentModelSelection;
          if (oldState.agentModels) {
            agentModels = migrateAgentModels(oldState.agentModels);
          } else {
            const model = migrateModelKey(oldState.selectedModel || "anthropic");
            agentModels = { default: model };
          }

          return stripSettingsPersistence(
            upgradeCustomEndpointState({
              agentModels,
              agentSelectedModels: oldState.agentSelectedModels || {},
              providerSettings: Object.fromEntries(
                Object.entries(oldState.providerApiKeys || {}).map(([provider, value]) => [
                  provider,
                  { apiKey: value },
                ])
              ),
              featureFlags: {
                fullCsvExport: oldState.featureFlags?.fullCsvExport ?? true,
                showLatencyPanel: oldState.featureFlags?.showLatencyPanel ?? false,
              },
            })
          );
        }
        if (version === 2) {
          const oldState = persistedState as { agentModels: Record<string, string> };
          return stripSettingsPersistence(
            upgradeCustomEndpointState({
              agentModels: migrateAgentModels(oldState.agentModels),
              agentSelectedModels: {},
              providerSettings: {},
              featureFlags: { ...DEFAULT_FEATURE_FLAGS },
            })
          );
        }
        if (version >= 3 && version <= 9) {
          const oldState = persistedState as {
            agentModels: Record<string, string>;
            agentCustomBaseUrls?: Partial<Record<AgentRole, string>>;
            agentCustomModelNames?: Partial<Record<AgentRole, string>>;
            providerApiKeys?: LegacyProviderApiKeys;
            awsRegion?: string;
            featureFlags?: FeatureFlags;
          };
          return stripSettingsPersistence(
            upgradeCustomEndpointState({
              agentModels: migrateAgentModels(oldState.agentModels),
              agentSelectedModels: {},
              agentCustomBaseUrls: oldState.agentCustomBaseUrls || {},
              agentCustomModelNames: oldState.agentCustomModelNames || {},
              providerSettings: {
                ...Object.fromEntries(
                  Object.entries(oldState.providerApiKeys || {}).map(([provider, value]) => [
                    provider,
                    { apiKey: value },
                  ])
                ),
                ...(oldState.awsRegion ? { bedrock: { region: oldState.awsRegion } } : {}),
              },
              featureFlags: oldState.featureFlags || { ...DEFAULT_FEATURE_FLAGS },
            })
          );
        }
        if (version === 10 || version === 11) {
          const oldState = persistedState as {
            agentModels: AgentModelSelection;
            agentSelectedModels?: AgentSelectedModels;
            agentCustomBaseUrls: Partial<Record<AgentRole, string>>;
            agentCustomModelNames: Partial<Record<AgentRole, string>>;
            agentCustomApiKeys?: Partial<Record<AgentRole, string>>;
            providerApiKeys: LegacyProviderApiKeys;
            awsRegion: string;
            featureFlags: FeatureFlags;
          };
          return stripSettingsPersistence(
            upgradeCustomEndpointState({
              agentModels: oldState.agentModels,
              agentSelectedModels: oldState.agentSelectedModels || {},
              agentCustomBaseUrls: oldState.agentCustomBaseUrls,
              agentCustomModelNames: oldState.agentCustomModelNames,
              agentCustomApiKeys: oldState.agentCustomApiKeys || {},
              providerSettings: {
                ...Object.fromEntries(
                  Object.entries(oldState.providerApiKeys).map(([provider, value]) => [
                    provider,
                    { apiKey: value },
                  ])
                ),
                ...(oldState.awsRegion ? { bedrock: { region: oldState.awsRegion } } : {}),
              },
              featureFlags: oldState.featureFlags,
            })
          );
        }
        if (version === 12) {
          const oldState = persistedState as {
            agentModels: AgentModelSelection;
            agentSelectedModels: AgentSelectedModels;
            agentCustomBaseUrls: Partial<Record<AgentRole, string>>;
            agentCustomModelNames: Partial<Record<AgentRole, string>>;
            agentCustomApiKeys: Partial<Record<AgentRole, string>>;
            providerApiKeys: LegacyProviderApiKeys;
            awsRegion: string;
            featureFlags: FeatureFlags;
          };
          return stripSettingsPersistence(
            upgradeCustomEndpointState({
              ...oldState,
              providerSettings: {
                ...Object.fromEntries(
                  Object.entries(oldState.providerApiKeys).map(([provider, value]) => [
                    provider,
                    { apiKey: value },
                  ])
                ),
                ...(oldState.awsRegion ? { bedrock: { region: oldState.awsRegion } } : {}),
              },
              featureFlags: { ...oldState.featureFlags, fullCsvExport: true },
            })
          );
        }
        if (version === 13 || version === 14) {
          const oldState = persistedState as {
            agentModels: AgentModelSelection;
            agentSelectedModels: AgentSelectedModels;
            agentCustomBaseUrls: Partial<Record<AgentRole, string>>;
            agentCustomModelNames: Partial<Record<AgentRole, string>>;
            agentCustomApiKeys: Partial<Record<AgentRole, string>>;
            providerApiKeys: LegacyProviderApiKeys;
            awsRegion: string;
            featureFlags: FeatureFlags;
          };
          return stripSettingsPersistence(
            upgradeCustomEndpointState({
              ...oldState,
              providerSettings: {
                ...Object.fromEntries(
                  Object.entries(oldState.providerApiKeys).map(([provider, value]) => [
                    provider,
                    { apiKey: value },
                  ])
                ),
                ...(oldState.awsRegion ? { bedrock: { region: oldState.awsRegion } } : {}),
              },
              thinkingBudgetTokens: 10000,
              featureFlags: { ...oldState.featureFlags, showLatencyPanel: false },
            })
          );
        }
        if (version === 15) {
          const oldState = persistedState as Record<string, unknown> & { awsRegion?: string };
          return stripSettingsPersistence(
            upgradeCustomEndpointState({
              ...oldState,
              providerSettings: oldState.awsRegion
                ? { bedrock: { region: oldState.awsRegion } }
                : {},
              thinkingBudgetTokens: 10000,
            })
          );
        }
        if (version === 16) {
          const oldState = persistedState as Record<string, unknown> & { awsRegion?: string };
          const oldFlags = (oldState.featureFlags || {}) as Record<string, unknown>;
          return stripSettingsPersistence(
            upgradeCustomEndpointState({
              ...oldState,
              providerSettings: oldState.awsRegion
                ? { bedrock: { region: oldState.awsRegion } }
                : {},
              featureFlags: { ...oldFlags, piiScramble: false },
            })
          );
        }
        if (version === 17) {
          const oldState = persistedState as Record<string, unknown> & { awsRegion?: string };
          return stripSettingsPersistence(
            upgradeCustomEndpointState({
              ...oldState,
              providerSettings: oldState.awsRegion
                ? { bedrock: { region: oldState.awsRegion } }
                : {},
              neondbApiKey: undefined,
              neondbProjectId: undefined,
            })
          );
        }
        if (version === 18) {
          const oldState = persistedState as Record<string, unknown> & { awsRegion?: string };
          return stripSettingsPersistence(
            upgradeCustomEndpointState({
              ...oldState,
              providerSettings: oldState.awsRegion
                ? { bedrock: { region: oldState.awsRegion } }
                : {},
              theme: "system",
            })
          );
        }
        if (version === 19) {
          const oldState = persistedState as Record<string, unknown> & { awsRegion?: string };
          return stripSettingsPersistence(
            upgradeCustomEndpointState({
              ...oldState,
              providerSettings: oldState.awsRegion
                ? { bedrock: { region: oldState.awsRegion } }
                : {},
              theme: "slate",
            })
          );
        }
        if (version === 20) {
          const oldState = persistedState as {
            awsRegion?: string;
            providerApiKeys?: LegacyProviderApiKeys;
            providerRegions?: Partial<Record<ProviderKey, string>>;
            providerSettings?: ProviderSettings;
          } & Record<string, unknown>;
          return stripSettingsPersistence(
            upgradeCustomEndpointState({
              ...oldState,
              providerSettings: oldState.providerSettings || {
                ...Object.fromEntries(
                  Object.entries(oldState.providerApiKeys || {}).map(([provider, value]) => [
                    provider,
                    { apiKey: value },
                  ])
                ),
                ...Object.fromEntries(
                  Object.entries(oldState.providerRegions || {}).map(([provider, region]) => [
                    provider,
                    { region },
                  ])
                ),
                ...(oldState.awsRegion ? { bedrock: { region: oldState.awsRegion } } : {}),
              },
            })
          );
        }
        if (version === 23) {
          return stripSettingsPersistence(
            upgradeEndpointTargetModelField(
              upgradeCustomEndpointState(persistedState as Record<string, unknown>)
            )
          );
        }
        const upgraded = upgradeCustomEndpointState(persistedState as Record<string, unknown>) as {
          maxTokens?: unknown;
          thinkingBudgetTokens?: unknown;
          reasoningEffort?: unknown;
          cacheControl?: unknown;
        } & Record<string, unknown>;
        const migratedSettings = normalizeModelSettings({
          maxTokens: typeof upgraded.maxTokens === "number" ? upgraded.maxTokens : undefined,
          thinkingBudgetTokens:
            typeof upgraded.thinkingBudgetTokens === "number" &&
            upgraded.thinkingBudgetTokens !== 10000
              ? upgraded.thinkingBudgetTokens
              : 0,
          reasoningEffort: isReasoningEffort(upgraded.reasoningEffort)
            ? upgraded.reasoningEffort
            : undefined,
          cacheControl:
            typeof upgraded.cacheControl === "boolean" ? upgraded.cacheControl : undefined,
        });
        return stripSettingsPersistence({
          ...upgraded,
          ...migratedSettings,
        });
      },
      onRehydrateStorage: () => {
        return (state, error) => {
          if (error) {
            console.error("[SettingsStore] Rehydration error:", error);
          }
          if (state) {
            state.isHydrated = true;
          }
        };
      },
    }
  )
);

// =============================================================================
// Controlled IO boundary: sync piiScramble to backend
//
// Watches for piiScramble changes and PUTs to the backend settings API.
// Local state is authoritative; the backend is a durable backup.
// =============================================================================

let _prevPiiScramble = useAgentConfig.getState().featureFlags.piiScramble;
const _unsubscribePiiSync = useAgentConfig.subscribe((state) => {
  const next = state.featureFlags.piiScramble;
  if (next !== _prevPiiScramble) {
    _prevPiiScramble = next;
    apiFetch("/settings/pii_scrambling_enabled", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ value: String(next) }),
    }).catch((err: unknown) => {
      console.warn("[settings] failed to sync pii_scrambling_enabled:", err);
    });
  }
});

// =============================================================================
// Controlled IO boundary: sync workspace-scoped model config to backend
//
// The backend is authoritative across reloads and workspace switches. The store
// only keeps the active workspace slice in memory to prevent cross-tenant leaks.
// =============================================================================

let _prevModelConfigSyncKey = workspaceModelConfigSyncKey(
  useAgentConfig.getState(),
  useWorkspace.getState().activeWorkspaceId
);

const _unsubscribeModelConfigSync = useAgentConfig.subscribe((state) => {
  const workspaceId = useWorkspace.getState().activeWorkspaceId;
  const nextKey = workspaceModelConfigSyncKey(state, workspaceId);
  const shouldSkipSync =
    _isApplyingBackendModelConfig ||
    !state.isLoaded ||
    state.activeModelConfigWorkspaceId !== workspaceId ||
    nextKey === _prevModelConfigSyncKey;

  _prevModelConfigSyncKey = nextKey;

  if (shouldSkipSync) {
    return;
  }

  apiFetch("/model-config", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(modelConfigSyncPayload(selectWorkspaceModelConfigState(state))),
  }).catch((err: unknown) => {
    console.warn("[settings] failed to sync workspace model config:", err);
  });
});

function rehydrateSettingsForWorkspace(workspaceId: string): void {
  Promise.resolve(useAgentConfig.persist.rehydrate()).catch((error: unknown) => {
    console.warn(`[settings] failed to rehydrate settings for workspace ${workspaceId}:`, error);
  });
}

rehydrateSettingsForWorkspace(useWorkspace.getState().activeWorkspaceId);

// Reset active model config immediately on workspace switch, then rehydrate it
// from the backend for the new tenant.
let _prevWorkspaceModelConfigId = useWorkspace.getState().activeWorkspaceId;
const _unsubscribeWorkspaceModelConfigSync = useWorkspace.subscribe((state) => {
  const nextWorkspaceId = state.activeWorkspaceId;
  if (nextWorkspaceId === _prevWorkspaceModelConfigId) {
    return;
  }

  _prevWorkspaceModelConfigId = nextWorkspaceId;
  _isApplyingBackendModelConfig = true;
  try {
    useAgentConfig.setState({
      ...createWorkspaceModelConfigStoreFields(),
      availableModels: [],
      agents: [],
      activeModelConfigWorkspaceId: nextWorkspaceId,
      isLoaded: false,
      isLoading: false,
      error: null,
    });
  } finally {
    _isApplyingBackendModelConfig = false;
  }
  _prevModelConfigSyncKey = workspaceModelConfigSyncKey(useAgentConfig.getState(), nextWorkspaceId);
  rehydrateSettingsForWorkspace(nextWorkspaceId);

  useAgentConfig
    .getState()
    .loadModelConfig()
    .catch((err: unknown) => {
      console.warn("[settings] failed to load workspace model config:", err);
    });
});

// Clean up during Vite HMR to prevent duplicate listeners.
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    _unsubscribePiiSync();
    _unsubscribeModelConfigSync();
    _unsubscribeWorkspaceModelConfigSync();
  });
}

// =============================================================================
// Selectors for granular subscriptions
// =============================================================================

// State selectors
export const selectAgentModels = (state: SettingsStore) => state.agentModels;
export const selectAgentSelectedModels = (state: SettingsStore) => state.agentSelectedModels;
export const selectAgentCustomEndpoints = (state: SettingsStore) => state.agentCustomEndpoints;
export const selectProviderSettings = (state: SettingsStore) => state.providerSettings;
export const selectAvailableModels = (state: SettingsStore) => state.availableModels;
export const selectProviderModels = (state: SettingsStore) => state.providerModels;
export const selectAgents = (state: SettingsStore) => state.agents;
export const selectMaxTokens = (state: SettingsStore) => state.maxTokens;
export const selectThinkingBudgetTokens = (state: SettingsStore) => state.thinkingBudgetTokens;
export const selectReasoningEffort = (state: SettingsStore) => state.reasoningEffort;
export const selectCacheControl = (state: SettingsStore) => state.cacheControl;
export const selectActiveModelConfigWorkspaceId = (state: SettingsStore) =>
  state.activeModelConfigWorkspaceId;
export const selectModelSettings = selectStableModelSettings;
export const selectFeatureFlags = (state: SettingsStore) => state.featureFlags;
export const selectIsHydrated = (state: SettingsStore) => state.isHydrated;
export const selectIsLoading = (state: SettingsStore) => state.isLoading;
export const selectIsLoaded = (state: SettingsStore) => state.isLoaded;
export const selectIsLoadingProviderModels = (state: SettingsStore) =>
  state.isLoadingProviderModels;
export const selectProviderModelsError = (state: SettingsStore) => state.providerModelsError;
export const selectSettingsError = (state: SettingsStore) => state.error;

// Feature flag selectors
export const selectShowLatencyPanel = (state: SettingsStore) => state.featureFlags.showLatencyPanel;
export const selectFullCsvExport = (state: SettingsStore) => state.featureFlags.fullCsvExport;
export const selectPiiScramble = (state: SettingsStore) => state.featureFlags.piiScramble;
export const selectNeondbApiKey = (state: SettingsStore) => state.neondbApiKey;
export const selectNeondbProjectId = (state: SettingsStore) => state.neondbProjectId;
export const selectTheme = (state: SettingsStore) => state.theme;
export const selectSettingsDialogOpen = (state: SettingsStore) => state.settingsDialogOpen;

// Model selector (generic — pass role name)
export const selectAgentModelForRole =
  (role: string) =>
  (state: SettingsStore): ModelKey | undefined =>
    state.agentModels[role];

// Action selectors (stable references)
export const selectSetAgentModel = (state: SettingsStore) => state.setAgentModel;
export const selectSetAgentSelectedModel = (state: SettingsStore) => state.setAgentSelectedModel;
export const selectSetAllAgentModels = (state: SettingsStore) => state.setAllAgentModels;
export const selectSetAgentCustomEndpoint = (state: SettingsStore) => state.setAgentCustomEndpoint;
export const selectSetProviderSetting = (state: SettingsStore) => state.setProviderSetting;
export const selectSetMaxTokens = (state: SettingsStore) => state.setMaxTokens;
export const selectSetThinkingBudgetTokens = (state: SettingsStore) =>
  state.setThinkingBudgetTokens;
export const selectSetReasoningEffort = (state: SettingsStore) => state.setReasoningEffort;
export const selectSetCacheControl = (state: SettingsStore) => state.setCacheControl;
export const selectSetFeatureFlag = (state: SettingsStore) => state.setFeatureFlag;
export const selectSetNeondbApiKey = (state: SettingsStore) => state.setNeondbApiKey;
export const selectSetNeondbProjectId = (state: SettingsStore) => state.setNeondbProjectId;
export const selectSetTheme = (state: SettingsStore) => state.setTheme;
export const selectOpenSettingsDialog = (state: SettingsStore) => state.openSettingsDialog;
export const selectCloseSettingsDialog = (state: SettingsStore) => state.closeSettingsDialog;
export const selectToggleSettingsDialog = (state: SettingsStore) => state.toggleSettingsDialog;
export const selectLoadModelConfig = (state: SettingsStore) => state.loadModelConfig;
export const selectLoadProviderModels = (state: SettingsStore) => state.loadProviderModels;
export const selectGetAgentModelConfig = (state: SettingsStore) => state.getAgentModelConfig;
export const selectGetModelsForProvider = (state: SettingsStore) => state.getModelsForProvider;
export const selectGetProviderSettings = (state: SettingsStore) => state.getProviderSettings;

// ============================================================================
// Hooks
// ============================================================================

/**
 * Hook for accessing settings with shallow equality comparison.
 */
export function useSettings() {
  return useAgentConfig(
    useShallow((state) => ({
      agentModels: state.agentModels,
      agentSelectedModels: state.agentSelectedModels,
      agentCustomEndpoints: state.agentCustomEndpoints,
      providerSettings: state.providerSettings,
      availableModels: state.availableModels,
      providerModels: state.providerModels,
      agents: state.agents,
      maxTokens: state.maxTokens,
      thinkingBudgetTokens: state.thinkingBudgetTokens,
      reasoningEffort: state.reasoningEffort,
      cacheControl: state.cacheControl,
      activeModelConfigWorkspaceId: state.activeModelConfigWorkspaceId,
      featureFlags: state.featureFlags,
      neondbApiKey: state.neondbApiKey,
      neondbProjectId: state.neondbProjectId,
      theme: state.theme,
      isHydrated: state.isHydrated,
      isLoading: state.isLoading,
      isLoaded: state.isLoaded,
      isLoadingProviderModels: state.isLoadingProviderModels,
      error: state.error,
      setAgentModel: state.setAgentModel,
      setAgentSelectedModel: state.setAgentSelectedModel,
      setAllAgentModels: state.setAllAgentModels,
      setAgentCustomEndpoint: state.setAgentCustomEndpoint,
      setProviderSetting: state.setProviderSetting,
      getProviderSettings: state.getProviderSettings,
      setMaxTokens: state.setMaxTokens,
      setThinkingBudgetTokens: state.setThinkingBudgetTokens,
      setReasoningEffort: state.setReasoningEffort,
      setCacheControl: state.setCacheControl,
      setFeatureFlag: state.setFeatureFlag,
      setNeondbApiKey: state.setNeondbApiKey,
      setNeondbProjectId: state.setNeondbProjectId,
      setTheme: state.setTheme,
      loadModelConfig: state.loadModelConfig,
      loadProviderModels: state.loadProviderModels,
      getAgentModelConfig: state.getAgentModelConfig,
      getModelsForProvider: state.getModelsForProvider,
    }))
  );
}

/**
 * Get a specific agent's model selection.
 */
export function useAgentModel(role: string): ProviderKey | undefined {
  return useAgentConfig((state) => state.agentModels[role]);
}

/**
 * Get a specific feature flag value.
 */
export function useFeatureFlag<K extends keyof FeatureFlags>(flag: K): FeatureFlags[K] {
  return useAgentConfig((state) => state.featureFlags[flag]);
}

/** @deprecated Use useAgentConfig directly. */
export const useSettingsStore = useAgentConfig;
