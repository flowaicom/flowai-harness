import { AlertTriangleIcon, CheckCircle2Icon, MinusCircleIcon } from "lucide-react";
import type { ReactNode } from "react";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat("en-US").format(value);
}

export type SharedTrajectoryMatchStatus = "matched" | "unexpected" | "missing";

export interface SharedTrajectoryStepLike {
  readonly index: number;
  readonly toolName: string;
  readonly matchStatus: SharedTrajectoryMatchStatus;
}

export interface SharedTrajectoryTokenUsageLike {
  readonly inputTokens: number;
  readonly outputTokens: number;
}

export interface SharedTrajectoryLatencyLike {
  readonly totalDurationMs: number;
  readonly ttftMs?: number | null;
  readonly retryCount?: number;
  readonly phases: {
    readonly llmTimeMs: number;
    readonly toolTimeMs: number;
  };
}

export interface SharedTrajectorySampleLike<TScore = unknown> {
  readonly scores: readonly TScore[];
  readonly durationMs: number;
  readonly tokenUsage: SharedTrajectoryTokenUsageLike;
  readonly error: string | null;
  readonly retryCount?: number;
  readonly latency?: SharedTrajectoryLatencyLike | null;
}

export interface SharedTrajectoryThreadProps<
  TScore = unknown,
  TSample extends SharedTrajectorySampleLike<TScore> = SharedTrajectorySampleLike<TScore>,
  TStep extends SharedTrajectoryStepLike = SharedTrajectoryStepLike,
> {
  readonly input: string;
  readonly steps: readonly TStep[];
  readonly sample: TSample;
  readonly passThreshold?: number;
  readonly formatText?: (value: string) => string;
  readonly getSampleScore: (scores: readonly TScore[]) => number;
  readonly renderScoreDetail: (scores: readonly TScore[]) => ReactNode;
}

function SectionCard({ children }: { readonly children: ReactNode }) {
  return <div className="rounded-lg keyline-card p-4">{children}</div>;
}

function SectionHeader({ children }: { readonly children: ReactNode }) {
  return <h3 className="section-label mb-3">{children}</h3>;
}

function StepIcon({ status }: { readonly status: SharedTrajectoryMatchStatus }) {
  switch (status) {
    case "matched":
      return <CheckCircle2Icon className="size-4 text-[var(--dot-emerald)] shrink-0" />;
    case "unexpected":
      return <AlertTriangleIcon className="size-4 text-[var(--dot-amber)] shrink-0" />;
    case "missing":
      return <MinusCircleIcon className="size-4 text-muted-foreground/50 shrink-0" />;
  }
}

function matchStatusLabel(status: SharedTrajectoryMatchStatus): string {
  switch (status) {
    case "matched":
      return "matched";
    case "unexpected":
      return "unexpected";
    case "missing":
      return "missing";
  }
}

function ScoreBreakdown<TScore>({
  scores,
  passThreshold = 0.7,
  getSampleScore,
  renderScoreDetail,
}: {
  readonly scores: readonly TScore[];
  readonly passThreshold?: number;
  readonly getSampleScore: (scores: readonly TScore[]) => number;
  readonly renderScoreDetail: (scores: readonly TScore[]) => ReactNode;
}) {
  if (scores.length === 0) {
    return null;
  }

  const score = getSampleScore(scores);
  const percentage = Math.round(score * 100);

  return (
    <SectionCard>
      <SectionHeader>Score</SectionHeader>
      <div className="flex items-center gap-3">
        <div className="flex-1">{renderScoreDetail(scores)}</div>
        <div className="w-24">
          <div className="flex items-center gap-2">
            <div className="flex-1 h-2 rounded-full bg-muted overflow-hidden">
              <div
                className="h-full rounded-full transition-all"
                style={{
                  width: `${percentage}%`,
                  backgroundColor:
                    percentage >= passThreshold * 100 ? "var(--dot-emerald)" : "var(--dot-red)",
                }}
              />
            </div>
            <span className="text-xs font-mono tabular-nums">{percentage}%</span>
          </div>
        </div>
      </div>
    </SectionCard>
  );
}

