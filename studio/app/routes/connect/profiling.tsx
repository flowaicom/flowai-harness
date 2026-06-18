import { ConnectProfilingPage, type ConnectProfilingStatusKey } from "@studio/features-connect";
import { BookOpenIcon, MessageSquareIcon, SearchIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Link } from "react-router";
import { ProfilingModelSelector } from "~/components/connect/profiling-model-selector";
import { useElapsedTimer } from "~/components/connect/profiling-progress-bar";
import { SourcePicker } from "~/components/connect/source-picker";
import type { IngestionEvent } from "~/lib/domain/data";
import { isOk } from "~/lib/domain/result";
import { useSourceId } from "~/lib/hooks/use-source-id";
import { useHarnessRuntime } from "~/lib/runtime/harness-runtime-context";
import { useScramble } from "~/lib/scramble";
import {
  useProfilingPipeline,
  useProfilingPipelineActions,
  useSchemaExplorer,
  useSchemaExplorerActions,
  useSourceCatalog,
  useSourceCatalogActions,
} from "~/lib/stores";
import {
  selectCompletedTableCount,
  selectCurrentProfilingTable,
  selectDiscoveredTableNames,
  selectProfilingActiveJobId,
  selectProfilingError,
  selectProfilingIsRunning,
  selectProfilingJobs,
  selectProfilingStartedAt,
  selectTableStages,
  selectTotalTableCount,
} from "~/lib/stores/profiling-pipeline";
import { selectTables } from "~/lib/stores/schema-explorer";
import { selectSourcesLoading } from "~/lib/stores/source-catalog";
import {
  connectTableToTableInfo,
  profilingSummaryFromUnknown,
  tableStagesToConnectTableStages,
  tableSummaryToConnectTable,
} from "./connect-package-adapters";

