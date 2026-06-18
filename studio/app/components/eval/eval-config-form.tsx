import {
  SharedEvalConfigForm,
  type SharedEvalConfigLike,
  type SharedEvalModeOption,
  type SharedEvalScoreWeightOption,
} from "@studio/features-evals";
import { useMemo } from "react";
import type {
  EvalCapabilityMode,
  EvalConfig,
  EvalMode,
  ScorerKey,
  TestCaseSet,
} from "~/lib/domain/eval";
import type { AuthoredTestCase, TestCaseStatus } from "~/lib/domain/test-case";
import { useScramble } from "~/lib/scramble";

interface EvalConfigFormProps {
  readonly config: EvalConfig;
  readonly evalModes?: readonly EvalCapabilityMode[];
  readonly testCaseSets: readonly TestCaseSet[];
  readonly testCases: readonly AuthoredTestCase[];
  readonly onUpdate: (partial: Partial<EvalConfig>) => void;
  readonly onSubmit: () => void;
  readonly isRunning: boolean;
}

type SharedAppEvalConfig = EvalConfig & SharedEvalConfigLike<EvalMode, ScorerKey>;
type MutableEvalConfigPatch = {
  -readonly [K in keyof EvalConfig]?: EvalConfig[K];
};

const DEFAULT_MODE_OPTIONS: readonly SharedEvalModeOption<EvalMode>[] = [
  { value: "planner", label: "Planner", description: "Score planned actions vs. the stored plan" },
  { value: "executor", label: "Executor", description: "Score executed actions" },
  {
    value: "sequential",
    label: "Sequential",
    description: "Full pipeline — trajectory + planned + executed",
  },
];

const STATUS_ORDER: Record<TestCaseStatus, number> = {
  active: 0,
  draft: 1,
  archived: 2,
};

const SCORER_OPTIONS: readonly Omit<SharedEvalScoreWeightOption<ScorerKey>, "eligible">[] = [
  {
    key: "trajectory",
    label: "Trajectory",
    description: "Expected tool path",
    defaultWeight: 1,
  },
  {
    key: "planned_actions",
    label: "Planned actions",
    description: "Stored plan output",
    defaultWeight: 1,
  },
  {
    key: "executed_actions",
    label: "Executed actions",
    description: "Executed action output",
    defaultWeight: 1,
  },
  {
    key: "final_response",
    label: "Final response",
    description: "Response text scorers",
    defaultWeight: 1,
  },
];

type ScorerEligibility = Record<ScorerKey, boolean>;

const EMPTY_SCORER_ELIGIBILITY: ScorerEligibility = {
  trajectory: false,
  planned_actions: false,
  executed_actions: false,
  final_response: false,
};

export function EvalConfigForm({
  config,
  evalModes,
  testCaseSets,
  testCases,
  onUpdate,
  onSubmit,
  isRunning,
}: EvalConfigFormProps) {
  const { s } = useScramble();
  const modeOptions = useMemo(
    () =>
      evalModes?.map((mode) => ({
        value: mode.mode,
        label: mode.label,
        description: mode.description,
        targetAgentId: mode.targetAgentId ?? null,
      })) ?? DEFAULT_MODE_OPTIONS,
    [evalModes]
  );
  const selectedIds = useMemo(() => new Set(config.testCaseIds ?? []), [config.testCaseIds]);
  const selectedTestCases = useMemo(
    () => testCases.filter((testCase) => selectedIds.has(testCase.id)),
    [selectedIds, testCases]
  );
  const scorerEligibility = useMemo(
    () =>
      selectedTestCases.length > 0
        ? scorerEligibilityFor(selectedTestCases)
        : EMPTY_SCORER_ELIGIBILITY,
    [selectedTestCases]
  );
  const scoreWeightOptions = useMemo(
    () =>
      SCORER_OPTIONS.map((option) => ({
        ...option,
        eligible: scorerEligibility[option.key],
      })),
    [scorerEligibility]
  );

  return (
    <SharedEvalConfigForm<
      EvalMode,
      TestCaseStatus,
      ScorerKey,
      SharedAppEvalConfig,
      AuthoredTestCase,
      TestCaseSet
    >
      config={config}
      modeOptions={modeOptions}
      testCaseSets={testCaseSets}
      testCases={testCases}
      onUpdate={(partial) => onUpdate(fromSharedConfigPatch(partial))}
      onSubmit={onSubmit}
      isRunning={isRunning}
      statusOrder={STATUS_ORDER}
      activeStatus="active"
      archivedStatus="archived"
      showAdvancedScoring={config.mode !== "single"}
      scoreWeightOptions={scoreWeightOptions}
      allowTestCaseSets={false}
      formatText={s}
      emptyTestCasesHref="/tests/new"
    />
  );
}

function fromSharedConfigPatch(partial: Partial<SharedAppEvalConfig>): Partial<EvalConfig> {
  const next: MutableEvalConfigPatch = { ...partial };
  if ("targetAgentId" in partial) {
    next.targetAgentId = partial.targetAgentId ?? null;
  }
  return next;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function structuredPayload(value: unknown): Record<string, unknown> | null {
  const groundTruth = asRecord(value);
  if (!groundTruth) return null;
  const rawPayload =
    groundTruth.kind === "structured" ? (groundTruth.payload ?? groundTruth.data) : groundTruth;
  const payload = asRecord(rawPayload);
  return payload?.kind === "flat" ? payload : null;
}

function hasActionBucket(testCase: AuthoredTestCase, key: "plannedActions" | "executedActions") {
  const payload = structuredPayload(testCase.structuredGroundTruth);
  return Array.isArray(payload?.[key]) && (payload[key] as unknown[]).length > 0;
}

function scorerEligibilityFor(testCases: readonly AuthoredTestCase[]): ScorerEligibility {
  return {
    trajectory: testCases.some((testCase) => testCase.expectedTrajectory.length > 0),
    planned_actions: testCases.some((testCase) => hasActionBucket(testCase, "plannedActions")),
    executed_actions: testCases.some((testCase) => hasActionBucket(testCase, "executedActions")),
    final_response: testCases.some((testCase) => !!testCase.finalResponse),
  };
}
