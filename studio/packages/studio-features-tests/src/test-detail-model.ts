import { deepEqual } from "./deep-equal";
import type { SharedGroundTruth, SharedTestCaseStatus, SharedTrajectoryMode } from "./domain";

export interface TestDetailCaseLike<
  TGroundTruth extends SharedGroundTruth | null = SharedGroundTruth | null,
  TStatus extends string = SharedTestCaseStatus,
  TMode extends string = SharedTrajectoryMode,
> {
  readonly name?: string | null;
  readonly description?: string | null;
  readonly input: string;
  readonly status: TStatus;
  readonly expectedTrajectory: readonly string[];
  readonly trajectoryMode: TMode;
  readonly groundTruth: string | null;
  readonly structuredGroundTruth: TGroundTruth;
  readonly tags: readonly string[];
}

export interface TestDetailFormSnapshot<
  TGroundTruth extends SharedGroundTruth | null = SharedGroundTruth | null,
  TStatus extends string = SharedTestCaseStatus,
  TMode extends string = SharedTrajectoryMode,
> {
  readonly name: string;
  readonly description: string;
  readonly input: string;
  readonly trajectory: string;
  readonly mode: TMode;
  readonly groundTruth: string;
  readonly structuredGroundTruth: TGroundTruth;
  readonly tags: string;
  readonly status: TStatus;
}

export const EMPTY_TEST_DETAIL_FORM_SNAPSHOT: TestDetailFormSnapshot<null> = {
  name: "",
  description: "",
  input: "",
  trajectory: "",
  mode: "unordered",
  groundTruth: "",
  structuredGroundTruth: null,
  tags: "",
  status: "draft",
};

export function createTestDetailSnapshot<
  TGroundTruth extends SharedGroundTruth | null,
  TStatus extends string,
  TMode extends string,
>(
  testCase: TestDetailCaseLike<TGroundTruth, TStatus, TMode>
): TestDetailFormSnapshot<TGroundTruth, TStatus, TMode> {
  return {
    name: testCase.name ?? "",
    description: testCase.description ?? "",
    input: testCase.input,
    trajectory: testCase.expectedTrajectory.join("\n"),
    mode: testCase.trajectoryMode,
    groundTruth: testCase.groundTruth ?? "",
    structuredGroundTruth: testCase.structuredGroundTruth,
    tags: testCase.tags.join(", "),
    status: testCase.status,
  };
}

export function areTestDetailSnapshotsEqual<
  TGroundTruth extends SharedGroundTruth | null,
  TStatus extends string,
  TMode extends string,
>(
  left: TestDetailFormSnapshot<TGroundTruth, TStatus, TMode>,
  right: TestDetailFormSnapshot<TGroundTruth, TStatus, TMode>
): boolean {
  return (
    left.name === right.name &&
    left.description === right.description &&
    left.input === right.input &&
    left.trajectory === right.trajectory &&
    left.mode === right.mode &&
    left.groundTruth === right.groundTruth &&
    deepEqual(left.structuredGroundTruth, right.structuredGroundTruth) &&
    left.tags === right.tags &&
    left.status === right.status
  );
}

export function validateStructuredGroundTruth<TGroundTruth extends SharedGroundTruth>(
  gt: TGroundTruth | null
): string | null {
  if (!gt) return null;
  if (gt.kind === "flat" && gt.expectedActions.length === 0) {
    return "Structured ground truth requires at least one action";
  }
  if (gt.kind === "multiGroup") {
    if (gt.groups.length === 0) return "Multi-group ground truth requires at least one group";
    const emptyGroup = gt.groups.findIndex((group) => group.actions.length === 0);
    if (emptyGroup >= 0) return `Group ${emptyGroup + 1} has no actions`;
  }
  return null;
}
