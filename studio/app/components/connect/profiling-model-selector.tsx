/**
 * Model selector + cost estimate for profiling.
 *
 * Shared between the profiling page and import page.
 * Shows a model dropdown and live cost estimate based on table/column counts.
 *
 * @module components/data/profiling-model-selector
 */

import { ChevronDownIcon, CoinsIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ProfilingCostEstimate } from "~/lib/domain/data";
import {
  selectAvailableModels,
  selectLoadProviderModels,
  selectProviderModels,
  selectProviderModelsError,
  useAgentConfig,
} from "~/lib/stores";
import { cn } from "~/lib/utils";

interface ProfilingModelSelectorProps {
  /** Number of tables to profile. */
  tableCount: number;
  /** Total columns across all tables. */
  totalColumns: number;
  /** Currently selected model ID. */
  selectedModelId: string | undefined;
  /** Callback when model changes. */
  onModelChange: (modelId: string | undefined) => void;
  /** Disable interaction (e.g. while profiling is running). */
  disabled?: boolean;
}

/** Format USD cost with appropriate precision. */
function formatCost(usd: number): string {
  if (usd === 0) return "$0.00";
  if (usd < 0.01) return `$${usd.toFixed(4)}`;
  if (usd < 1) return `$${usd.toFixed(3)}`;
  return `$${usd.toFixed(2)}`;
}

/** Format token count with thousands separators. */
function formatTokens(n: number): string {
  return n.toLocaleString();
}

