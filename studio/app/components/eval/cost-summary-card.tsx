/**
 * Token usage card for eval runs.
 *
 * Shows token breakdown (input, output, total) with cache percentage.
 *
 * @module components/eval/cost-summary-card
 */

import { SectionCard, SectionHeader } from "~/components/shared/section-card";
import type { TokenUsageSummary } from "~/lib/domain/eval";
import { formatNumber } from "~/lib/utils";

interface TokenUsageCardProps {
  usage: TokenUsageSummary;
  className?: string;
}

export function TokenUsageCard({ usage, className }: TokenUsageCardProps) {
  const total = usage.inputTokens + usage.outputTokens;
  const cacheHitPercent = ((usage.cachedTokens / Math.max(usage.inputTokens, 1)) * 100).toFixed(2);
  return (
    <SectionCard className={className}>
      <SectionHeader>Token Usage</SectionHeader>
      <div className="grid grid-cols-3 gap-2 text-xs">
        <div>
          <div className="text-muted-foreground">Input</div>
          <div className="font-mono">{formatNumber(usage.inputTokens)}</div>
        </div>
        <div>
          <div className="text-muted-foreground">Output</div>
          <div className="font-mono">{formatNumber(usage.outputTokens)}</div>
        </div>
        <div>
          <div className="text-muted-foreground">Total</div>
          <div className="font-mono">{formatNumber(total)}</div>
        </div>
      </div>
      {usage.cachedTokens > 0 && (
        <div className="text-xs text-[var(--dot-emerald)]">{cacheHitPercent}% cached</div>
      )}
      {usage.cacheCreationTokens > 0 && (
        <div className="text-xs text-[var(--dot-blue)]">
          {formatNumber(usage.cacheCreationTokens)} cache-write tokens
        </div>
      )}
    </SectionCard>
  );
}
