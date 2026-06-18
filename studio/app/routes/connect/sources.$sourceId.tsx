import { DatabaseIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { Link, useParams } from "react-router";
import { DataRow } from "~/components/shared/data-row";
import { EmptyState } from "~/components/shared/empty-state";
import { ErrorBanner } from "~/components/shared/error-banner";
import { SectionCard, SectionHeader } from "~/components/shared/section-card";
import { isOk } from "~/lib/domain/result";
import { useHarnessRuntime } from "~/lib/runtime/harness-runtime-context";
import { useSourceCatalogActions } from "~/lib/stores";

interface SourceDetail {
  readonly sourceId: string;
  readonly name: string;
  readonly kind?: string;
  readonly status?: string;
  readonly metadata?: Record<string, unknown>;
}

export default function SourceDetailPage() {
  const { sourceId } = useParams<{ sourceId: string }>();
  const { adapter, scope } = useHarnessRuntime();
  const { selectSource } = useSourceCatalogActions();
  const [source, setSource] = useState<SourceDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!sourceId) return;
    selectSource(sourceId);
    const load = async () => {
      const result = await adapter.listDataSources(scope);
      if (isOk(result)) {
        const match = result.value.find((item) => item.sourceId === sourceId) ?? null;
        setSource(match);
        if (!match) setError(`Data source "${sourceId}" was not found.`);
      } else {
        setError(result.error.message);
      }
    };
    void load();
  }, [adapter, scope, selectSource, sourceId]);

  if (error && !source) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <EmptyState icon={DatabaseIcon} title="Source unavailable" description={error} />
      </div>
    );
  }

  if (!source) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <EmptyState icon={DatabaseIcon} title="Loading source" description="Loading..." />
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto scroll-container">
      <div className="max-w-lg mx-auto p-6 space-y-6">
        <div className="flex items-center gap-3">
          <DatabaseIcon className="size-6 text-primary shrink-0" />
          <div>
            <h1 className="text-lg font-semibold">{source.name}</h1>
            <p className="text-xs text-muted-foreground font-mono">{source.sourceId}</p>
          </div>
        </div>

        <SectionCard>
          <SectionHeader>Harness Source</SectionHeader>
          <div className="space-y-2">
            <DataRow label="Kind" value={source.kind ?? "workspace-runtime"} />
            <DataRow label="Status" value={source.status ?? "ready"} />
            <DataRow
              label="Database"
              value={String(source.metadata?.databaseName ?? "workspace-runtime")}
              mono
            />
            <DataRow label="Schema" value={String(source.metadata?.schemaName ?? "public")} mono />
          </div>
        </SectionCard>

        <SectionCard>
          <SectionHeader>Actions</SectionHeader>
          <div className="flex flex-wrap gap-2">
            <Link
              to="/connect/discovery"
              className="inline-flex items-center rounded-md border px-3 py-1.5 text-xs hover:bg-muted"
            >
              Discover Schema
            </Link>
            <Link
              to="/connect/profiling"
              className="inline-flex items-center rounded-md border px-3 py-1.5 text-xs hover:bg-muted"
            >
              Profile Tables
            </Link>
          </div>
          <p className="text-xs text-muted-foreground">
            Source create, update, delete, and connection-test operations are intentionally
            unsupported in harness mode. Configure the workspace data_environment in code.
          </p>
        </SectionCard>

        {error && <ErrorBanner message={error} onDismiss={() => setError(null)} />}
      </div>
    </div>
  );
}
