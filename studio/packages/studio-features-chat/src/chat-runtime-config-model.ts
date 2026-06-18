export interface ChatAvailableModelLike {
  key: string;
  model: string;
  displayName?: string;
  endpointTransport?: string | null;
}

export interface ChatCustomEndpointLike {
  settings: Record<string, string>;
  targetModel?: string;
}

export interface ResolvedAgentEndpointLike {
  transport: string;
  settings: Record<string, string>;
  targetModel?: string;
}

export interface ResolveChatAgentOverridesParams {
  agentModels: Readonly<Record<string, string | undefined>>;
  agentSelectedModels?: Readonly<Record<string, string | undefined>>;
  availableModels: readonly ChatAvailableModelLike[];
  agentCustomEndpoints?: Readonly<Record<string, ChatCustomEndpointLike | undefined>>;
}

export type ResolveChatAgentOverridesResult =
  | {
      ok: true;
      value: {
        agentModels: Record<string, string>;
        agentEndpoints: Record<string, ResolvedAgentEndpointLike>;
      };
    }
  | { ok: false; error: string };

const normalizeEndpointSettings = (
  settings: Record<string, string> | undefined
): Record<string, string> =>
  Object.fromEntries(
    Object.entries(settings || {}).filter(
      (entry): entry is [string, string] =>
        typeof entry[1] === "string" && entry[1].trim().length > 0
    )
  );

export function resolveChatAgentOverrides(
  params: ResolveChatAgentOverridesParams
): ResolveChatAgentOverridesResult {
  const { agentModels, agentSelectedModels, availableModels, agentCustomEndpoints } = params;
  const resolvedAgentModels: Record<string, string> = {};
  const resolvedAgentEndpoints: Record<string, ResolvedAgentEndpointLike> = {};

  for (const role of Object.keys(agentModels)) {
    const providerKey = agentModels[role];
    if (!providerKey) continue;

    const selectedModel = agentSelectedModels?.[role];
    const config = availableModels.find((model) => model.key === providerKey);

    if (selectedModel) {
      resolvedAgentModels[role] = `${providerKey}/${selectedModel}`;
    } else if (config) {
      resolvedAgentModels[role] = config.model;
    }

    const customEndpoint = agentCustomEndpoints?.[role];
    const endpointSettings = normalizeEndpointSettings(customEndpoint?.settings);
    if (Object.keys(endpointSettings).length === 0) continue;

    if (!config?.endpointTransport) {
      return {
        ok: false,
        error: `Provider "${config?.displayName || providerKey}" does not declare a custom endpoint transport.`,
      };
    }

    resolvedAgentEndpoints[role] = {
      transport: config.endpointTransport,
      settings: endpointSettings,
      ...(customEndpoint?.targetModel ? { targetModel: customEndpoint.targetModel } : {}),
    };
  }

  return {
    ok: true,
    value: {
      agentModels: resolvedAgentModels,
      agentEndpoints: resolvedAgentEndpoints,
    },
  };
}