export default function ProfilingPage() {
  const { s } = useScramble();
  const { adapter, scope } = useHarnessRuntime();
  const isRunning = useProfilingPipeline(selectProfilingIsRunning);
  const profilingJobs = useProfilingPipeline(selectProfilingJobs);
  const activeJobId = useProfilingPipeline(selectProfilingActiveJobId);
  const { sourceId: activeSourceId, setSourceId: setActiveSourceId } = useSourceId("target");
  const {
    startProfiling: startProfilingAction,
    interpretIngestionEvent: handleIngestionEvent,
    completeProfiling,
    resetMatrix: resetProfilingMatrix,
    setError: setStoreError,
  } = useProfilingPipelineActions();
  const tableStages = useProfilingPipeline(selectTableStages);
  const discoveredTableNames = useProfilingPipeline(selectDiscoveredTableNames);
  const currentProfilingTable = useProfilingPipeline(selectCurrentProfilingTable);
  const profilingStartedAt = useProfilingPipeline(selectProfilingStartedAt);
  const totalTableCount = useProfilingPipeline(selectTotalTableCount);
  const tableCompletedCount = useProfilingPipeline(selectCompletedTableCount);
  const storeError = useProfilingPipeline(selectProfilingError);
  const tables = useSchemaExplorer(selectTables);
  const { setTables } = useSchemaExplorerActions();
  const isLoading = useSourceCatalog(selectSourcesLoading);
  const { setLoadPhase } = useSourceCatalogActions();
  const [abortFn, setAbortFn] = useState<(() => void) | null>(null);
  const [selectedModelId, setSelectedModelId] = useState<string | undefined>(undefined);

  const sourceId = activeSourceId ?? "default";
  const elapsedMs = useElapsedTimer(isRunning ? profilingStartedAt : null);

  useEffect(() => {
    const load = async () => {
      setLoadPhase({ phase: "loading" });
      const result = await adapter.listTables(scope, { sourceId: activeSourceId ?? undefined });
      if (isOk(result)) {
        setTables(
          result.value.map((table) => connectTableToTableInfo(tableSummaryToConnectTable(table)))
        );
        setLoadPhase({ phase: "ready" });
      } else {
        setLoadPhase({ phase: "failed", reason: "Failed to load tables" });
      }
    };
    void load();
  }, [activeSourceId, adapter, scope, setLoadPhase, setTables]);

  const activeStatus = activeJobId ? profilingJobs.get(activeJobId) : undefined;
  const currentStatusKey: ConnectProfilingStatusKey = isRunning
    ? (activeStatus?.status ?? "queued")
    : activeStatus?.status === "completed"
      ? "completed"
      : storeError
        ? "failed"
        : "queued";
  const isCompleted = !isRunning && activeJobId !== null && activeStatus?.status === "completed";
  const summary =
    activeStatus?.status === "completed" ? profilingSummaryFromUnknown(activeStatus.summary) : null;
  const connectTableStages = useMemo(
    () => tableStagesToConnectTableStages(tableStages),
    [tableStages]
  );

  const handleProfileTable = useCallback(
    async (tableName: string) => {
      setStoreError(null);

      let activeProfilingJobId: string | null = null;
      const result = await adapter.startProfileTable(scope, {
        sourceId,
        tableName,
        modelId: selectedModelId || undefined,
        handlers: {
          onEvent: (event) => {
            const ingestionEvent = event as IngestionEvent;
            if (ingestionEvent.type === "started") {
              activeProfilingJobId = ingestionEvent.jobId;
              startProfilingAction(ingestionEvent.jobId);
            }
            if (activeProfilingJobId) {
              handleIngestionEvent(activeProfilingJobId, ingestionEvent);
            }
          },
          onComplete: () => completeProfiling(),
          onError: (error) => {
            setStoreError(error.message);
            completeProfiling();
          },
        },
      });

      if (isOk(result)) {
        setAbortFn(() => result.value.abort);
      } else {
        setStoreError(result.error.message);
      }
    },
    [
      adapter,
      completeProfiling,
      handleIngestionEvent,
      scope,
      selectedModelId,
      setStoreError,
      sourceId,
      startProfilingAction,
    ]
  );

  const handleProfileAll = useCallback(async () => {
    setStoreError(null);

    let activeProfilingJobId: string | null = null;
    const result = await adapter.startProfileDatabase(scope, {
      sourceId,
      modelId: selectedModelId || undefined,
      handlers: {
        onEvent: (event) => {
          const ingestionEvent = event as IngestionEvent;
          if (ingestionEvent.type === "started") {
            activeProfilingJobId = ingestionEvent.jobId;
            startProfilingAction(ingestionEvent.jobId);
          }
          if (activeProfilingJobId) {
            handleIngestionEvent(activeProfilingJobId, ingestionEvent);
          }
        },
        onComplete: () => completeProfiling(),
        onError: (error) => {
          setStoreError(error.message);
          completeProfiling();
        },
      },
    });

    if (isOk(result)) {
      setAbortFn(() => result.value.abort);
    } else {
      setStoreError(result.error.message);
    }
  }, [
    adapter,
    completeProfiling,
    handleIngestionEvent,
    scope,
    selectedModelId,
    setStoreError,
    sourceId,
    startProfilingAction,
  ]);

  const handleCancel = useCallback(() => {
    abortFn?.();
    completeProfiling();
    setAbortFn(null);
  }, [abortFn, completeProfiling]);

  const handleClear = useCallback(() => {
    resetProfilingMatrix();
    setAbortFn(null);
  }, [resetProfilingMatrix]);

  return (
    <ConnectProfilingPage
      tables={tables}
      isLoading={isLoading}
      hasTarget={true}
      isRunning={isRunning}
      isCompleted={isCompleted}
      currentStatusKey={currentStatusKey}
      tableStages={connectTableStages}
      discoveredTableNames={discoveredTableNames}
      currentTable={currentProfilingTable}
      totalTableCount={totalTableCount}
      completedTableCount={tableCompletedCount}
      elapsedMs={elapsedMs}
      error={storeError}
      onDismissError={() => setStoreError(null)}
      onRetryError={() => {
        void handleProfileAll();
      }}
      onProfileAll={() => {
        void handleProfileAll();
      }}
      onProfileTable={(tableName) => {
        void handleProfileTable(tableName);
      }}
      onCancel={handleCancel}
      onClear={handleClear}
      canStartProfiling={tables.length > 0}
      summary={summary}
      controls={({ tableCount, totalColumns, isRunning: controlsRunning }) => (
        <div className="flex items-center gap-4 flex-wrap">
          <SourcePicker
            sourceId={activeSourceId}
            onSourceChange={setActiveSourceId}
            disabled={controlsRunning}
          />
          <ProfilingModelSelector
            tableCount={tableCount}
            totalColumns={totalColumns}
            selectedModelId={selectedModelId}
            onModelChange={setSelectedModelId}
            disabled={controlsRunning}
          />
        </div>
      )}
      emptyState={{
        title: "No tables available",
        description: "Connect a data source to discover tables for profiling",
      }}
      nextSteps={
        <div className="flex flex-wrap gap-2">
          <Link
            to="/connect/knowledge"
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md border hover:bg-muted transition-colors"
          >
            <BookOpenIcon className="size-3.5" />
            Browse Knowledge
          </Link>
          <Link
            to="/connect/search"
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md border hover:bg-muted transition-colors"
          >
            <SearchIcon className="size-3.5" />
            Search Data
          </Link>
          <Link
            to="/playground"
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
          >
            <MessageSquareIcon className="size-3.5" />
            Start Chatting
          </Link>
        </div>
      }
      formatText={s}
    />
  );
}
