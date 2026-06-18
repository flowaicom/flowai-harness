import type { SharedGroundTruth, TestBuilderSessionLike } from "./domain";

export type BuilderWorkflowStep = "describe" | "compose" | "groundTruth" | "review";

export interface BuilderWorkflowStepDef {
  readonly key: BuilderWorkflowStep;
  readonly label: string;
  readonly description: string;
}

export const TEST_BUILDER_WORKFLOW_STEPS: readonly BuilderWorkflowStepDef[] = [
  { key: "describe", label: "Describe", description: "Describe the scenario" },
  { key: "compose", label: "Trajectory", description: "Tool call sequence" },
  { key: "groundTruth", label: "Ground Truth", description: "Expected outcome" },
  { key: "review", label: "Save", description: "Review & save" },
];

const BUILDER_STEP_ORDER: readonly BuilderWorkflowStep[] = [
  "describe",
  "compose",
  "groundTruth",
  "review",
];

export function deriveBuilderWorkflowStep(
  hasMessages: boolean,
  session: TestBuilderSessionLike | null,
  options: { readonly requireUserPromptForReview?: boolean } = {}
): BuilderWorkflowStep {
  if (!hasMessages) return "describe";
  if (!session || session.composedTrajectory.length === 0) return "compose";
  if (!session.structuredGroundTruth && !session.groundTruth) return "groundTruth";
  if (options.requireUserPromptForReview && !session.userPrompt) return "compose";
  return "review";
}

export function isBuilderWorkflowStepComplete(
  step: BuilderWorkflowStep,
  currentStep: BuilderWorkflowStep
): boolean {
  return BUILDER_STEP_ORDER.indexOf(step) < BUILDER_STEP_ORDER.indexOf(currentStep);
}

export function summarizeBuilderGroundTruth(gt: SharedGroundTruth): string {
  switch (gt.kind) {
    case "textOnly":
      return `Text: "${gt.text.slice(0, 60)}${gt.text.length > 60 ? "..." : ""}"`;
    case "flat": {
      const actionCount = gt.expectedActions.length;
      return `${actionCount} action${actionCount !== 1 ? "s" : ""} (structured)`;
    }
    case "multiGroup": {
      const groupCount = gt.groups.length;
      const actionCount = gt.groups.reduce((sum, group) => sum + group.actions.length, 0);
      return `${groupCount} group${groupCount !== 1 ? "s" : ""}, ${actionCount} action${actionCount !== 1 ? "s" : ""}`;
    }
  }
}
