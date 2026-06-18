import { type AppScope, createWorkspaceScope, getWorkspaceKey } from "@studio/core/domain/scope";
import type { FlowAIHarnessRuntimeAdapter } from "@studio/core/runtime";
import { createContext, useContext, useMemo } from "react";
import { selectActiveWorkspaceId, useWorkspace } from "~/lib/stores/workspace-store";
import { flowAIHarnessRuntimeAdapter } from "./flowai-harness-runtime-adapter";

export interface HarnessRuntimeContextValue {
  readonly adapter: FlowAIHarnessRuntimeAdapter;
  readonly scope: AppScope;
  readonly workspaceKey: string;
}

const HarnessRuntimeContext = createContext<HarnessRuntimeContextValue | null>(null);

export function HarnessRuntimeProvider({
  adapter = flowAIHarnessRuntimeAdapter,
  children,
}: {
  readonly adapter?: FlowAIHarnessRuntimeAdapter;
  readonly children: React.ReactNode;
}) {
  const activeWorkspaceId = useWorkspace(selectActiveWorkspaceId);
  const scope = useMemo(
    () => createWorkspaceScope(activeWorkspaceId || "default"),
    [activeWorkspaceId]
  );
  const value = useMemo(
    () => ({
      adapter,
      scope,
      workspaceKey: getWorkspaceKey(scope),
    }),
    [adapter, scope]
  );

  return <HarnessRuntimeContext.Provider value={value}>{children}</HarnessRuntimeContext.Provider>;
}

export function useHarnessRuntime(): HarnessRuntimeContextValue {
  const context = useContext(HarnessRuntimeContext);
  if (!context) {
    throw new Error("useHarnessRuntime must be used within HarnessRuntimeProvider");
  }
  return context;
}
