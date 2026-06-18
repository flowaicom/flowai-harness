import {
  type ConnectDiscoveryDetailLike,
  ConnectDiscoveryPage,
  type ConnectDiscoveryRuntimeLike,
  type ConnectDiscoveryTableLike,
  type ConnectRuntimeResult,
} from "@studio/features-connect";
import { BarChart3Icon } from "lucide-react";
import { useCallback, useMemo } from "react";
import { Link, useNavigate } from "react-router";
import { SourcePicker } from "~/components/connect/source-picker";
import { useSourceId } from "~/lib/hooks/use-source-id";
import { useHarnessRuntime } from "~/lib/runtime/harness-runtime-context";
import { useScramble } from "~/lib/scramble";
import { useSchemaExplorer, useSchemaExplorerActions } from "~/lib/stores";
import { selectTableDetail, selectTables } from "~/lib/stores/schema-explorer";
import {
  connectDetailToPhysicalTable,
  connectScopeKey,
  connectTableToTableInfo,
  mapConnectResult,
  tableDetailToConnectDetail,
  tableSummaryToConnectTable,
  toolOutputToConnectRelationships,
} from "./connect-package-adapters";

export default function DiscoveryPage() {
  const { s } = useScramble();
  const navigate = useNavigate();
  const { adapter, scope } = useHarnessRuntime();
  const tables = useSchemaExplorer(selectTables);
  const tableDetail = useSchemaExplorer(selectTableDetail);
  const { setTables, setTableDetail } = useSchemaExplorerActions();
  const { sourceId, setSourceId } = useSourceId("target");
  const scopeKey = connectScopeKey({ scope, sourceId });

  const handleSetTables = useCallback(
    (nextTables: ConnectDiscoveryTableLike[]) => {
      setTables(nextTables.map(connectTableToTableInfo));
    },
    [setTables]
  );

  const handleSetTableDetail = useCallback(
    (nextDetail: ConnectDiscoveryDetailLike | null) => {
      setTableDetail(nextDetail ? connectDetailToPhysicalTable(nextDetail) : null);
    },
    [setTableDetail]
  );

  const runtime = useMemo<ConnectDiscoveryRuntimeLike>(
    () => ({
      async listTables(inputScope, params) {
        const result = await adapter.listTables(inputScope, {
          sourceId,
          schema: params?.schema,
          signal: params?.signal,
        });
        return mapConnectResult(result, (value) => value.map(tableSummaryToConnectTable));
      },
      async getTableDetail(inputScope, tableName, params) {
        const result = await adapter.getTableDetail(inputScope, {
          tableName,
          schema: params?.schema,
          sourceId,
        });
        return mapConnectResult(result, tableDetailToConnectDetail);
      },
    }),
    [adapter, sourceId]
  );

  const loadRelationships = useCallback(
    async (
      _inputScope: typeof scope,
      tableName: string
    ): Promise<ConnectRuntimeResult<ReturnType<typeof toolOutputToConnectRelationships>>> => {
      const result = await adapter.executeTool(scope, {
        toolId: "get_catalog_relations",
        input: {
          refs: [{ name: tableName, kind: "table" }],
          target_kinds: ["table"],
          ...(sourceId ? { sourceId } : {}),
        },
      });
      if (result._tag !== "Ok") {
        return result;
      }
      if (result.value.status !== "success") {
        return { _tag: "Err", error: { message: "No relationships found" } };
      }
      return {
        _tag: "Ok",
        value: toolOutputToConnectRelationships(tableName, result.value.output),
      };
    },
    [adapter, scope, sourceId]
  );

  return (
    <ConnectDiscoveryPage
      scope={scope}
      scopeKey={scopeKey}
      hasTarget={true}
      runtime={runtime}
      tables={tables}
      tableDetail={tableDetail}
      setTables={handleSetTables}
      setTableDetail={handleSetTableDetail}
      loadRelationships={loadRelationships}
      emptyState={{
        title: "No tables found",
        description: "No tables were discovered for the active workspace runtime data environment.",
      }}
      onExploreTable={({ prompt }) => {
        void prompt;
        navigate("/playground");
      }}
      exploreLabel="Explore in Chat"
      headerAccessory={<SourcePicker sourceId={sourceId} onSourceChange={setSourceId} />}
      detailSecondaryAction={(detail) => (
        <Link
          to="/connect/profiling"
          aria-label={`Profile ${detail.tableName}`}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs text-muted-foreground hover:bg-muted hover:text-foreground transition-colors border"
        >
          <BarChart3Icon className="size-3.5" />
          Profile
        </Link>
      )}
      formatText={s}
    />
  );
}
