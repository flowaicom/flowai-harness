/**
 * Trajectory thread — chat-like display of trajectory steps.
 *
 * Rendering is driven by exhaustive switches on `TrajectoryMatchStatus`
 * and `parseScorerDetails` dispatch for type-safe score detail rendering.
 *
 * - User bubble: right-aligned, primary colors
 * - Tool steps: border cards with status icons
 * - Score card: primary scorer breakdown + progress bar
 * - Metrics card: duration, tokens
 *
 * @module components/eval/trajectory-thread
 */

import { AlertTriangleIcon, CheckCircle2Icon, MinusCircleIcon } from "lucide-react";
import { SectionCard, SectionHeader } from "~/components/shared/section-card";
import type {
  ActionMatchResult,
  ComparisonSummary,
  FinalResponseScoreDetail,
  ParsedScorerDetails,
  SampleResult,
  ScorerResult,
  TrajectoryMatchStatus,
  TrajectoryStepView,
} from "~/lib/domain/eval";
import {
  EVAL_STATUS_COLORS,
  extractSampleScore,
  matchStatusLabel,
  parseScorerDetails,
} from "~/lib/domain/eval";
import { useScramble } from "~/lib/scramble";
import { assertNever, cn, formatNumber } from "~/lib/utils";
import { ActionDiffView } from "./action-diff-view";
import { ActionStatusPill } from "./action-status";

interface TrajectoryThreadProps {
  readonly input: string;
  readonly steps: readonly TrajectoryStepView[];
  readonly sample: SampleResult;
  /** Pass threshold from eval config (default 0.7). */
  readonly passThreshold?: number;
}

// =============================================================================
// Step Icon (exhaustive on TrajectoryMatchStatus)
// =============================================================================

function StepIcon({ status }: { readonly status: TrajectoryMatchStatus }) {
  switch (status) {
    case "matched":
      return <CheckCircle2Icon className="size-4 text-[var(--dot-emerald)] shrink-0" />;
    case "unexpected":
      return <AlertTriangleIcon className="size-4 text-[var(--dot-amber)] shrink-0" />;
    case "missing":
      return <MinusCircleIcon className="size-4 text-muted-foreground/50 shrink-0" />;
    default:
      return assertNever(status);
  }
}

// =============================================================================
// Score Detail (exhaustive dispatch on ParsedScorerDetails)
// =============================================================================

function ScoreDetail({ scorer }: { readonly scorer: ScorerResult }) {
  const parsed = parseScorerDetails(scorer);
  switch (parsed.kind) {
    case "trajectory":
      return <TrajectoryScoreDetail parsed={parsed} />;
    case "actionMatch":
      return <ActionMatchDetail actionMatch={parsed.actionMatch} />;
    case "finalResponse":
      return <FinalResponseDetail result={parsed.result} />;
    case "generic":
      return <GenericScoreDetail parsed={parsed} />;
    default:
      return assertNever(parsed);
  }
}

