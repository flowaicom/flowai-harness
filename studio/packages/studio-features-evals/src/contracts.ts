import type { AppScope, EvalRuntimeAdapter } from "@studio/core";

export interface EvalsFeatureNavigation {
  readonly buildRunHref: (runId: string) => string;
  readonly buildCaseHref: (runId: string, testCaseId: string) => string;
  readonly buildTraceHref: (runId: string, testCaseId: string, traceId: string) => string;
}

export interface EvalsFeatureHost {
  readonly scope: AppScope;
  readonly runtime: EvalRuntimeAdapter;
  readonly navigation: EvalsFeatureNavigation;
  readonly activeRunId?: string;
}

export type EvalsFeatureRuntime = EvalRuntimeAdapter;
