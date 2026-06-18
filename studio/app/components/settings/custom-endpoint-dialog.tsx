/**
 * Custom endpoint configuration dialog.
 *
 * Renders transport-scoped endpoint settings from backend metadata instead of
 * hardcoding one OpenAI-compatible form.
 *
 * @module components/settings/custom-endpoint-dialog
 */

import {
  CheckCircleIcon,
  ChevronDownIcon,
  EyeIcon,
  EyeOffIcon,
  Loader2Icon,
  ServerIcon,
  XCircleIcon,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "~/components/ui/dialog";
import { verifyConnection as verifyEndpointConnection } from "~/lib/api/studio";
import { isOk } from "~/lib/domain/result";
import type { AgentCustomEndpoint, ProviderSetting } from "~/lib/stores/settings-store";
import { cn } from "~/lib/utils";

type ConnectionStatus = "idle" | "checking" | "success" | "error";

interface RemoteModel {
  id: string;
  object: string;
  created?: number;
  owned_by?: string;
}

export interface CustomEndpointDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  agentDisplayName: string;
  modelDisplayName: string;
  endpointTransport?: string | null;
  endpointTransportLabel?: string | null;
  endpointTransportSettings: ProviderSetting[];
  customEndpoint: AgentCustomEndpoint | undefined;
  onSaveCustomEndpoint: (endpoint: Partial<AgentCustomEndpoint> | undefined) => void;
}

function normalizeSettingValue(value: string | null | undefined): string | undefined {
  const normalized = value?.trim();
  return normalized ? normalized : undefined;
}

function renderSettingLabel(setting: ProviderSetting): string {
  return setting.required ? `${setting.displayName} *` : setting.displayName;
}

