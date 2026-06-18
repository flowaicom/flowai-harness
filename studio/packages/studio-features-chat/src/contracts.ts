import type { AppScope, ChatRuntimeAdapter, ThreadSummary } from "@studio/core";

export interface ChatFeatureNavigation {
  readonly buildThreadHref: (threadId: string) => string;
  readonly buildPlaygroundHref?: (threadId?: string) => string;
  readonly openThread?: (threadId: string) => void | Promise<void>;
}

export interface ChatFeatureHost {
  readonly scope: AppScope;
  readonly runtime: ChatRuntimeAdapter;
  readonly navigation: ChatFeatureNavigation;
  readonly activeThreadId?: string;
  readonly threads?: readonly ThreadSummary[];
}

export type ChatFeatureRuntime = ChatRuntimeAdapter;
