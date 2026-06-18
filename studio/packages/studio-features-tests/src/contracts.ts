import type { AppScope, TestBuilderRuntimeAdapter, TestRuntimeAdapter } from "@studio/core";

export interface TestsFeatureNavigation {
  readonly buildTestCaseHref: (testCaseId: string) => string;
  readonly buildNewTestHref: () => string;
  readonly openChatWithPrompt?: (prompt: string) => void | Promise<void>;
}

export interface TestsFeatureHost {
  readonly scope: AppScope;
  readonly runtime: TestRuntimeAdapter;
  readonly builderRuntime?: TestBuilderRuntimeAdapter;
  readonly navigation: TestsFeatureNavigation;
  readonly activeTestCaseId?: string;
}

export type TestsFeatureRuntime = TestRuntimeAdapter;
export type TestsFeatureBuilderRuntime = TestBuilderRuntimeAdapter;