export function CustomEndpointDialog({
  open,
  onOpenChange,
  agentDisplayName,
  modelDisplayName,
  endpointTransport,
  endpointTransportLabel,
  endpointTransportSettings,
  customEndpoint,
  onSaveCustomEndpoint,
}: CustomEndpointDialogProps) {
  const [settingValues, setSettingValues] = useState<Record<string, string>>(
    customEndpoint?.settings || {}
  );
  const [targetModelInput, setTargetModelInput] = useState(customEndpoint?.targetModel || "");
  const [secretVisibility, setSecretVisibility] = useState<Record<string, boolean>>({});
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>("idle");
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const [availableModels, setAvailableModels] = useState<RemoteModel[]>([]);
  const [showModelDropdown, setShowModelDropdown] = useState(false);
  const resetTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const verifyGenRef = useRef(0);

  useEffect(() => {
    if (open) {
      setSettingValues(customEndpoint?.settings || {});
      setTargetModelInput(customEndpoint?.targetModel || "");
      setSecretVisibility({});
      setConnectionStatus("idle");
      setConnectionError(null);
      setAvailableModels([]);
    }
    return () => {
      if (resetTimerRef.current) {
        clearTimeout(resetTimerRef.current);
        resetTimerRef.current = null;
      }
    };
  }, [open, customEndpoint]);

  const resolvedSettings = useMemo(() => {
    const entries = endpointTransportSettings
      .map((setting) => {
        const explicit = normalizeSettingValue(settingValues[setting.key]);
        const fallback = normalizeSettingValue(setting.defaultValue);
        return [setting.key, explicit || fallback] as const;
      })
      .filter((entry): entry is readonly [string, string] => !!entry[1]);
    return Object.fromEntries(entries);
  }, [endpointTransportSettings, settingValues]);

  const hasRequiredSettings = endpointTransportSettings
    .filter((setting) => setting.required)
    .every((setting) => !!resolvedSettings[setting.key]);
  const hasEndpointSettings = Object.keys(resolvedSettings).length > 0;

  const verifyConnection = async () => {
    if (!endpointTransport || !hasRequiredSettings) return;

    // Cancel any pending reset timer from a previous verification.
    if (resetTimerRef.current) {
      clearTimeout(resetTimerRef.current);
      resetTimerRef.current = null;
    }

    const gen = ++verifyGenRef.current;

    setConnectionStatus("checking");
    setConnectionError(null);
    setAvailableModels([]);

    try {
      const response = await verifyEndpointConnection({
        verifier: endpointTransport,
        settings: resolvedSettings,
      });

      // Discard stale responses from a superseded verification.
      if (gen !== verifyGenRef.current) return;

      if (isOk(response) && response.value.connected) {
        setConnectionStatus("success");

        if (response.value.models && response.value.models.length > 0) {
          const models = response.value.models.map((id) => ({ id, object: "model" }));
          setAvailableModels(models);
        }
      } else {
        setConnectionStatus("error");
        setConnectionError(
          isOk(response) ? response.value.error || "Connection failed" : response.error.message
        );
      }

      resetTimerRef.current = setTimeout(() => {
        setConnectionStatus("idle");
      }, 3000);
    } catch (error) {
      if (gen !== verifyGenRef.current) return;

      if (import.meta.env.DEV) {
        console.error("Connection verification failed:", error);
      }
      setConnectionStatus("error");
      setConnectionError(error instanceof Error ? error.message : "Connection verification failed");
      resetTimerRef.current = setTimeout(() => {
        setConnectionStatus("idle");
      }, 3000);
    }
  };

  const handleSave = () => {
    onSaveCustomEndpoint(
      endpointTransport && hasEndpointSettings
        ? {
            settings: resolvedSettings,
            ...(normalizeSettingValue(targetModelInput)
              ? { targetModel: normalizeSettingValue(targetModelInput) }
              : {}),
          }
        : undefined
    );
    onOpenChange(false);
  };

  const handleClear = () => {
    setSettingValues({});
    setTargetModelInput("");
    setAvailableModels([]);
    onSaveCustomEndpoint(undefined);
    setConnectionStatus("idle");
    setConnectionError(null);
  };

  const selectModel = (modelId: string) => {
    setTargetModelInput(modelId);
    setShowModelDropdown(false);
  };

  const renderSettingField = (setting: ProviderSetting) => {
    const value = settingValues[setting.key] ?? "";
    const placeholder =
      setting.defaultValue ?? (setting.key === "baseUrl" ? "http://localhost:5001/v1" : undefined);

    if (setting.kind === "select") {
      return (
        <div key={setting.key} className="space-y-2">
          <label htmlFor={`endpoint-setting-${setting.key}`} className="text-sm font-medium">
            {renderSettingLabel(setting)}
          </label>
          <select
            id={`endpoint-setting-${setting.key}`}
            value={value}
            onChange={(event) =>
              setSettingValues((current) => ({
                ...current,
                [setting.key]: event.target.value,
              }))
            }
            className={cn(
              "w-full text-sm px-3 py-2 rounded-md border bg-background",
              "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
            )}
          >
            <option value="">Select...</option>
            {setting.options.map((option) => (
              <option key={option.key} value={option.key}>
                {option.displayName}
              </option>
            ))}
          </select>
          {setting.description && (
            <p className="text-xs text-muted-foreground">{setting.description}</p>
          )}
        </div>
      );
    }

    if (setting.kind === "secret") {
      const isVisible = secretVisibility[setting.key] || false;
      return (
        <div key={setting.key} className="space-y-2">
          <label htmlFor={`endpoint-setting-${setting.key}`} className="text-sm font-medium">
            {renderSettingLabel(setting)}
          </label>
          <div className="relative">
            <input
              id={`endpoint-setting-${setting.key}`}
              type={isVisible ? "text" : "password"}
              placeholder={placeholder}
              value={value}
              onChange={(event) =>
                setSettingValues((current) => ({
                  ...current,
                  [setting.key]: event.target.value,
                }))
              }
              className={cn(
                "w-full text-sm px-3 py-2 pr-10 rounded-md border bg-background",
                "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                "placeholder:text-muted-foreground/50"
              )}
            />
            {value && (
              <button
                type="button"
                onClick={() =>
                  setSecretVisibility((current) => ({
                    ...current,
                    [setting.key]: !isVisible,
                  }))
                }
                className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                aria-label={isVisible ? "Hide secret" : "Show secret"}
              >
                {isVisible ? <EyeOffIcon className="size-4" /> : <EyeIcon className="size-4" />}
              </button>
            )}
          </div>
          {setting.description && (
            <p className="text-xs text-muted-foreground">{setting.description}</p>
          )}
        </div>
      );
    }

    return (
      <div key={setting.key} className="space-y-2">
        <label htmlFor={`endpoint-setting-${setting.key}`} className="text-sm font-medium">
          {renderSettingLabel(setting)}
        </label>
        <input
          id={`endpoint-setting-${setting.key}`}
          type={setting.key.toLowerCase().includes("url") ? "url" : "text"}
          placeholder={placeholder}
          value={value}
          onChange={(event) =>
            setSettingValues((current) => ({
              ...current,
              [setting.key]: event.target.value,
            }))
          }
          className={cn(
            "w-full text-sm px-3 py-2 rounded-md border bg-background",
            "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
            "placeholder:text-muted-foreground/50"
          )}
        />
        {setting.description && (
          <p className="text-xs text-muted-foreground">{setting.description}</p>
        )}
      </div>
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Custom Endpoint</DialogTitle>
          <DialogDescription>
            Configure a custom endpoint for <strong>{agentDisplayName}</strong> using{" "}
            <strong>{modelDisplayName}</strong>
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-4">
          <div className="space-y-2">
            <label className="text-sm font-medium">Transport</label>
            <div
              className={cn(
                "w-full text-sm px-3 py-2 rounded-md border bg-muted/40",
                endpointTransport ? "text-foreground" : "text-destructive border-destructive"
              )}
            >
              {endpointTransportLabel || endpointTransport || "Not declared by this provider"}
            </div>
            <p className="text-xs text-muted-foreground">
              Custom endpoints use the transport declared by the selected provider metadata.
            </p>
          </div>

          {endpointTransportSettings.map(renderSettingField)}

          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={verifyConnection}
              disabled={
                !endpointTransport || !hasRequiredSettings || connectionStatus === "checking"
              }
              className={cn(
                "flex-1 flex items-center justify-center gap-2 px-4 py-2 text-sm rounded-md border transition-colors",
                (!endpointTransport || !hasRequiredSettings) && "opacity-50 cursor-not-allowed",
                hasRequiredSettings && connectionStatus === "idle" && "hover:bg-muted",
                connectionStatus === "checking" && "cursor-wait",
                connectionStatus === "success" &&
                  "bg-[var(--accent-emerald)] border-[var(--dot-emerald)] text-[var(--dot-emerald)]",
                connectionStatus === "error" &&
                  "bg-[var(--accent-red)] border-[var(--dot-red)] text-[var(--dot-red)]"
              )}
            >
              {connectionStatus === "checking" && (
                <>
                  <Loader2Icon className="size-4 animate-spin" />
                  Verifying connection...
                </>
              )}
              {connectionStatus === "success" && (
                <>
                  <CheckCircleIcon className="size-4" />
                  Connection successful
                </>
              )}
              {connectionStatus === "error" && (
                <>
                  <XCircleIcon className="size-4" />
                  Connection failed
                </>
              )}
              {connectionStatus === "idle" && (
                <>
                  <ServerIcon className="size-4" />
                  Verify Connection
                </>
              )}
            </button>
          </div>
          {connectionError && <p className="text-xs text-destructive">{connectionError}</p>}
          {!endpointTransport && (
            <p className="text-xs text-destructive">
              This provider does not declare a custom endpoint transport. Switch providers or extend
              the provider metadata before using a custom endpoint here.
            </p>
          )}

          <div className="space-y-2">
            <label htmlFor="target-model-input" className="text-sm font-medium">
              Target Model
            </label>
            <div className="relative">
              <input
                id="target-model-input"
                type="text"
                placeholder="Optional: override the endpoint model identifier"
                value={targetModelInput}
                onChange={(event) => setTargetModelInput(event.target.value)}
                onFocus={() => availableModels.length > 0 && setShowModelDropdown(true)}
                className={cn(
                  "w-full text-sm px-3 py-2 rounded-md border bg-background",
                  "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                  "placeholder:text-muted-foreground/50",
                  availableModels.length > 0 && "pr-10"
                )}
              />
              {availableModels.length > 0 && (
                <button
                  type="button"
                  onClick={() => setShowModelDropdown(!showModelDropdown)}
                  className="absolute right-2 top-1/2 -translate-y-1/2 p-1 hover:bg-muted rounded"
                >
                  <ChevronDownIcon
                    className={cn("size-4 transition-transform", showModelDropdown && "rotate-180")}
                  />
                </button>
              )}

              {showModelDropdown && availableModels.length > 0 && (
                <div className="absolute z-10 w-full mt-1 bg-background border rounded-md shadow-lg max-h-48 overflow-y-auto scroll-container">
                  {availableModels.map((model) => (
                    <button
                      key={model.id}
                      type="button"
                      onClick={() => selectModel(model.id)}
                      className={cn(
                        "w-full text-left px-3 py-2 text-sm hover:bg-muted transition-colors",
                        targetModelInput === model.id && "bg-muted font-medium"
                      )}
                    >
                      <div className="font-medium">{model.id}</div>
                      {model.owned_by && (
                        <div className="text-xs text-muted-foreground">by {model.owned_by}</div>
                      )}
                    </button>
                  ))}
                </div>
              )}
            </div>
            <p className="text-xs text-muted-foreground">
              {availableModels.length > 0
                ? `${availableModels.length} model(s) available from server`
                : "Verify connection to inspect models, or leave this empty to use the agent model"}
            </p>
          </div>
        </div>

        <div className="flex items-center justify-end gap-2 pt-4 border-t">
          {(Object.keys(customEndpoint?.settings || {}).length > 0 ||
            customEndpoint?.targetModel) && (
            <button
              type="button"
              onClick={handleClear}
              className="px-4 py-2 text-sm rounded-md border hover:bg-muted transition-colors"
            >
              Clear
            </button>
          )}
          <button
            type="button"
            onClick={() => onOpenChange(false)}
            className="px-4 py-2 text-sm rounded-md border hover:bg-muted transition-colors"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={handleSave}
            disabled={hasEndpointSettings && (!endpointTransport || !hasRequiredSettings)}
            className={cn(
              "px-4 py-2 text-sm rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors",
              hasEndpointSettings &&
                (!endpointTransport || !hasRequiredSettings) &&
                "opacity-50 cursor-not-allowed"
            )}
          >
            Save
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
