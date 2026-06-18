/**
 * Connect sidebar component.
 *
 * Section-based navigation:
 * - AppNav at top
 * - Data sources with status dots + loading skeleton
 * - Section links (Discovery, Profiling, Knowledge, Search, Tools)
 *
 * @module components/connect/connect-sidebar
 */

import {
  DatabaseIcon,
  MessageSquareIcon,
  PlusIcon,
  SearchIcon,
  TableIcon,
  TrashIcon,
  WrenchIcon,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useNavigate } from "react-router";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "~/components/ui/dialog";
import type { DataSource, DataSourceStatus } from "~/lib/domain/data";
import { isOk } from "~/lib/domain/result";
import { useHarnessRuntime } from "~/lib/runtime/harness-runtime-context";
import { useScramble } from "~/lib/scramble";
import { useSchemaExplorer, useSourceCatalog, useSourceCatalogActions } from "~/lib/stores";
import { selectTableCount } from "~/lib/stores/schema-explorer";
import {
  selectConnectionStatuses,
  selectSelectedSourceId,
  selectSources,
} from "~/lib/stores/source-catalog";
import { selectActiveWorkspaceId, useWorkspace } from "~/lib/stores/workspace-store";
import { cn } from "~/lib/utils";

// ============================================================================
// Section Links
// ============================================================================

const sectionItems: { id: string; label: string; icon: typeof DatabaseIcon; to: string }[] = [
  { id: "discovery", label: "Discovery", icon: TableIcon, to: "/connect/discovery" },
  { id: "search", label: "Search", icon: SearchIcon, to: "/connect/search" },
  { id: "tools", label: "Tools", icon: WrenchIcon, to: "/connect/tools" },
];

const SOURCE_CRUD_ENABLED = false;

function dataSourceSummaryToDataSource(source: {
  readonly sourceId: string;
  readonly name: string;
  readonly kind?: string;
  readonly status?: string;
  readonly metadata?: Record<string, unknown>;
}): DataSource {
  const metadata = source.metadata ?? {};
  return {
    id: source.sourceId,
    name: source.name,
    kind: source.kind,
    status: source.status,
    databaseType: metadata.databaseType === "sqlite" ? "sqlite" : "postgresql",
    host: typeof metadata.host === "string" ? metadata.host : "workspace-runtime",
    port: typeof metadata.port === "number" ? metadata.port : 0,
    databaseName:
      typeof metadata.databaseName === "string" ? metadata.databaseName : "workspace-runtime",
    schemaName: typeof metadata.schemaName === "string" ? metadata.schemaName : "public",
    encryptedCredentials: null,
    isActive: true,
    createdAt: "1970-01-01T00:00:00Z",
    updatedAt: "1970-01-01T00:00:00Z",
    metadata,
  };
}

// ============================================================================
// Source Item
// ============================================================================

interface SourceItemProps {
  source: DataSource;
  isSelected: boolean;
  connectionStatus?: DataSourceStatus;
  onDelete?: (id: string) => void;
}

function getStatusDotClass(source: DataSource, status?: DataSourceStatus): string {
  if (!source.isActive) return "bg-muted-foreground/30";
  switch (status?.status) {
    case "connected":
      return "bg-[var(--dot-emerald)]";
    case "disconnected":
      return "bg-[var(--dot-amber)]";
    case "error":
      return "bg-[var(--dot-red)]";
    default:
      return "bg-[var(--dot-emerald)]/50";
  }
}

function getStatusTitle(source: DataSource, status?: DataSourceStatus): string {
  if (!source.isActive) return "Inactive";
  switch (status?.status) {
    case "connected":
      return "Connected";
    case "disconnected":
      return "Disconnected";
    case "error":
      return `Error: ${status.message}`;
    default:
      return "Unknown";
  }
}

