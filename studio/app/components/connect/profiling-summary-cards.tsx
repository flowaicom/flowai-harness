/**
 * Profiling summary cards.
 *
 * Post-completion 2x3 grid showing key metrics via shared MetricCell.
 * Includes enrichment quality breakdown: fresh, cached, and fallback counts.
 *
 * Law L1: All content inside SectionCard.
 * Law L5: All numbers use tabular-nums (MetricCell handles this).
 *
 * @module components/data/profiling-summary-cards
 */

import { AlertTriangleIcon, SparklesIcon, ZapIcon } from "lucide-react";
import { MetricCell } from "~/components/shared/metric-cell";
import { SectionCard, SectionHeader } from "~/components/shared/section-card";
import type { IngestionSummary } from "~/lib/domain/data";
import { formatDuration } from "~/lib/utils";

interface ProfilingSummaryCardsProps {
  summary: IngestionSummary;
}

export function ProfilingSummaryCards({ summary }: ProfilingSummaryCardsProps) {
  const fresh = summary.enrichmentFresh ?? 0;
  const cacheHits = summary.enrichmentCacheHits ?? 0;
  const fallbacks = summary.enrichmentFallbacks ?? 0;
  const hasBreakdown = fresh > 0 || cacheHits > 0 || fallbacks > 0;

  return (
    <SectionCard>
      <SectionHeader>Summary</SectionHeader>
      <div className="grid grid-cols-3 gap-3">
        <MetricCell value={summary.tablesDiscovered.toLocaleString()} label="Tables" />
        <MetricCell value={summary.columnsProfiled.toLocaleString()} label="Columns" />
        <MetricCell value={summary.enumsExtracted.toLocaleString()} label="Enums" />
        <MetricCell value={summary.relationshipsFound.toLocaleString()} label="Relations" />
        <MetricCell value={summary.catalogItemsIndexed.toLocaleString()} label="Catalog Items" />
        <MetricCell value={formatDuration(summary.durationMs)} label="Duration" />
      </div>

      {/* Enrichment quality breakdown */}
      {hasBreakdown && (
        <div className="mt-3 pt-3 border-t flex items-center gap-4 text-xs">
          {fresh > 0 && (
            <span className="flex items-center gap-1.5 text-[var(--dot-blue)]">
              <SparklesIcon className="size-3" />
              {fresh} fresh
            </span>
          )}
          {cacheHits > 0 && (
            <span className="flex items-center gap-1.5 text-[var(--dot-emerald)]">
              <ZapIcon className="size-3" />
              {cacheHits} cached
            </span>
          )}
          {fallbacks > 0 && (
            <span className="flex items-center gap-1.5 text-[var(--dot-amber)]">
              <AlertTriangleIcon className="size-3" />
              {fallbacks} fallback
            </span>
          )}
        </div>
      )}
    </SectionCard>
  );
}