export function ProfilingModelSelector({
  tableCount,
  totalColumns,
  selectedModelId,
  onModelChange,
  disabled,
}: ProfilingModelSelectorProps) {
  const availableModels = useAgentConfig(selectAvailableModels);
  const providerModels = useAgentConfig(selectProviderModels);
  const providerModelsError = useAgentConfig(selectProviderModelsError);
  const loadProviderModels = useAgentConfig(selectLoadProviderModels);

  const [estimate, setEstimate] = useState<ProfilingCostEstimate | null>(null);
  const [estimateError, setEstimateError] = useState<string | null>(null);
  const [isOpen, setIsOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  // Load provider models on mount if not cached
  useEffect(() => {
    if (providerModels.length === 0) {
      loadProviderModels(undefined, false, undefined);
    }
  }, [providerModels.length, loadProviderModels]);

  // Models with pricing, grouped by provider
  const modelsWithPricing = useMemo(
    () => providerModels.filter((m) => m.pricing && m.pricing.inputPerMTok >= 0),
    [providerModels]
  );

  // Currently selected model info
  const selectedModel = useMemo(
    () => modelsWithPricing.find((m) => m.id === selectedModelId),
    [modelsWithPricing, selectedModelId]
  );

  // Fetch estimate when inputs change
  const debounceRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => {
    if (tableCount === 0) {
      setEstimate(null);
      setEstimateError(null);
      return;
    }
    clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(async () => {
      const model = selectedModel ?? modelsWithPricing[0];
      const estimatedInputTokens = tableCount * 1200 + totalColumns * 250;
      const estimatedOutputTokens = tableCount * 300 + totalColumns * 60;
      const inputPerMTok = model?.pricing?.inputPerMTok ?? 0;
      const outputPerMTok = model?.pricing?.outputPerMTok ?? 0;
      setEstimate({
        estimatedInputTokens,
        estimatedOutputTokens,
        estimatedCachedTokens: 0,
        estimatedCostUsd:
          (estimatedInputTokens / 1_000_000) * inputPerMTok +
          (estimatedOutputTokens / 1_000_000) * outputPerMTok,
        modelId: selectedModelId ?? model?.id ?? "default",
        modelDisplayName: model?.displayName ?? "Default",
        inputPerMTok,
        outputPerMTok,
      });
      setEstimateError(null);
    }, 200);
    return () => clearTimeout(debounceRef.current);
  }, [modelsWithPricing, selectedModel, tableCount, totalColumns, selectedModelId]);

  // Close dropdown on outside click
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    if (isOpen) document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [isOpen]);

  const handleSelect = useCallback(
    (modelId: string) => {
      onModelChange(modelId);
      setIsOpen(false);
    },
    [onModelChange]
  );

  // Group models by provider for the dropdown
  const grouped = useMemo(() => {
    const map = new Map<string, typeof modelsWithPricing>();
    for (const m of modelsWithPricing) {
      const list = map.get(m.provider) ?? [];
      list.push(m);
      map.set(m.provider, list);
    }
    return map;
  }, [modelsWithPricing]);

  return (
    <div className="flex items-center gap-4 flex-wrap">
      {/* Model selector */}
      <div className="relative" ref={dropdownRef}>
        <button
          type="button"
          onClick={() => !disabled && setIsOpen(!isOpen)}
          disabled={disabled}
          className="flex items-center gap-2 px-3 py-1.5 text-xs border rounded-md hover:bg-muted transition-colors disabled:opacity-50 min-w-[200px]"
        >
          <span className="text-muted-foreground">Model:</span>
          <span className="font-medium truncate">
            {selectedModel?.displayName ?? estimate?.modelDisplayName ?? "Default"}
          </span>
          <ChevronDownIcon className="size-3.5 ml-auto shrink-0 text-muted-foreground" />
        </button>

        {isOpen && (
          <div className="absolute top-full left-0 mt-1 w-[320px] bg-popover border rounded-md shadow-lg z-50 max-h-[300px] overflow-y-auto scroll-container">
            {/* Default option */}
            <button
              type="button"
              onClick={() => handleSelect("")}
              className="w-full text-left px-3 py-2 text-xs hover:bg-muted transition-colors border-b"
            >
              <span className="font-medium">Server Default</span>
              <span className="text-muted-foreground ml-2">(uses configured model)</span>
            </button>

            {[...grouped.entries()].map(([provider, models]) => (
              <div key={provider}>
                <div className="px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground bg-muted/50">
                  {availableModels.find((candidate) => candidate.key === provider)?.displayName ??
                    provider}
                </div>
                {models.map((m) => (
                  <button
                    key={m.id}
                    type="button"
                    onClick={() => handleSelect(m.id)}
                    className={cn(
                      "w-full text-left px-3 py-2 text-xs hover:bg-muted transition-colors flex items-center justify-between focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                      m.id === selectedModelId && "bg-primary/5 font-medium"
                    )}
                  >
                    <span className="truncate">{m.displayName}</span>
                    {m.pricing && (
                      <span className="text-muted-foreground tabular-nums shrink-0 ml-2">
                        ${m.pricing.inputPerMTok}/{m.pricing.outputPerMTok}
                      </span>
                    )}
                  </button>
                ))}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Cost estimate */}
      {estimate && tableCount > 0 && (
        <div className="flex items-center gap-3 text-xs">
          <CoinsIcon className="size-3.5 text-[var(--dot-amber)] shrink-0" />
          <span className="text-muted-foreground">
            Est.{" "}
            <span className="font-medium text-foreground tabular-nums">
              {formatCost(estimate.estimatedCostUsd)}
            </span>
          </span>
          <span className="text-muted-foreground tabular-nums">
            {formatTokens(estimate.estimatedInputTokens)} in
            {estimate.estimatedCachedTokens > 0 && (
              <span className="text-[var(--dot-emerald)]">
                {" "}
                ({formatTokens(estimate.estimatedCachedTokens)} cached)
              </span>
            )}
            {" / "}
            {formatTokens(estimate.estimatedOutputTokens)} out
          </span>
          <span className="text-muted-foreground tabular-nums">
            @ ${estimate.inputPerMTok}/{estimate.outputPerMTok}
            {estimate.cacheReadPerMTok != null && `/${estimate.cacheReadPerMTok}`}
            /M tok
          </span>
        </div>
      )}

      {/* Free provider indicator */}
      {estimate && estimate.estimatedCostUsd === 0 && tableCount > 0 && (
        <span className="text-xs text-[var(--dot-emerald)] font-medium">Free</span>
      )}

      {!estimate &&
        tableCount > 0 &&
        modelsWithPricing.length === 0 &&
        !estimateError &&
        !providerModelsError && (
          <span className="text-xs text-muted-foreground">
            No priced models configured. Profiling estimates require current pricing from the
            server.
          </span>
        )}

      {!estimate && estimateError && (
        <span className="text-xs text-destructive">{estimateError}</span>
      )}

      {!estimate && !estimateError && providerModelsError && (
        <span className="text-xs text-destructive">{providerModelsError}</span>
      )}
    </div>
  );
}