function SourceItem({ source, isSelected, connectionStatus, onDelete }: SourceItemProps) {
  const { s } = useScramble();
  const canDelete = onDelete && source.metadata?.readOnly !== true;
  return (
    <Link
      to={`/connect/sources/${source.id}`}
      prefetch="intent"
      className={cn(
        "group flex items-center gap-2 px-3 py-1.5 rounded-md transition-colors",
        isSelected ? "bg-primary/10 text-primary" : "hover:bg-muted text-foreground"
      )}
    >
      <span
        className={cn("status-dot", getStatusDotClass(source, connectionStatus))}
        title={getStatusTitle(source, connectionStatus)}
      />
      <span className="text-sm truncate flex-1">{s(source.name)}</span>
      <span className="text-[10px] font-mono text-muted-foreground/70 truncate shrink-0 max-w-24 group-hover:hidden">
        {s(source.host)}
      </span>
      {canDelete && (
        <button
          type="button"
          onClick={(e) => {
            e.preventDefault();
            e.stopPropagation();
            onDelete(source.id);
          }}
          className="hidden group-hover:block p-0.5 rounded hover:bg-destructive/10 hover:text-destructive transition-all shrink-0"
          aria-label="Delete data source"
        >
          <TrashIcon className="size-3" />
        </button>
      )}
    </Link>
  );
}

// ============================================================================
// Main Component
// ============================================================================

