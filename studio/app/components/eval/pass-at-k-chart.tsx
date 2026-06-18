/**
 * Pass@K horizontal bar chart (CSS-only, no chart library).
 *
 * @module components/eval/pass-at-k-chart
 */

import type { PassAtKResult } from "~/lib/domain/eval";
import { cn } from "~/lib/utils";

interface PassAtKChartProps {
  results: readonly PassAtKResult[];
  threshold?: number;
  className?: string;
}

export function PassAtKChart({ results, threshold = 0.7, className }: PassAtKChartProps) {
  if (results.length === 0) {
    return <div className="text-sm text-muted-foreground">No pass@k data</div>;
  }

  return (
    <div className={cn("space-y-2", className)}>
      <h4 className="section-label">Pass@K</h4>
      {results.map((r) => {
        const pct = Math.round(r.simpleEstimate * 100);
        const meetsThreshold = r.simpleEstimate >= threshold;

        return (
          <div key={r.k} className="flex items-center gap-3">
            <span className="text-xs font-mono w-12 text-right text-muted-foreground">k={r.k}</span>
            <div className="flex-1 h-5 bg-muted rounded-sm relative overflow-hidden">
              {/* Bar fill */}
              <div
                className={cn(
                  "h-full rounded-sm transition-all duration-500",
                  meetsThreshold ? "bg-[var(--dot-emerald)]" : "bg-[var(--dot-amber)]"
                )}
                style={{ width: `${pct}%` }}
              />
              {/* Threshold line */}
              <div
                className="absolute top-0 bottom-0 w-px bg-foreground/30"
                style={{ left: `${threshold * 100}%` }}
              />
            </div>
            <span className="text-xs font-mono w-12 text-right">{pct}%</span>
          </div>
        );
      })}
      <div className="flex items-center gap-1 text-xs text-muted-foreground">
        <div className="w-3 h-px bg-foreground/30" />
        <span>Threshold: {Math.round(threshold * 100)}%</span>
      </div>
    </div>
  );
}
