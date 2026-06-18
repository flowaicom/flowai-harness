import { ConnectSearchPage, type ConnectSearchRuntimeLike } from "@studio/features-connect";
import { useMemo } from "react";
import { useNavigate } from "react-router";
import { SourcePicker } from "~/components/connect/source-picker";
import { useSourceId } from "~/lib/hooks/use-source-id";
import { useHarnessRuntime } from "~/lib/runtime/harness-runtime-context";
import { useScramble } from "~/lib/scramble";
import { useCatalogSearch, useCatalogSearchActions } from "~/lib/stores";
import { selectSearchQuery, selectSearchResults } from "~/lib/stores/catalog-search";
import {
  catalogSearchToConnectResults,
  connectScopeKey,
  mapConnectResult,
  toolExecutionToConnectResult,
} from "./connect-package-adapters";

export default function SearchPage() {
  const { s } = useScramble();
  const navigate = useNavigate();
  const { adapter, scope } = useHarnessRuntime();
  const searchQuery = useCatalogSearch(selectSearchQuery);
  const searchResults = useCatalogSearch(selectSearchResults);
  const { setQuery: setSearchQuery, setResults: setSearchResults } = useCatalogSearchActions();
  const { sourceId, setSourceId } = useSourceId("target");
  const scopeKey = connectScopeKey({ scope, sourceId });

  const runtime = useMemo<ConnectSearchRuntimeLike>(
    () => ({
      async searchCatalog(inputScope, input) {
        const result = await adapter.searchCatalog(inputScope, {
          query: input.query,
          sourceId,
          mode: input.mode === "semantic" ? "semantic" : undefined,
        });
        return mapConnectResult(result, catalogSearchToConnectResults);
      },
      async runSearchTool(inputScope, toolId, input) {
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
    <ConnectSearchPage
      scope={scope}
      scopeKey={scopeKey}
      runtime={runtime}
      query={searchQuery}
      setQuery={setSearchQuery}
      results={searchResults}
      setResults={setSearchResults}
      onAskAboutItem={() => {
        navigate("/playground");
      }}
      headerAccessory={<SourcePicker sourceId={sourceId} onSourceChange={setSourceId} />}
      formatText={s}
    />
  );
}