export function ConnectSidebar() {
  const navigate = useNavigate();
  const { adapter, scope } = useHarnessRuntime();
  const sources = useSourceCatalog(selectSources);
  const activeSourceId = useSourceCatalog(selectSelectedSourceId);
  const connectionStatuses = useSourceCatalog(selectConnectionStatuses);
  const {
    reset: resetSourceCatalog,
    setSources,
    removeSource,
    selectSource,
  } = useSourceCatalogActions();
  const tableCount = useSchemaExplorer(selectTableCount);

  const activeWorkspaceId = useWorkspace(selectActiveWorkspaceId);

  const [isLoading, setIsLoading] = useState(true);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const exploreBusyRef = useRef(false);
  const loadRequestIdRef = useRef(0);
  const loadWorkspaceIdRef = useRef("");

  // Auto-clear transient delete error
  useEffect(() => {
    if (!deleteError) return;
    const timer = setTimeout(() => setDeleteError(null), 4000);
    return () => clearTimeout(timer);
  }, [deleteError]);

  // Load sources on mount
  useEffect(() => {
    const requestId = loadRequestIdRef.current + 1;
    const requestWorkspaceId = activeWorkspaceId;
    loadRequestIdRef.current = requestId;
    loadWorkspaceIdRef.current = requestWorkspaceId;
    let cancelled = false;

    const load = async () => {
      resetSourceCatalog();
      setIsLoading(true);
      const result = await adapter.listDataSources(scope);
      if (
        cancelled ||
        loadRequestIdRef.current !== requestId ||
        loadWorkspaceIdRef.current !== requestWorkspaceId
      ) {
        return;
      }
      if (isOk(result)) setSources(result.value.map(dataSourceSummaryToDataSource));
      setIsLoading(false);
    };
    void load();

    return () => {
      cancelled = true;
    };
  }, [activeWorkspaceId, adapter, resetSourceCatalog, scope, setSources]);

  const handleNewSource = useCallback(() => {
    selectSource(null);
    navigate("/connect/sources/new");
  }, [selectSource, navigate]);

  const handleDeleteSource = useCallback((id: string) => {
    setPendingDeleteId(id);
  }, []);

  const confirmDelete = useCallback(async () => {
    if (!pendingDeleteId) return;
    const id = pendingDeleteId;
    setPendingDeleteId(null);
    setDeleteError("Data source deletion is not available in harness mode.");
    removeSource(id);
    if (id === activeSourceId) {
      navigate("/connect");
    }
  }, [pendingDeleteId, removeSource, activeSourceId, navigate]);

  return (
    <div className="flex flex-col h-full">
      {/* Sources Header */}
      <div className="px-4 py-3 border-b flex items-center justify-between">
        <h2 className="font-semibold text-sm text-foreground">
          Sources
          {sources.length > 0 && (
            <span className="text-muted-foreground font-normal ml-1.5 tabular-nums">
              {sources.length}
            </span>
          )}
        </h2>
        {SOURCE_CRUD_ENABLED && (
          <button
            type="button"
            onClick={handleNewSource}
            className="p-1.5 rounded-md hover:bg-muted transition-colors"
            aria-label="New data source"
          >
            <PlusIcon className="size-4" />
          </button>
        )}
      </div>

      {/* Source List */}
      {deleteError && (
        <div className="mx-2 mt-1 px-2.5 py-1.5 text-xs text-destructive bg-destructive/10 rounded-md">
          {deleteError}
        </div>
      )}
      <div className="overflow-y-auto scroll-container p-2 space-y-0.5">
        {isLoading && sources.length === 0 ? (
          <div className="space-y-0.5 px-1 pt-1">
            {Array.from({ length: 3 }, (_, i) => (
              <div
                // biome-ignore lint/suspicious/noArrayIndexKey: static skeleton
                key={i}
                className="flex items-center gap-2 px-3 py-1.5"
                style={{ animationDelay: `${i * 50}ms` }}
              >
                <div className="status-dot bg-muted animate-pulse" />
                <div
                  className="h-3.5 bg-muted rounded animate-pulse flex-1"
                  style={{ maxWidth: `${70 - i * 10}%` }}
                />
                <div className="w-14 h-3 bg-muted/60 rounded animate-pulse" />
              </div>
            ))}
          </div>
        ) : sources.length === 0 ? (
          <div className="text-center py-4 px-4">
            <DatabaseIcon className="size-6 mx-auto mb-1.5 text-muted-foreground/30" />
            <p className="text-sm text-muted-foreground">No sources configured</p>
            {SOURCE_CRUD_ENABLED && (
              <button
                type="button"
                onClick={handleNewSource}
                className="mt-2 text-xs text-primary hover:underline"
              >
                Add a data source
              </button>
            )}
          </div>
        ) : (
          sources.map((source) => (
            <SourceItem
              key={source.id}
              source={source}
              isSelected={source.id === activeSourceId}
              connectionStatus={connectionStatuses.get(source.id)}
              onDelete={SOURCE_CRUD_ENABLED ? handleDeleteSource : undefined}
            />
          ))
        )}
      </div>

      {/* Section Links */}
      <div className="border-t p-2 space-y-0.5">
        <span className="px-3 py-1 text-[10px] font-medium text-muted-foreground/60 uppercase tracking-wider">
          Sections
        </span>
        {sectionItems.map(({ id, label, icon: Icon, to }) => {
          let badge: React.ReactNode = null;
          if (id === "discovery" && tableCount > 0) {
            badge = (
              <span className="ml-auto text-[10px] font-mono tabular-nums text-muted-foreground/60">
                {tableCount}
              </span>
            );
          }

          return (
            <Link
              key={id}
              to={to}
              prefetch="intent"
              className="flex items-center gap-2.5 px-3 py-1.5 rounded-md text-sm transition-colors text-muted-foreground hover:bg-muted/60 hover:text-foreground"
            >
              <Icon className="size-3.5" />
              {label}
              {badge}
            </Link>
          );
        })}
      </div>

      {/* Explore in Chat */}
      <div className="border-t p-2">
        <button
          type="button"
          onClick={async () => {
            if (exploreBusyRef.current) return;
            exploreBusyRef.current = true;
            try {
              const parts: string[] = [];
              if (tableCount > 0) parts.push(`${tableCount} table${tableCount > 1 ? "s" : ""}`);
              const context = parts.length > 0 ? ` I have ${parts.join(", ")} available.` : "";
              const msg = `I'd like to explore and analyze the data.${context} What can you help me with?`;

              void msg;
              navigate("/playground");
            } finally {
              exploreBusyRef.current = false;
            }
          }}
          className="group flex w-full items-center gap-2.5 rounded-md border border-border/70 bg-background/70 px-3 py-2 text-left text-sm text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <span className="flex size-6 shrink-0 items-center justify-center rounded-md border border-border/60 bg-muted/30 transition-colors group-hover:bg-muted/50">
            <MessageSquareIcon className="size-3.5" />
          </span>
          Explore in Chat
        </button>
      </div>

      {/* Delete confirmation dialog */}
      <Dialog
        open={pendingDeleteId !== null}
        onOpenChange={(open) => {
          if (!open) setPendingDeleteId(null);
        }}
      >
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>Delete data source</DialogTitle>
            <DialogDescription>Are you sure? This action cannot be undone.</DialogDescription>
          </DialogHeader>
          <div className="flex justify-end gap-2 pt-4">
            <button
              type="button"
              onClick={() => setPendingDeleteId(null)}
              className="px-3 py-1.5 text-sm rounded-md border hover:bg-muted transition-colors"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={confirmDelete}
              className="px-3 py-1.5 text-sm rounded-md bg-destructive text-destructive-foreground hover:bg-destructive/90 transition-colors"
            >
              Delete
            </button>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