function MetricsCard<TScore, TSample extends SharedTrajectorySampleLike<TScore>>({
  sample,
}: {
  readonly sample: TSample;
}) {
  const totalTokens = sample.tokenUsage.inputTokens + sample.tokenUsage.outputTokens;
  const latency = sample.latency;
  const retryCount = sample.retryCount ?? latency?.retryCount ?? 0;

  return (
    <SectionCard>
      <SectionHeader>Metrics</SectionHeader>
      <div className="flex items-center gap-4 text-sm flex-wrap">
        <span className="font-mono tabular-nums">{(sample.durationMs / 1000).toFixed(1)}s</span>
        <span className="text-muted-foreground">|</span>
        <span className="font-mono tabular-nums">{formatNumber(totalTokens)} tok</span>
        {latency?.ttftMs != null ? (
          <>
            <span className="text-muted-foreground">|</span>
            <span className="font-mono tabular-nums text-muted-foreground">
              TTFT {latency.ttftMs}ms
            </span>
          </>
        ) : null}
        {retryCount > 0 ? (
          <>
            <span className="text-muted-foreground">|</span>
            <span className="text-[var(--dot-amber)] font-medium">{retryCount} retries</span>
          </>
        ) : null}
        {latency && latency.totalDurationMs > 0 ? (
          <>
            <span className="text-muted-foreground">|</span>
            <span className="flex items-center gap-1">
              <span className="status-dot bg-[var(--dot-blue)]" />
              LLM {Math.round((latency.phases.llmTimeMs / latency.totalDurationMs) * 100)}%
            </span>
            <span className="flex items-center gap-1">
              <span className="status-dot bg-[var(--dot-amber)]" />
              Tools {Math.round((latency.phases.toolTimeMs / latency.totalDurationMs) * 100)}%
            </span>
            {(() => {
              const overheadPercentage = Math.round(
                Math.max(
                  0,
                  100 -
                    (latency.phases.llmTimeMs / latency.totalDurationMs) * 100 -
                    (latency.phases.toolTimeMs / latency.totalDurationMs) * 100
                )
              );
              return overheadPercentage > 1 ? (
                <span className="flex items-center gap-1">
                  <span className="status-dot bg-muted-foreground/20" />
                  Overhead {overheadPercentage}%
                </span>
              ) : null;
            })()}
          </>
        ) : null}
      </div>
    </SectionCard>
  );
}

export function SharedTrajectoryThread<
  TScore = unknown,
  TSample extends SharedTrajectorySampleLike<TScore> = SharedTrajectorySampleLike<TScore>,
  TStep extends SharedTrajectoryStepLike = SharedTrajectoryStepLike,
>({
  input,
  steps,
  sample,
  passThreshold = 0.7,
  formatText = (value) => value,
  getSampleScore,
  renderScoreDetail,
}: SharedTrajectoryThreadProps<TScore, TSample, TStep>) {
  return (
    <div className="space-y-3">
      <div className="flex justify-end">
        <div className="bg-primary text-primary-foreground rounded-2xl px-4 py-2 max-w-[80%]">
          <div className="text-sm whitespace-pre-wrap">{formatText(input)}</div>
        </div>
      </div>

      {steps.map((step) => (
        <div
          key={step.index}
          className={cx(
            "flex items-center gap-3 rounded-md border border-l-2 px-4 py-2.5 text-sm transition-colors",
            step.matchStatus === "matched" && "border-l-[var(--dot-emerald)] border-border",
            step.matchStatus === "unexpected" &&
              "border-l-[var(--dot-amber)] bg-[var(--accent-amber)]",
            step.matchStatus === "missing" &&
              "border-l-muted-foreground/30 border-dashed opacity-50"
          )}
        >
          <StepIcon status={step.matchStatus} />
          <span
            className={cx("font-mono text-xs", step.matchStatus === "missing" && "line-through")}
          >
            {step.toolName}
          </span>
          <span className="text-muted-foreground text-xs ml-auto">
            {matchStatusLabel(step.matchStatus)}
          </span>
          {step.matchStatus !== "missing" ? (
            <span className="text-muted-foreground text-xs">#{step.index + 1}</span>
          ) : null}
        </div>
      ))}

      {sample.error ? (
        <div className="rounded-md border border-red-500/30 bg-[var(--accent-red)] px-4 py-2 text-sm text-red-600 dark:text-red-400 font-mono accent-bar-red">
          {formatText(sample.error)}
        </div>
      ) : null}

      <ScoreBreakdown
        scores={sample.scores}
        passThreshold={passThreshold}
        getSampleScore={getSampleScore}
        renderScoreDetail={renderScoreDetail}
      />

      <MetricsCard sample={sample} />
    </div>
  );
}
