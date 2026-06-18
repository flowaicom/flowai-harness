/**
 * Pure workspace-scoped model configuration description.
 *
 * The backend stores concrete model IDs per workspace. The Studio store
 * interprets provider selection and provider-local model IDs separately for UI
 * controls, so this module owns the reversible mapping between both shapes.
 */

import {
  DEFAULT_MODEL_SETTINGS,
  type ModelSettings,
  normalizeModelSettings,
} from "./model-settings";

export type AgentModelSelection = Record<string, string>;
export type AgentSelectedModels = Partial<Record<string, string>>;

export interface WorkspaceModelConfigResponseLike {
  readonly defaultModels: AgentModelSelection;
  readonly selectedModels?: AgentModelSelection;
  readonly modelSettings?: Partial<ModelSettings> | null;
}

export interface WorkspaceModelConfigState extends ModelSettings {
  readonly agentModels: AgentModelSelection;
  readonly agentSelectedModels: AgentSelectedModels;
}

export interface WorkspaceModelConfigSyncPayload extends ModelSettings {
  readonly roleModels: AgentModelSelection;
}

const DEFAULT_AGENT_MODELS: AgentModelSelection = Object.freeze({
  default: "anthropic",
});

export function defaultWorkspaceModelConfigState(): WorkspaceModelConfigState {
  return {
    agentModels: { ...DEFAULT_AGENT_MODELS },
    agentSelectedModels: {},
    ...DEFAULT_MODEL_SETTINGS,
  };
}

export function deriveWorkspaceModelConfigState(
  response: WorkspaceModelConfigResponseLike,
  fallbackProvider = DEFAULT_AGENT_MODELS.default
): WorkspaceModelConfigState {
  const roles = new Set([
    ...Object.keys(response.defaultModels),
    ...Object.keys(response.selectedModels ?? {}),
  ]);
  const agentModels: AgentModelSelection = {};
  const agentSelectedModels: AgentSelectedModels = {};

  for (const role of roles) {
    const provider = response.defaultModels[role] || fallbackProvider;
    agentModels[role] = provider;

    const selectedModelId = providerLocalModelId(provider, response.selectedModels?.[role]);
    if (selectedModelId) {
      agentSelectedModels[role] = selectedModelId;
    }
  }

  return {
    agentModels: Object.keys(agentModels).length > 0 ? agentModels : { ...DEFAULT_AGENT_MODELS },
    agentSelectedModels,
    ...normalizeModelSettings(response.modelSettings),
  };
}

export function modelConfigSyncPayload(
  state: WorkspaceModelConfigState
): WorkspaceModelConfigSyncPayload {
  const roleModels: AgentModelSelection = {};

  for (const [role, provider] of Object.entries(state.agentModels)) {
    roleModels[role] = wireModelId(provider, state.agentSelectedModels[role]);
  }

  return {
    roleModels,
    ...normalizeModelSettings(state),
  };
}

export function stripWorkspaceModelConfigPersistence(
  state: Record<string, unknown>
): Record<string, unknown> {
  const {
    agentModels: _agentModels,
    agentSelectedModels: _agentSelectedModels,
    agentCustomBaseUrls: _agentCustomBaseUrls,
    agentCustomModelNames: _agentCustomModelNames,
    agentCustomApiKeys: _agentCustomApiKeys,
    agentCustomEndpoints: _agentCustomEndpoints,
    maxTokens: _maxTokens,
    thinkingBudgetTokens: _thinkingBudgetTokens,
    reasoningEffort: _reasoningEffort,
    cacheControl: _cacheControl,
    ...rest
  } = state;
  return rest;
}

function providerLocalModelId(
  provider: string,
  selectedModelId: string | undefined
): string | undefined {
  if (!selectedModelId || selectedModelId === provider) {
    return undefined;
  }

  const providerPrefix = `${provider}/`;
  if (selectedModelId.startsWith(providerPrefix)) {
    return selectedModelId.slice(providerPrefix.length);
  }

  return selectedModelId;
}

function wireModelId(provider: string, selectedModelId: string | undefined): string {
  if (
    !selectedModelId ||
    selectedModelId === provider ||
    selectedModelId.startsWith(`${provider}/`)
  ) {
    return selectedModelId || provider;
  }

  return `${provider}/${selectedModelId}`;
}