function formatDetailValue(value: unknown): string {
  if (typeof value === "number") {
    return Number.isInteger(value) ? String(value) : value.toFixed(3);
  }
  if (typeof value === "string" || typeof value === "boolean") return String(value);
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

/** Generic renderer for any scorer: name + score + raw details (key/value). */
function GenericScoreDetail({
  parsed,
}: {
  readonly parsed: Extract<ParsedScorerDetails, { kind: "generic" }>;
}) {
  const entries = parsed.details
    ? Object.entries(parsed.details).filter(([, v]) => v !== null && v !== undefined)
    : [];
  return (
    <div className="min-w-0 text-sm space-y-0.5">
      <div>
        <span className="capitalize">{parsed.scorerName || "Score"}</span>:{" "}
        <span className="font-mono">{parsed.score.toFixed(3)}</span>
      </div>
      {entries.length > 0 && (
        <div className="text-xs text-muted-foreground space-y-0.5">
          {entries.map(([key, value]) => (
            <div key={key} className="font-mono break-all whitespace-pre-wrap">
              {key}: {formatDetailValue(value)}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function TrajectoryScoreDetail({
  parsed,
}: {
  readonly parsed: Extract<ParsedScorerDetails, { kind: "trajectory" }>;
}) {
  const entries = parsed.details
    ? Object.entries(parsed.details).filter(([, v]) => v !== null && v !== undefined)
    : [];
  return (
    <div className="min-w-0 text-sm space-y-0.5">
      <div>
        Trajectory: <span className="font-mono">{parsed.score.toFixed(3)}</span>
      </div>
      {entries.length > 0 && (
        <div className="text-xs text-muted-foreground space-y-0.5">
          {entries.map(([key, value]) => (
            <div key={key} className="font-mono break-all whitespace-pre-wrap">
              {key}: {formatDetailValue(value)}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function FinalResponseDetail({ result }: { readonly result: FinalResponseScoreDetail }) {
  const responseScorers = result.responseScorers ?? [];
  const rawEntries = result.details
    ? Object.entries(result.details).filter(
        ([key]) =>
          ![
            "passed",
            "score",
            "effectiveScore",
            "passThreshold",
            "passedWeight",
            "totalWeight",
            "requiredFailed",
            "responseScorers",
          ].includes(key)
      )
    : [];
  return (
    <div className="min-w-0 text-sm space-y-2">
      <div className="flex items-center gap-2">
        <span>Final response:</span>
        {typeof result.passed === "boolean" && (
          <span
            className={cn(
              "px-1.5 py-0.5 rounded text-xs font-mono",
              result.passed
                ? "bg-[var(--accent-emerald)] text-[var(--dot-emerald)]"
                : "bg-[var(--accent-red)] text-[var(--dot-red)]"
            )}
          >
            {result.passed ? "PASS" : "FAIL"}
          </span>
        )}
        {typeof result.score === "number" && (
          <span className="font-mono text-xs">{Math.round(result.score * 100)}%</span>
        )}
      </div>
      {result.reason && <div className="text-xs text-muted-foreground">{result.reason}</div>}
      {result.requiredFailed && result.requiredFailed.length > 0 && (
        <div className="text-xs text-[var(--dot-red)]">
          Required failed: {result.requiredFailed.join(", ")}
        </div>
      )}
      {responseScorers.length > 0 && (
        <div className="space-y-1.5">
          {responseScorers.map((scorer, index) => (
            <div
              key={`${scorer.id ?? "scorer"}-${index}`}
              className="rounded-md border bg-muted/20 px-2 py-1.5 text-xs space-y-1"
            >
              <div className="flex items-center gap-2">
                <span className="font-mono">{scorer.id ?? `scorer-${index + 1}`}</span>
                {scorer.method && (
                  <span className="rounded bg-muted px-1 py-px uppercase text-[9px] text-muted-foreground">
                    {scorer.method}
                  </span>
                )}
                {typeof scorer.passed === "boolean" && (
                  <span
                    className={cn(
                      "ml-auto font-mono",
                      scorer.passed ? "text-[var(--dot-emerald)]" : "text-[var(--dot-red)]"
                    )}
                  >
                    {scorer.passed ? "pass" : "fail"}
                  </span>
                )}
              </div>
              {scorer.reason && <div className="text-muted-foreground">{scorer.reason}</div>}
              {scorer.details && Object.keys(scorer.details).length > 0 && (
                <div className="font-mono text-muted-foreground space-y-0.5">
                  {Object.entries(scorer.details).map(([key, value]) => (
                    <div key={key} className="break-all whitespace-pre-wrap">
                      {key}: {formatDetailValue(value)}
                    </div>
                  ))}
                </div>
              )}
            </div>
          ))}
        </div>
      )}
      {rawEntries.length > 0 && (
        <div className="font-mono text-xs text-muted-foreground space-y-0.5">
          {rawEntries.map(([key, value]) => (
            <div key={key} className="break-all whitespace-pre-wrap">
              {key}: {formatDetailValue(value)}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// =============================================================================
// Action Match Detail (exhaustive on ActionStatus)
// =============================================================================

function summaryNumber(
  summary: ComparisonSummary,
  legacyKey: keyof ComparisonSummary,
  modernKey: string,
  fallback: number
): number {
  const record = summary as unknown as Record<string, unknown>;
  const legacyValue = record[legacyKey];
  if (typeof legacyValue === "number") return legacyValue;
  const modernValue = record[modernKey];
  if (typeof modernValue === "number") return modernValue;
  return fallback;
}

function actionPassRate(actionMatch: ActionMatchResult): number {
  const record = actionMatch.summary as unknown as Record<string, unknown>;
  if (typeof record.passRate === "number") return record.passRate;
  const total = summaryNumber(
    actionMatch.summary,
    "total",
    "totalExpected",
    actionMatch.actions.length
  );
  const exact = summaryNumber(
    actionMatch.summary,
    "exact",
    "exactCount",
    actionMatch.actions.filter((action) => action.status === "exact").length
  );
  return total === 0 ? 1 : exact / total;
}

function ActionMatchDetail({ actionMatch }: { readonly actionMatch: ActionMatchResult }) {
  const { summary } = actionMatch;
  const score = actionPassRate(actionMatch);
  const exact = summaryNumber(
    summary,
    "exact",
    "exactCount",
    actionMatch.actions.filter((action) => action.status === "exact").length
  );
  const productsWrong = summaryNumber(summary, "productsWrong", "productsWrongCount", 0);
  const scopeWrong = summaryNumber(summary, "scopeWrong", "scopeWrongCount", 0);
  const bothWrong = summaryNumber(summary, "bothWrong", "bothWrongCount", 0);
  const missing = summaryNumber(
    summary,
    "missing",
    "missingCount",
    actionMatch.actions.filter((action) => action.status === "missing").length
  );
  const extra = summaryNumber(
    summary,
    "extra",
    "extraCount",
    actionMatch.actions.filter((action) => action.status === "extra").length
  );

  return (
    <div className="text-sm space-y-2">
      {/* Summary line */}
      <div className="flex items-center gap-2 flex-wrap">
        <span className="font-mono">{(score * 100).toFixed(0)}%</span>
        <span className={actionMatch.pass ? "text-[var(--dot-emerald)]" : "text-[var(--dot-red)]"}>
          ({actionMatch.pass ? "PASS" : "FAIL"})
        </span>
        {exact > 0 && <ActionStatusPill status="exact" />}
        {productsWrong > 0 && <ActionStatusPill status="products_wrong" />}
        {scopeWrong > 0 && <ActionStatusPill status="scope_wrong" />}
        {bothWrong > 0 && <ActionStatusPill status="both_wrong" />}
        {missing > 0 && <ActionStatusPill status="missing" />}
        {extra > 0 && <ActionStatusPill status="extra" />}
      </div>

      {/* Expandable diff cards */}
      <ActionDiffView actionMatch={actionMatch} />
    </div>
  );
}

// =============================================================================
// Score Breakdown
// =============================================================================

function ScoreBreakdown({
  sample,
  passThreshold = 0.7,
}: {
  readonly sample: SampleResult;
  readonly passThreshold?: number;
}) {
  const { scores } = sample;
  if (scores.length === 0) return null;

  const score = extractSampleScore(sample);
  const pct = Math.round(score * 100);

  return (
    <SectionCard className="overflow-hidden">
      <SectionHeader>Score</SectionHeader>
      <div className="flex min-w-0 items-start gap-3">
        <div className="min-w-0 flex-1 max-h-72 overflow-auto scroll-container pr-1 space-y-3">
          {scores.map((scorer, index) => (
            <ScoreDetail key={`${scorer.scorerName}-${index}`} scorer={scorer} />
          ))}
        </div>
        <div className="w-24 shrink-0">
          <div className="flex items-center gap-2">
            <div className="flex-1 h-2 rounded-full bg-muted overflow-hidden">
              <div
                className="h-full rounded-full transition-all"
                style={{
                  width: `${pct}%`,
                  backgroundColor:
                    pct >= passThreshold * 100
                      ? EVAL_STATUS_COLORS.completed
                      : EVAL_STATUS_COLORS.failed,
                }}
              />
            </div>
            <span className="text-xs font-mono tabular-nums">{pct}%</span>
          </div>
        </div>
      </div>
    </SectionCard>
  );
}

// =============================================================================
// Metrics Card
// =============================================================================

function MetricsCard({ sample }: { readonly sample: SampleResult }) {
  const totalTokens = sample.tokenUsage.inputTokens + sample.tokenUsage.outputTokens;
  const retryCount = sample.retryCount ?? 0;

  return (
    <SectionCard>
      <SectionHeader>Metrics</SectionHeader>
      <div className="flex items-center gap-4 text-sm flex-wrap">
        <span className="font-mono tabular-nums">{(sample.durationMs / 1000).toFixed(1)}s</span>
        <span className="text-muted-foreground">|</span>
        <span className="font-mono tabular-nums">{formatNumber(totalTokens)} tok</span>
        {retryCount > 0 && (
          <>
            <span className="text-muted-foreground">|</span>
            <span className="text-[var(--dot-amber)] font-medium">{retryCount} retries</span>
          </>
        )}
      </div>
    </SectionCard>
  );
}

// =============================================================================
// Main Component
// =============================================================================

export function TrajectoryThread({
  input,
  steps,
  sample,
  passThreshold = 0.7,
}: TrajectoryThreadProps) {
  const { s } = useScramble();
  return (
    <div className="space-y-3">
      {/* User message bubble */}
      <div className="flex justify-end">
        <div className="bg-primary text-primary-foreground rounded-2xl px-4 py-2 max-w-[80%]">
          <div className="text-sm whitespace-pre-wrap">{s(input)}</div>
        </div>
      </div>

      {/* Trajectory steps */}
      {steps.map((step) => (
        <div
          key={step.index}
          className={cn(
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
            className={cn("font-mono text-xs", step.matchStatus === "missing" && "line-through")}
          >
            {step.toolName}
          </span>
          <span className="text-muted-foreground text-xs ml-auto">
            {matchStatusLabel(step.matchStatus)}
          </span>
          {step.matchStatus !== "missing" && (
            <span className="text-muted-foreground text-xs">#{step.index + 1}</span>
          )}
        </div>
      ))}

      {/* Error banner */}
      {sample.error && (
        <div className="rounded-md border border-destructive/30 bg-[var(--accent-red)] px-4 py-2 text-sm text-[var(--dot-red)] font-mono accent-bar-red">
          {s(sample.error)}
        </div>
      )}

      {/* Score breakdown */}
      <ScoreBreakdown sample={sample} passThreshold={passThreshold} />

      {/* Metrics */}
      <MetricsCard sample={sample} />
    </div>
  );
}
