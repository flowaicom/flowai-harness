/**
 * Sample tab bar — pill tabs for switching between eval samples.
 *
 * Visual language:
 * - Samples: colored dot (green/red) + score % — primary accent when active
 *
 * @module components/eval/sample-tab-bar
 */

import type { SampleResult } from "~/lib/domain/eval";
import { EVAL_STATUS_COLORS, extractSampleScore } from "~/lib/domain/eval";
import { cn } from "~/lib/utils";

interface SampleTabBarProps {
  readonly samples: readonly SampleResult[];
  readonly selectedIndex: number;
  readonly onSelect: (index: number) => void;
  readonly onCreateTestCase?: () => void;
}

export function SampleTabBar({
  samples,
  selectedIndex,
  onSelect,
  onCreateTestCase,
}: SampleTabBarProps) {
  return (
    <div className="flex items-center gap-1 overflow-x-auto pb-1">
      {/* Sample tabs */}
      {samples.map((sample) => {
        const score = extractSampleScore(sample);
        const isActive = selectedIndex === sample.sampleIndex;

        return (
          <button
            key={sample.sampleIndex}
            type="button"
            onClick={() => onSelect(sample.sampleIndex)}
            className={cn(
              "flex items-center gap-1.5 px-3 py-1.5 rounded-full text-xs font-medium transition-colors whitespace-nowrap",
              isActive
                ? "bg-primary text-primary-foreground"
                : "bg-muted/60 text-muted-foreground hover:bg-muted"
            )}
          >
            <span
              className="w-2 h-2 rounded-full shrink-0"
              style={{
                backgroundColor: sample.passed
                  ? EVAL_STATUS_COLORS.completed
                  : EVAL_STATUS_COLORS.failed,
              }}
            />
            Sample {sample.sampleIndex + 1}
            <span className="tabular-nums">{Math.round(score * 100)}%</span>
          </button>
        );
      })}

      {/* Action buttons */}
      {onCreateTestCase && (
        <>
          <div className="w-px h-5 bg-border mx-1" />
          <button
            type="button"
            onClick={onCreateTestCase}
            className="flex items-center gap-1 px-3 py-1.5 rounded-full text-xs font-medium text-muted-foreground hover:bg-muted transition-colors whitespace-nowrap border border-dashed border-muted-foreground/30"
          >
            + Create Test
          </button>
        </>
      )}
    </div>
  );
}
