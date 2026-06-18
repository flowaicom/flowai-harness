import type { AppScope, ConnectRuntimeAdapter, DataSourceSummary } from "@studio/core";

export interface ConnectFeatureNavigation {
  readonly buildConnectHref: (path?: string) => string;
  readonly buildSourceHref: (sourceId: string) => string;
  readonly openChatWithPrompt: (prompt: string) => void | Promise<void>;
}

export interface ConnectFeatureHost {
  readonly scope: AppScope;
  readonly runtime: ConnectRuntimeAdapter;
  readonly navigation: ConnectFeatureNavigation;
  readonly selectedSourceId?: string;
  readonly sources?: readonly DataSourceSummary[];
}

export type ConnectFeatureRuntime = ConnectRuntimeAdapter;
