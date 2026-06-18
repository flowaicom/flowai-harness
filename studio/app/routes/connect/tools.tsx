import { ConnectToolsPage, type ConnectToolsRuntimeLike } from "@studio/features-connect";
import { useMemo } from "react";
import { SourcePicker } from "~/components/connect/source-picker";
import { useSourceId } from "~/lib/hooks/use-source-id";
import { useHarnessRuntime } from "~/lib/runtime/harness-runtime-context";
import {
  connectScopeKey,
  mapConnectResult,
  toolExecutionToConnectResult,
  toolSummaryToConnectTool,
} from "./connect-package-adapters";

export default function ToolsPage() {
  const { adapter, scope } = useHarnessRuntime();
  const { sourceId, setSourceId } = useSourceId("target");
  const scopeKey = connectScopeKey({ scope, sourceId });

  const runtime = useMemo<ConnectToolsRuntimeLike>(
    () => ({
      async listTools(inputScope) {
        const result = await adapter.listTools(inputScope);
        return mapConnectResult(result, (tools) => tools.map(toolSummaryToConnectTool));
      },
      async executeTool(inputScope, toolId, input) {
        const result = await adapter.executeTool(inputScope, {
          toolId,
          input: {
            ...input,
            ...(sourceId ? { sourceId } : {}),
          },
        });
        return mapConnectResult(result, toolExecutionToConnectResult);
      },
    }),
    [adapter, sourceId]
  );

  return (
    <ConnectToolsPage
      scope={scope}
      scopeKey={scopeKey}
      runtime={runtime}
      headerAccessory={<SourcePicker sourceId={sourceId} onSourceChange={setSourceId} />}
      emptyDescription="No inspectable tools are exposed for this workspace yet."
    />
  );
}
