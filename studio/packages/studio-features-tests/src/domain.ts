export type SharedTestCaseStatus = "draft" | "active" | "archived";

export type SharedTrajectoryMode = "strict" | "unordered" | "subset" | "superset" | "subsequence";

export interface SharedExpectedActionLike {
  readonly actionType: string;
  readonly payload?: unknown;
  readonly scope?: unknown;
  readonly expectedFilters?: unknown;
  readonly entityIds?: readonly string[];
  readonly entityFingerprints?: readonly string[];
  readonly entitySql?: string;
  readonly entityDescription?: string;
  readonly productIds?: readonly string[];
  readonly productFingerprints?: readonly string[];
  readonly productSql?: string;
  readonly productDescription?: string;
}

export interface SharedExpectedGroupLike {
  readonly actions: readonly SharedExpectedActionLike[];
  readonly filters?: unknown;
  readonly scope?: unknown;
  readonly entityIds?: readonly string[];
  readonly entitySql?: string;
  readonly entityDescription?: string;
  readonly productIds?: readonly string[];
  readonly productSql?: string;
  readonly productDescription?: string;
}

export type SharedGroundTruth =
  | {
      readonly kind: "textOnly";
      readonly text: string;
    }
  | {
      readonly kind: "flat";
      readonly expectedActions: readonly SharedExpectedActionLike[];
      readonly expectedFilters?: unknown;
      readonly expectedScope?: unknown;
      readonly expectedScopeCodes?: unknown;
      readonly groundTruthSql?: string;
      readonly groundTruthEntityIds?: readonly string[];
      readonly groundTruthEntityFingerprints?: readonly string[];
      readonly groundTruthProductIds?: readonly string[];
      readonly groundTruthProductFingerprints?: readonly string[];
    }
  | {
      readonly kind: "multiGroup";
      readonly groups: readonly SharedExpectedGroupLike[];
    };

export interface TestBuilderSessionLike {
  readonly sessionId?: string;
  readonly userPrompt: string | null;
  readonly composedTrajectory: readonly unknown[];
  readonly trajectorySources?: readonly unknown[];
  readonly trajectoryMode?: SharedTrajectoryMode | null;
  readonly groundTruth?: string | null;
  readonly structuredGroundTruth?: SharedGroundTruth | null;
  readonly tags?: readonly string[];
  readonly createdAt?: string;
  readonly updatedAt?: string;
}
