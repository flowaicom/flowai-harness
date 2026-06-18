/**
 * Settings dialog component.
 *
 * Harness-focused settings panel:
 * - Appearance
 * - Useful presentation/debug toggles
 *
 * @module components/settings/settings-dialog
 */

import {
  CheckCircleIcon,
  ChevronDownIcon,
  DatabaseIcon,
  EyeIcon,
  EyeOffIcon,
  FlaskConicalIcon,
  KeyIcon,
  Loader2Icon,
  MoreHorizontalIcon,
  PaletteIcon,
  SearchIcon,
  ServerIcon,
  ShieldIcon,
  XCircleIcon,
} from "lucide-react";
import { memo, useEffect, useRef, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "~/components/ui/dialog";
import {
  type AgentCustomEndpoint,
  type AgentInfo,
  type AgentRole,
  type FeatureFlags,
  type ModelConfig,
  type ModelKey,
  type ProviderKey,
  type ProviderModel,
  type ProviderSetting,
  type ReasoningEffort,
  type ThemeMode,
  useAgentConfig,
} from "~/lib/stores/settings-store";
import { cn } from "~/lib/utils";
import { SettingsButton } from "./settings-button";

// ============================================================================
// Types
// ============================================================================

export interface SettingsDialogProps {
  children?: React.ReactNode;
  className?: string;
}

// ============================================================================
// Provider Logo (text-based fallback — no next/image)
// ============================================================================

const ProviderBadge = memo(function ProviderBadge({
  label,
  size = 14,
}: {
  label: string;
  size?: number;
}) {
  const initials = label.trim().charAt(0).toUpperCase() || "?";
  return (
    <div
      className="shrink-0 rounded-sm bg-muted flex items-center justify-center text-xs font-medium text-muted-foreground"
      style={{ width: size, height: size }}
    >
      {initials}
    </div>
  );
});

// ============================================================================
// AgentModelSelector
// ============================================================================

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: UI component with conditional rendering
function AgentModelSelector({
  agent,
  selectedProvider,
  selectedModelId,
  availableProviders,
  providerModels,
  customEndpoint,
  isLoadingModels,
  onSelectProvider,
  onSelectModel,
  onOpenCustomEndpointDialog,
  onExpandProvider,
}: {
  agent: AgentInfo;
  selectedProvider: ModelKey;
  selectedModelId: string | undefined;
  availableProviders: ModelConfig[];
  providerModels: ProviderModel[];
  customEndpoint: AgentCustomEndpoint | undefined;
  isLoadingModels: boolean;
  onSelectProvider: (provider: ModelKey) => void;
  onSelectModel: (modelId: string | undefined) => void;
  onOpenCustomEndpointDialog: () => void;
  onExpandProvider: (provider: ModelKey) => void;
}) {
  const [isProviderOpen, setIsProviderOpen] = useState(false);
  const [isModelOpen, setIsModelOpen] = useState(false);
  const [modelSearch, setModelSearch] = useState("");
  const searchInputRef = useRef<HTMLInputElement>(null);
  const selectedProviderConfig = availableProviders.find((p) => p.key === selectedProvider);
  const customSettings = customEndpoint?.settings || {};
  const customSettingCount = Object.keys(customSettings).length;
  const customSettingPreview = Object.entries(customSettings)
    .map(([key, value]) => `${key}=${value}`)
    .join("\n");
  const customTargetModel = customEndpoint?.targetModel;

  const isCustomEndpoint = customSettingCount > 0;

  const modelsForProvider = providerModels
    .filter((m) => m.provider === selectedProvider)
    .sort((a, b) => a.displayName.localeCompare(b.displayName));
  const hasModels = modelsForProvider.length > 0;

  const filteredModels = modelSearch.trim()
    ? modelsForProvider.filter(
        (m) =>
          m.displayName.toLowerCase().includes(modelSearch.toLowerCase()) ||
          m.id.toLowerCase().includes(modelSearch.toLowerCase())
      )
    : modelsForProvider;

  useEffect(() => {
    if (isModelOpen) {
      const timer = setTimeout(() => searchInputRef.current?.focus(), 10);
      return () => clearTimeout(timer);
    }
    setModelSearch("");
  }, [isModelOpen]);

  const selectedModelConfig = selectedModelId
    ? modelsForProvider.find((m) => m.id === selectedModelId)
    : null;
  const isModelIdInvalid = selectedModelId && hasModels && !selectedModelConfig;

  const defaultModelName = selectedProviderConfig?.model
    ? selectedProviderConfig.model.split("/").pop()?.replace(/:.*$/, "") || "Default"
    : "Default";
  const selectedModelDisplay = isCustomEndpoint
    ? customTargetModel || "Custom endpoint"
    : selectedModelId
      ? selectedModelConfig?.displayName ||
        (isModelIdInvalid ? `Warning: ${selectedModelId}` : selectedModelId)
      : defaultModelName;

  return (
    <div className="flex items-center justify-between gap-2 py-3">
      <p className="text-sm font-medium w-24 shrink-0">{agent.displayName}</p>
      <div className="flex items-center gap-1.5 flex-1 min-w-0">
        {/* Provider dropdown */}
        <div className="relative">
          <button
            type="button"
            onClick={() => setIsProviderOpen(!isProviderOpen)}
            className={cn(
              "flex items-center gap-1.5 rounded-md border px-2 py-1.5 text-sm transition-colors",
              "hover:bg-muted/50 w-[120px] justify-between"
            )}
          >
            <span className="flex items-center gap-1.5 min-w-0">
              <ProviderBadge
                label={selectedProviderConfig?.displayName || selectedProvider}
                size={14}
              />
              <span className="truncate text-xs">
                {selectedProviderConfig?.displayName || selectedProvider}
              </span>
            </span>
            <ChevronDownIcon className="size-3 text-muted-foreground shrink-0" />
          </button>

          {isProviderOpen && (
            <>
              {/* biome-ignore lint/a11y/noStaticElementInteractions: backdrop overlay */}
              {/* biome-ignore lint/a11y/useKeyWithClickEvents: dropdown closes on blur */}
              <div className="fixed inset-0 z-10" onClick={() => setIsProviderOpen(false)} />
              <div className="absolute left-0 top-full mt-1 z-20 w-[160px] rounded-md border bg-popover shadow-md max-h-60 overflow-y-auto scroll-container">
                {availableProviders.map((provider) => (
                  <button
                    key={provider.key}
                    type="button"
                    onClick={() => {
                      if (provider.available) {
                        onSelectProvider(provider.key);
                        onExpandProvider(provider.key);
                        setIsProviderOpen(false);
                      }
                    }}
                    disabled={!provider.available}
                    className={cn(
                      "w-full flex items-center gap-2 px-2.5 py-2 text-sm text-left",
                      provider.available
                        ? "hover:bg-muted cursor-pointer"
                        : "opacity-50 cursor-not-allowed",
                      provider.key === selectedProvider && "bg-muted/50"
                    )}
                  >
                    <ProviderBadge label={provider.displayName} size={14} />
                    <span className="flex-1 truncate text-xs">{provider.displayName}</span>
                    {provider.available ? (
                      <CheckCircleIcon className="size-3 text-[var(--dot-emerald)] shrink-0" />
                    ) : (
                      <XCircleIcon className="size-3 text-muted-foreground shrink-0" />
                    )}
                  </button>
                ))}
              </div>
            </>
          )}
        </div>

        {/* Model selection */}
        <div className="relative flex-1 w-0 min-w-0">
          {isCustomEndpoint ? (
            <button
              type="button"
              onClick={onOpenCustomEndpointDialog}
              className={cn(
                "flex items-center gap-1.5 rounded-md border px-2 py-1.5 text-sm transition-colors w-full min-w-0 overflow-hidden",
                "hover:bg-muted/50 justify-between",
                "border-[var(--dot-emerald)]/50 bg-[var(--accent-emerald)]"
              )}
              title={`Custom endpoint transport: ${selectedProviderConfig?.endpointTransportDisplayName || selectedProviderConfig?.endpointTransport || "custom"}\n${customTargetModel ? `Target model: ${customTargetModel}\n` : ""}${customSettingPreview ? `${customSettingPreview}\n` : ""}Click to configure`}
            >
              <span className="flex items-center gap-1 truncate text-xs min-w-0">
                <ServerIcon className="size-3 text-[var(--dot-emerald)] shrink-0" />
                <span className="text-[var(--dot-emerald)] truncate min-w-0">
                  {customTargetModel || "Custom endpoint"}
                </span>
              </span>
              <ChevronDownIcon className="size-3 text-[var(--dot-emerald)] shrink-0" />
            </button>
          ) : (
            <button
              type="button"
              onClick={() => {
                if (!isLoadingModels && hasModels) {
                  setIsModelOpen(!isModelOpen);
                } else if (!hasModels) {
                  onExpandProvider(selectedProvider);
                }
              }}
              disabled={isLoadingModels}
              className={cn(
                "flex items-center gap-1.5 rounded-md border px-2 py-1.5 text-sm transition-colors w-full min-w-0 overflow-hidden",
                "hover:bg-muted/50 justify-between",
                isLoadingModels && "opacity-50",
                isModelIdInvalid && "border-[var(--dot-amber)] bg-[var(--accent-amber)]"
              )}
              title={
                isModelIdInvalid
                  ? `Model "${selectedModelId}" not found in ${selectedProvider}. Click to select a valid model.`
                  : undefined
              }
            >
              <span
                className={cn(
                  "truncate text-xs min-w-0",
                  isModelIdInvalid ? "text-[var(--dot-amber)]" : "text-muted-foreground"
                )}
              >
                {isLoadingModels ? (
                  <span className="inline-block h-3 w-24 bg-muted rounded animate-pulse" />
                ) : (
                  selectedModelDisplay
                )}
              </span>
              {isLoadingModels ? (
                <Loader2Icon className="size-3 animate-spin text-muted-foreground shrink-0" />
              ) : (
                <ChevronDownIcon className="size-3 text-muted-foreground shrink-0" />
              )}
            </button>
          )}

          {isModelOpen && hasModels && (
            <>
              {/* biome-ignore lint/a11y/noStaticElementInteractions: backdrop overlay */}
              {/* biome-ignore lint/a11y/useKeyWithClickEvents: dropdown closes on blur */}
              <div className="fixed inset-0 z-10" onClick={() => setIsModelOpen(false)} />
              <div className="absolute left-0 right-0 top-full mt-1 z-20 rounded-md border bg-popover shadow-md max-h-72 flex flex-col">
                <div className="p-1.5 border-b sticky top-0 bg-popover">
                  <div className="relative">
                    <SearchIcon className="absolute left-2 top-1/2 -translate-y-1/2 size-3 text-muted-foreground" />
                    <input
                      ref={searchInputRef}
                      type="text"
                      value={modelSearch}
                      onChange={(e) => setModelSearch(e.target.value)}
                      placeholder="Search models..."
                      className="w-full text-xs px-2 py-1.5 pl-7 rounded border bg-background focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none"
                      onClick={(e) => e.stopPropagation()}
                    />
                  </div>
                </div>
                <div className="overflow-y-auto scroll-container flex-1">
                  {(!modelSearch.trim() ||
                    defaultModelName.toLowerCase().includes(modelSearch.toLowerCase()) ||
                    "default".includes(modelSearch.toLowerCase())) && (
                    <button
                      type="button"
                      onClick={() => {
                        onSelectModel(undefined);
                        setIsModelOpen(false);
                      }}
                      className={cn(
                        "w-full flex items-center gap-2 px-2.5 py-2 text-xs text-left",
                        "hover:bg-muted cursor-pointer",
                        !selectedModelId && "bg-muted/50"
                      )}
                    >
                      <span className="flex-1 truncate">{defaultModelName}</span>
                      <span className="text-muted-foreground shrink-0">(default)</span>
                      {!selectedModelId && (
                        <CheckCircleIcon className="size-3 text-[var(--dot-emerald)] shrink-0" />
                      )}
                    </button>
                  )}
                  {filteredModels.length > 0 ? (
                    filteredModels.map((model) => (
                      <button
                        key={model.id}
                        type="button"
                        onClick={() => {
                          onSelectModel(model.id);
                          setIsModelOpen(false);
                        }}
                        className={cn(
                          "w-full flex items-center gap-2 px-2.5 py-2 text-xs text-left",
                          "hover:bg-muted cursor-pointer",
                          model.id === selectedModelId && "bg-muted/50"
                        )}
                      >
                        <span className="flex-1 truncate">{model.displayName}</span>
                        {model.pricing && (
                          <span className="text-muted-foreground shrink-0">
                            ${model.pricing.inputPerMTok.toFixed(2)}/M
                          </span>
                        )}
                        {model.id === selectedModelId && (
                          <CheckCircleIcon className="size-3 text-[var(--dot-emerald)] shrink-0" />
                        )}
                      </button>
                    ))
                  ) : (
                    <div className="px-2.5 py-3 text-xs text-muted-foreground text-center">
                      No models match &quot;{modelSearch}&quot;
                    </div>
                  )}
                </div>
              </div>
            </>
          )}
        </div>

        {/* Custom endpoint button */}
        <button
          type="button"
          onClick={onOpenCustomEndpointDialog}
          title={isCustomEndpoint ? "Custom endpoint configured" : "Configure custom endpoint"}
          className={cn(
            "p-1.5 rounded-md border transition-colors shrink-0",
            isCustomEndpoint
              ? "bg-[var(--accent-emerald)] border-[var(--dot-emerald)] text-[var(--dot-emerald)]"
              : "hover:bg-muted/50 text-muted-foreground"
          )}
        >
          {isCustomEndpoint ? (
            <ServerIcon className="size-4" />
          ) : (
            <MoreHorizontalIcon className="size-4" />
          )}
        </button>
      </div>
    </div>
  );
}

// ============================================================================
// ProviderSettingField
// ============================================================================

function ProviderSettingField({
  provider,
  setting,
  value,
  onSetValue,
}: {
  provider: ModelConfig;
  setting: ProviderSetting;
  value: string | undefined;
  onSetValue: (value: string | undefined) => void;
}) {
  const [isVisible, setIsVisible] = useState(false);
  const [draftValue, setDraftValue] = useState(value ?? setting.defaultValue ?? "");

  useEffect(() => {
    setDraftValue(value ?? setting.defaultValue ?? "");
  }, [setting.defaultValue, value]);

  const handleBlur = () => {
    const nextValue = draftValue.trim() || undefined;
    if (nextValue !== value) {
      onSetValue(nextValue);
    }
  };

  if (setting.kind === "select") {
    const selectedValue = value ?? setting.defaultValue ?? setting.options[0]?.key ?? "";
    return (
      <div className="flex items-center gap-3 py-2">
        <ProviderBadge label={provider.displayName} size={20} />
        <div className="w-32">
          <span className="text-sm font-medium">{setting.displayName}</span>
          <p className="text-xs text-muted-foreground">{provider.displayName}</p>
        </div>
        <div className="flex-1">
          <select
            value={selectedValue}
            onChange={(e) => onSetValue(e.target.value || undefined)}
            className={cn(
              "w-full text-sm px-2.5 py-1.5 rounded-md border bg-background",
              "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
            )}
          >
            {setting.options.map((option) => (
              <option key={option.key} value={option.key}>
                {option.displayName}
              </option>
            ))}
          </select>
          {setting.description && (
            <p className="text-xs text-muted-foreground mt-1">{setting.description}</p>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="flex items-center gap-3 py-2">
      <ProviderBadge label={provider.displayName} size={20} />
      <div className="w-32">
        <span className="text-sm font-medium">{setting.displayName}</span>
        <p className="text-xs text-muted-foreground">{provider.displayName}</p>
      </div>
      <div className="flex-1 relative">
        <input
          type={setting.kind === "secret" && !isVisible ? "password" : "text"}
          value={draftValue}
          onChange={(e) => setDraftValue(e.target.value)}
          onBlur={handleBlur}
          placeholder={setting.description || `Enter ${setting.displayName.toLowerCase()}...`}
          className={cn(
            "w-full text-sm px-2.5 py-1.5 rounded-md border bg-background",
            "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          )}
        />
        {setting.kind === "secret" && draftValue && (
          <button
            type="button"
            onClick={() => setIsVisible(!isVisible)}
            className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
            aria-label={isVisible ? "Hide API key" : "Show API key"}
          >
            {isVisible ? <EyeOffIcon className="size-4" /> : <EyeIcon className="size-4" />}
          </button>
        )}
      </div>
      {value && <CheckCircleIcon className="size-4 text-[var(--dot-emerald)] shrink-0" />}
    </div>
  );
}

// ============================================================================
// ProviderSettingsSection
// ============================================================================

function ProviderSettingsSection({
  providers,
  providerSettings,
  onSetProviderSetting,
}: {
  providers: readonly ModelConfig[];
  providerSettings: Partial<Record<ProviderKey, Record<string, string>>>;
  onSetProviderSetting: (
    provider: ProviderKey,
    settingKey: string,
    value: string | undefined
  ) => void;
}) {
  const [isExpanded, setIsExpanded] = useState(false);
  const configurableProviders = providers
    .filter((provider) => provider.settings.length > 0)
    .sort((a, b) => a.displayName.localeCompare(b.displayName));

  if (configurableProviders.length === 0) {
    return null;
  }

  return (
    <div className="border-t pt-4">
      <button
        type="button"
        onClick={() => setIsExpanded(!isExpanded)}
        className="flex items-center gap-2 text-xs text-muted-foreground hover:text-foreground transition-colors w-full"
      >
        <KeyIcon className="size-3.5" />
        <span>Provider Settings</span>
        <ChevronDownIcon
          className={cn("size-3.5 ml-auto transition-transform", isExpanded && "rotate-180")}
        />
      </button>

      {isExpanded && (
        <div className="mt-3 space-y-1">
          <p className="text-xs text-muted-foreground mb-3">
            Override provider-specific credentials and context. Settings are stored locally and used
            when Studio loads provider models.
          </p>
          {configurableProviders.map((provider) => (
            <div key={provider.key} className="space-y-1">
              {provider.settings.map((setting) => (
                <ProviderSettingField
                  key={`${provider.key}:${setting.key}`}
                  provider={provider}
                  setting={setting}
                  value={providerSettings[provider.key]?.[setting.key]}
                  onSetValue={(nextValue) =>
                    onSetProviderSetting(provider.key, setting.key, nextValue)
                  }
                />
              ))}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ============================================================================
// InfrastructureSection
// ============================================================================

function InfrastructureSection({
  neondbApiKey,
  neondbProjectId,
  onSetNeondbApiKey,
  onSetNeondbProjectId,
}: {
  neondbApiKey: string | undefined;
  neondbProjectId: string | undefined;
  onSetNeondbApiKey: (apiKey: string | undefined) => void;
  onSetNeondbProjectId: (projectId: string | undefined) => void;
}) {
  const [isExpanded, setIsExpanded] = useState(false);
  const [apiKeyVisible, setApiKeyVisible] = useState(false);
  const [apiKeyValue, setApiKeyValue] = useState(neondbApiKey || "");
  const [projectIdValue, setProjectIdValue] = useState(neondbProjectId || "");

  useEffect(() => {
    setApiKeyValue(neondbApiKey || "");
  }, [neondbApiKey]);

  useEffect(() => {
    setProjectIdValue(neondbProjectId || "");
  }, [neondbProjectId]);

  const handleApiKeyBlur = () => {
    if (apiKeyValue.trim() !== (neondbApiKey || "").trim()) {
      onSetNeondbApiKey(apiKeyValue.trim() || undefined);
    }
  };

  const handleProjectIdBlur = () => {
    if (projectIdValue.trim() !== (neondbProjectId || "").trim()) {
      onSetNeondbProjectId(projectIdValue.trim() || undefined);
    }
  };

  return (
    <div className="border-t pt-4">
      <button
        type="button"
        onClick={() => setIsExpanded(!isExpanded)}
        className="flex items-center gap-2 text-xs text-muted-foreground hover:text-foreground transition-colors w-full"
      >
        <DatabaseIcon className="size-3.5" />
        <span>Infrastructure</span>
        <ChevronDownIcon
          className={cn("size-3.5 ml-auto transition-transform", isExpanded && "rotate-180")}
        />
      </button>

      {isExpanded && (
        <div className="mt-3 space-y-3">
          <p className="text-xs text-muted-foreground mb-3">
            Configure database providers for workspace storage.
          </p>

          {/* NeonDB API Key */}
          <div className="flex items-center gap-3 py-2">
            <span className="text-sm font-medium w-28 shrink-0">NeonDB API Key</span>
            <div className="flex-1 relative">
              <input
                type={apiKeyVisible ? "text" : "password"}
                value={apiKeyValue}
                onChange={(e) => setApiKeyValue(e.target.value)}
                onBlur={handleApiKeyBlur}
                placeholder="napi-..."
                className={cn(
                  "w-full text-sm px-2.5 py-1.5 rounded-md border bg-background",
                  "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                )}
              />
              {apiKeyValue && (
                <button
                  type="button"
                  onClick={() => setApiKeyVisible(!apiKeyVisible)}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                  aria-label={apiKeyVisible ? "Hide API key" : "Show API key"}
                >
                  {apiKeyVisible ? (
                    <EyeOffIcon className="size-4" />
                  ) : (
                    <EyeIcon className="size-4" />
                  )}
                </button>
              )}
            </div>
            {neondbApiKey && (
              <CheckCircleIcon className="size-4 text-[var(--dot-emerald)] shrink-0" />
            )}
          </div>

          {/* NeonDB Project ID */}
          <div className="flex items-center gap-3 py-2">
            <span className="text-sm font-medium w-28 shrink-0">NeonDB Project ID</span>
            <div className="flex-1">
              <input
                type="text"
                value={projectIdValue}
                onChange={(e) => setProjectIdValue(e.target.value)}
                onBlur={handleProjectIdBlur}
                placeholder="project-id-..."
                className={cn(
                  "w-full text-sm px-2.5 py-1.5 rounded-md border bg-background",
                  "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                )}
              />
            </div>
            {neondbProjectId && (
              <CheckCircleIcon className="size-4 text-[var(--dot-emerald)] shrink-0" />
            )}
          </div>
        </div>
      )}
    </div>
  );
}

// ============================================================================
// ExperimentalFeaturesSection
// ============================================================================

function ExperimentalFeaturesSection({
  featureFlags,
  onSetFeatureFlag,
}: {
  featureFlags: FeatureFlags;
  onSetFeatureFlag: <K extends keyof FeatureFlags>(flag: K, value: FeatureFlags[K]) => void;
}) {
  const [isExpanded, setIsExpanded] = useState(false);

  return (
    <div className="border-t pt-4">
      <button
        type="button"
        onClick={() => setIsExpanded(!isExpanded)}
        className="flex items-center gap-2 text-xs text-muted-foreground hover:text-foreground transition-colors w-full"
      >
        <FlaskConicalIcon className="size-3.5" />
        <span>Experimental Features</span>
        <ChevronDownIcon
          className={cn("size-3.5 ml-auto transition-transform", isExpanded && "rotate-180")}
        />
      </button>

      {isExpanded && (
        <div className="mt-3 space-y-3">
          <p className="text-xs text-muted-foreground">
            Enable experimental features. These may be unstable or incomplete.
          </p>

          {/* Latency Panel toggle */}
          <label className="flex items-center justify-between gap-3 py-2 cursor-pointer">
            <div className="flex-1">
              <p className="text-sm font-medium">Latency Metrics Panel</p>
              <p className="text-xs text-muted-foreground">
                Show streaming latency metrics for debugging performance (TTFC, TTFT, tool calls)
              </p>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={featureFlags.showLatencyPanel}
              onClick={() => onSetFeatureFlag("showLatencyPanel", !featureFlags.showLatencyPanel)}
              className={cn(
                "relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors",
                "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
                featureFlags.showLatencyPanel ? "bg-primary" : "bg-input"
              )}
            >
              <span
                className={cn(
                  "pointer-events-none block h-4 w-4 rounded-full bg-background shadow-lg ring-0 transition-transform",
                  featureFlags.showLatencyPanel ? "translate-x-4" : "translate-x-0"
                )}
              />
            </button>
          </label>

          {/* Demo Mode (PII Scramble) toggle */}
          <label className="flex items-center justify-between gap-3 py-2 cursor-pointer">
            <div className="flex-1">
              <div className="flex items-center gap-1.5">
                <ShieldIcon className="size-3.5 text-[var(--dot-amber)]" />
                <p className="text-sm font-medium">Demo Mode</p>
              </div>
              <p className="text-xs text-muted-foreground">
                Scramble business data text for presentations. Numbers, UI controls, and settings
                remain readable. Purely visual — no functional impact.
              </p>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={featureFlags.piiScramble}
              onClick={() => onSetFeatureFlag("piiScramble", !featureFlags.piiScramble)}
              className={cn(
                "relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors",
                "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
                featureFlags.piiScramble ? "bg-[var(--dot-amber)]" : "bg-input"
              )}
            >
              <span
                className={cn(
                  "pointer-events-none block h-4 w-4 rounded-full bg-background shadow-lg ring-0 transition-transform",
                  featureFlags.piiScramble ? "translate-x-4" : "translate-x-0"
                )}
              />
            </button>
          </label>
        </div>
      )}
    </div>
  );
}

// ============================================================================
// AppearanceSection
// ============================================================================

const THEME_OPTIONS: { value: ThemeMode; label: string; bg: string; bar: string }[] = [
  {
    value: "system",
    label: "System",
    bg: "bg-gradient-to-br from-white to-zinc-800",
    bar: "bg-zinc-400",
  },
  { value: "light", label: "Light", bg: "bg-white", bar: "bg-zinc-200" },
  { value: "dark", label: "Dark", bg: "bg-zinc-900", bar: "bg-zinc-700" },
  { value: "slate", label: "Slate", bg: "bg-slate-800", bar: "bg-slate-600" },
];

function AppearanceSection() {
  const theme = useAgentConfig((s) => s.theme);
  const setTheme = useAgentConfig((s) => s.setTheme);
  const [isExpanded, setIsExpanded] = useState(false);

  return (
    <div className="border-t pt-4">
      <button
        type="button"
        onClick={() => setIsExpanded(!isExpanded)}
        className="flex items-center gap-2 text-xs text-muted-foreground hover:text-foreground transition-colors w-full"
      >
        <PaletteIcon className="size-3.5" />
        <span>Appearance</span>
        <span className="ml-1 text-foreground font-medium capitalize">{theme}</span>
        <ChevronDownIcon
          className={cn("size-3.5 ml-auto transition-transform", isExpanded && "rotate-180")}
        />
      </button>

      {isExpanded && (
        <div className="mt-3 space-y-3">
          <p className="text-xs text-muted-foreground">
            Choose a color theme for the studio interface.
          </p>
          <div className="grid grid-cols-4 gap-2">
            {THEME_OPTIONS.map((t) => (
              <button
                key={t.value}
                type="button"
                onClick={() => setTheme(t.value)}
                className={cn(
                  "flex flex-col items-center gap-1.5 rounded-lg border p-2.5 transition-colors",
                  theme === t.value
                    ? "border-primary bg-primary/5"
                    : "border-transparent hover:bg-muted/50"
                )}
              >
                <div className={cn("w-full aspect-[4/3] rounded-md border overflow-hidden", t.bg)}>
                  <div className={cn("h-1.5 w-full", t.bar)} />
                  <div className="p-1 space-y-0.5">
                    <div className={cn("h-1 w-3/4 rounded-sm opacity-60", t.bar)} />
                    <div className={cn("h-1 w-1/2 rounded-sm opacity-40", t.bar)} />
                  </div>
                </div>
                <span className="text-xs font-medium">{t.label}</span>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

// ============================================================================
// ThinkingBudgetInput
// ============================================================================

function REASONING_EFFORT_OPTIONS(): Array<{
  value: ReasoningEffort;
  label: string;
  description: string;
}> {
  return [
    { value: "low", label: "Low", description: "Fastest, most conservative token usage." },
    { value: "medium", label: "Medium", description: "Balanced speed, cost, and capability." },
    {
      value: "high",
      label: "High",
      description: "Default; highest unconstrained reasoning quality.",
    },
    {
      value: "max",
      label: "Max",
      description: "Absolute maximum effort on Opus 4.6; downgraded automatically elsewhere.",
    },
  ];
}

function ReasoningEffortInput() {
  const reasoningEffort = useAgentConfig((s) => s.reasoningEffort);
  const setReasoningEffort = useAgentConfig((s) => s.setReasoningEffort);

  return (
    <div className="border-t pt-4">
      <div className="flex items-center justify-between gap-3">
        <div className="flex-1">
          <p className="text-sm font-medium">Reasoning Effort</p>
          <p className="text-xs text-muted-foreground">
            Anthropic effort level. High is the default; lower values trade quality for speed.
          </p>
        </div>
        <select
          value={reasoningEffort}
          onChange={(e) => setReasoningEffort(e.target.value as ReasoningEffort)}
          className={cn(
            "w-32 text-sm px-2.5 py-1.5 rounded-md border bg-background",
            "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          )}
        >
          {REASONING_EFFORT_OPTIONS().map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      </div>
    </div>
  );
}

function ThinkingBudgetInput() {
  const thinkingBudgetTokens = useAgentConfig((s) => s.thinkingBudgetTokens);
  const setThinkingBudgetTokens = useAgentConfig((s) => s.setThinkingBudgetTokens);
  const [localValue, setLocalValue] = useState(String(thinkingBudgetTokens));

  useEffect(() => {
    setLocalValue(String(thinkingBudgetTokens));
  }, [thinkingBudgetTokens]);

  const handleBlur = () => {
    const parsed = Number.parseInt(localValue, 10);
    if (!Number.isNaN(parsed) && parsed >= 0) {
      setThinkingBudgetTokens(parsed);
    } else {
      setLocalValue(String(thinkingBudgetTokens));
    }
  };

  return (
    <div className="border-t pt-4">
      <div className="flex items-center justify-between gap-3">
        <div className="flex-1">
          <p className="text-sm font-medium">Thinking Budget</p>
          <p className="text-xs text-muted-foreground">
            `0` uses adaptive thinking with no manual cap on Claude 4.6. Positive values set a
            manual budget.
          </p>
        </div>
        <input
          type="number"
          min={0}
          step={1000}
          value={localValue}
          onChange={(e) => setLocalValue(e.target.value)}
          onBlur={handleBlur}
          onKeyDown={(e) => e.key === "Enter" && handleBlur()}
          className={cn(
            "w-28 text-sm px-2.5 py-1.5 rounded-md border bg-background text-right",
            "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          )}
        />
      </div>
    </div>
  );
}

function MaxTokensInput() {
  const maxTokens = useAgentConfig((s) => s.maxTokens);
  const setMaxTokens = useAgentConfig((s) => s.setMaxTokens);
  const [localValue, setLocalValue] = useState(String(maxTokens));

  useEffect(() => {
    setLocalValue(String(maxTokens));
  }, [maxTokens]);

  const handleBlur = () => {
    const parsed = Number.parseInt(localValue, 10);
    if (!Number.isNaN(parsed) && parsed > 0) {
      setMaxTokens(parsed);
    } else {
      setLocalValue(String(maxTokens));
    }
  };

  return (
    <div className="border-t pt-4">
      <div className="flex items-center justify-between gap-3">
        <div className="flex-1">
          <p className="text-sm font-medium">Max Tokens</p>
          <p className="text-xs text-muted-foreground">
            Hard response cap. Raise this to reduce truncation on long tool-heavy turns.
          </p>
        </div>
        <input
          type="number"
          min={1}
          step={1024}
          value={localValue}
          onChange={(e) => setLocalValue(e.target.value)}
          onBlur={handleBlur}
          onKeyDown={(e) => e.key === "Enter" && handleBlur()}
          className={cn(
            "w-28 text-sm px-2.5 py-1.5 rounded-md border bg-background text-right",
            "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          )}
        />
      </div>
    </div>
  );
}

// ============================================================================
// Main Settings Dialog
// ============================================================================

export function SettingsDialog({ children, className }: SettingsDialogProps) {
  const isOpen = useAgentConfig((s) => s.settingsDialogOpen);
  const featureFlags = useAgentConfig((s) => s.featureFlags);
  const setFeatureFlag = useAgentConfig((s) => s.setFeatureFlag);
  const openSettingsDialog = useAgentConfig((s) => s.openSettingsDialog);
  const closeSettingsDialog = useAgentConfig((s) => s.closeSettingsDialog);

  // Load config when dialog opens
  const handleOpenChange = (open: boolean) => {
    if (open) {
      openSettingsDialog();
    } else {
      closeSettingsDialog();
    }
  };

  return (
    <Dialog open={isOpen} onOpenChange={handleOpenChange}>
      {children ? (
        <button
          type="button"
          onClick={() => handleOpenChange(true)}
          className="inline-flex appearance-none bg-transparent border-0 p-0 cursor-pointer"
        >
          {children}
        </button>
      ) : (
        <SettingsButton onClick={() => handleOpenChange(true)} />
      )}
      <DialogContent className={cn("max-w-xl", className)}>
        <DialogHeader>
          <DialogTitle>Settings</DialogTitle>
          <DialogDescription>Configure Studio appearance and presentation tools</DialogDescription>
        </DialogHeader>

        <div className="py-4">
          <div className="space-y-4">
            <AppearanceSection />
            <ExperimentalFeaturesSection
              featureFlags={featureFlags}
              onSetFeatureFlag={setFeatureFlag}
            />
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
