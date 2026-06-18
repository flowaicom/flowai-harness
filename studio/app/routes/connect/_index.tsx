import {
  ActivityIcon,
  DatabaseIcon,
  ExternalLinkIcon,
  type LucideIcon,
  MessageSquareIcon,
  SearchIcon,
  WrenchIcon,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useNavigate } from "react-router";
import { EmptyState } from "~/components/shared/empty-state";
import type {
  DocumentItem,
  KnowledgeItem,
  KnowledgeType,
  TableInfo,
  TableType,
} from "~/lib/domain/data";
import { isOk } from "~/lib/domain/result";
import { useHarnessRuntime } from "~/lib/runtime/harness-runtime-context";
import {
  selectActiveWorkspaceId,
  useKnowledgeBase,
  useKnowledgeBaseActions,
  useSchemaExplorer,
  useSchemaExplorerActions,
  useSourceCatalog,
  useWorkspace,
} from "~/lib/stores";
import { selectDocumentCount, selectKnowledgeCount } from "~/lib/stores/knowledge-base";
import { selectTableCount, selectTables } from "~/lib/stores/schema-explorer";
import { selectSelectedSourceId, selectSources } from "~/lib/stores/source-catalog";
import { cn } from "~/lib/utils";

// =============================================================================
// Stat Cards
// =============================================================================

interface StatCardProps {
  readonly value: number;
  readonly label: string;
  readonly isLoading?: boolean;
}

function StatCard({ value, label, isLoading = false }: StatCardProps) {
  return (
    <div className="rounded-lg border border-border bg-card p-3 transition-colors">
      <p className="text-xs text-muted-foreground">{label}</p>
      {isLoading ? (
        <div aria-hidden="true" className="mt-1.5 h-8 w-14 rounded-md bg-muted animate-pulse" />
      ) : (
        <p className="mt-1 text-2xl font-semibold tabular-nums tracking-tight text-foreground">
          {value}
        </p>
      )}
    </div>
  );
}

// =============================================================================
// Quick Actions
// =============================================================================

interface QuickAction {
  to: string;
  icon: LucideIcon;
  label: string;
  description: string;
  meta?: string;
}

const QUICK_ACTIONS: QuickAction[] = [
  {
    to: "/connect/search",
    icon: SearchIcon,
    label: "Search",
    description: "Search across tables, columns, and metrics",
  },
  {
    to: "/connect/tools",
    icon: WrenchIcon,
    label: "Tools",
    description: "Browse and test data catalog tools",
  },
  {
    to: "/runs",
    icon: ActivityIcon,
    label: "Workspace activity",
    description: "Review recent runs and approvals",
    meta: "Runs",
  },
];

const connectActionClass =
  "group flex w-full min-w-0 items-center gap-3 rounded-lg border border-border/70 bg-background/70 px-3 py-2 text-left transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

function ConnectActionContent({
  icon: Icon,
  label,
  description,
  meta,
}: {
  readonly icon: LucideIcon;
  readonly label: string;
  readonly description: string;
  readonly meta?: string;
}) {
  return (
    <>
      <span className="flex size-7 shrink-0 items-center justify-center rounded-md border border-border/60 bg-muted/30 text-muted-foreground transition-colors group-hover:bg-muted/50 group-hover:text-foreground">
        <Icon aria-hidden="true" className="size-3.5" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-foreground">{label}</span>
        <span className="block truncate text-xs text-muted-foreground">{description}</span>
      </span>
      {meta && (
        <span className="flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
          {meta}
          <ExternalLinkIcon aria-hidden="true" className="size-3" />
        </span>
      )}
    </>
  );
}

function ConnectActionLink({
  to,
  icon,
  label,
  description,
  meta,
  className,
}: QuickAction & { readonly className?: string }) {
  return (
    <Link to={to} className={cn(connectActionClass, className)}>
      <ConnectActionContent icon={icon} label={label} description={description} meta={meta} />
    </Link>
  );
}

function tableSummaryToTableInfo(table: {
  readonly tableName: string;
  readonly schemaName?: string;
  readonly columnCount?: number;
  readonly metadata?: Record<string, unknown>;
}): TableInfo {
  return {
    schemaName: table.schemaName ?? "public",
    tableName: table.tableName,
    tableType: table.metadata?.tableType === "view" ? "view" : ("base_table" as TableType),
    rowCount: typeof table.metadata?.rowCount === "number" ? table.metadata.rowCount : null,
    columnCount: table.columnCount ?? null,
    description:
      typeof table.metadata?.description === "string" ? table.metadata.description : null,
  };
}

function documentSummaryToDocumentItem(document: {
  readonly documentId: string;
  readonly title: string;
  readonly sourceId?: string;
  readonly metadata?: Record<string, unknown>;
}): DocumentItem {
  const status = document.metadata?.extractionStatus;
  return {
    id: document.documentId,
    name: document.title,
    content: "",
    targetDatabaseId: document.sourceId ?? "workspace-runtime",
    extractionStatus:
      status === "processing" || status === "processed" || status === "failed" ? status : "pending",
    extractedKnowledgeIds: Array.isArray(document.metadata?.extractedKnowledgeIds)
      ? document.metadata.extractedKnowledgeIds.filter((id): id is string => typeof id === "string")
      : [],
    createdAt:
      typeof document.metadata?.createdAt === "string"
        ? document.metadata.createdAt
        : "1970-01-01T00:00:00Z",
  };
}

function knowledgeSummaryToKnowledgeItem(item: {
  readonly itemId: string;
  readonly title?: string;
  readonly content?: string;
  readonly metadata?: Record<string, unknown>;
}): KnowledgeItem {
  const knowledgeType = item.metadata?.knowledgeType;
  return {
    id: item.itemId,
    name: item.title ?? item.itemId,
    description: item.content ?? "",
    knowledgeType: isKnowledgeType(knowledgeType) ? knowledgeType : "custom",
    scopeTables: stringList(item.metadata?.scopeTables),
    scopeColumns: stringList(item.metadata?.scopeColumns),
    sqlExpression:
      typeof item.metadata?.sqlExpression === "string" ? item.metadata.sqlExpression : null,
    synonyms: stringList(item.metadata?.synonyms),
    sourceDocumentId:
      typeof item.metadata?.sourceDocumentId === "string" ? item.metadata.sourceDocumentId : null,
  };
}

function stringList(value: unknown): readonly string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string")
    : [];
}

function isKnowledgeType(value: unknown): value is KnowledgeType {
  return (
    value === "business_rule" ||
    value === "predicate" ||
    value === "terminology" ||
    value === "constraint" ||
    value === "temporal_rule" ||
    value === "implicit_intent" ||
    value === "data_quality" ||
    value === "custom"
  );
}

// =============================================================================
// Explore in Chat CTA
// =============================================================================

function ExploreInChatCTA({
  tableCount,
  tableNames,
  knowledgeCount,
  documentCount,
}: {
  readonly tableCount: number;
  readonly tableNames: readonly string[];
  readonly knowledgeCount: number;
  readonly documentCount: number;
}) {
  const navigate = useNavigate();
  const busyRef = useRef(false);

  const handleClick = useCallback(async () => {
    if (busyRef.current) return;
    busyRef.current = true;
    try {
      const parts: string[] = [];
      if (tableCount > 0) {
        const names = tableNames.slice(0, 8).join(", ");
        const suffix = tableNames.length > 8 ? ` and ${tableNames.length - 8} more` : "";
        parts.push(`${tableCount} table${tableCount > 1 ? "s" : ""} (${names}${suffix})`);
      }
      if (knowledgeCount > 0)
        parts.push(`${knowledgeCount} knowledge item${knowledgeCount > 1 ? "s" : ""}`);
      if (documentCount > 0) parts.push(`${documentCount} document${documentCount > 1 ? "s" : ""}`);

      const context = parts.length > 0 ? ` I have ${parts.join(", ")} available.` : "";
      const msg = `I'd like to explore and analyze the data.${context} What can you help me with?`;

      void msg;
      navigate("/playground");
    } finally {
      busyRef.current = false;
    }
  }, [tableCount, tableNames, knowledgeCount, documentCount, navigate]);

  return (
    <button type="button" onClick={handleClick} className={connectActionClass}>
      <ConnectActionContent
        icon={MessageSquareIcon}
        label="Explore in Chat"
        description="Ask the agent about your data"
      />
    </button>
  );
}

// =============================================================================
// Component
// =============================================================================

export default function ConnectIndex() {
  const { adapter, scope } = useHarnessRuntime();
  const sources = useSourceCatalog(selectSources);
  const selectedSourceId = useSourceCatalog(selectSelectedSourceId);
  const tables = useSchemaExplorer(selectTables);
  const tableCount = useSchemaExplorer(selectTableCount);
  const knowledgeCount = useKnowledgeBase(selectKnowledgeCount);
  const documentCount = useKnowledgeBase(selectDocumentCount);
  const activeWorkspaceId = useWorkspace(selectActiveWorkspaceId);
  const { setDocuments, setKnowledgeItems } = useKnowledgeBaseActions();
  const { setTables } = useSchemaExplorerActions();
  const loadRequestIdRef = useRef(0);
  const loadTargetKeyRef = useRef("");
  const [isSummaryLoading, setIsSummaryLoading] = useState(false);
  const effectiveSourceId =
    selectedSourceId ?? (sources.length === 1 ? (sources[0]?.id ?? null) : null);
  const hasConnectTarget = effectiveSourceId !== null;
  const shouldShowOverview =
    hasConnectTarget || tableCount > 0 || knowledgeCount > 0 || documentCount > 0;

  useEffect(() => {
    const requestId = loadRequestIdRef.current + 1;
    const requestTargetKey = `${activeWorkspaceId}:${effectiveSourceId ?? "none"}`;
    loadRequestIdRef.current = requestId;
    loadTargetKeyRef.current = requestTargetKey;
    let cancelled = false;

    setTables([]);
    setDocuments([]);
    setKnowledgeItems([]);

    if (!hasConnectTarget) {
      setIsSummaryLoading(false);
      return () => {
        cancelled = true;
      };
    }

    const load = async () => {
      setIsSummaryLoading(true);
      const [tablesResult, documentsResult, knowledgeResult] = await Promise.all([
        adapter.listTables(scope, { sourceId: effectiveSourceId ?? undefined }),
        adapter.listDocuments(scope),
        adapter.browseKnowledge(scope, { sourceId: effectiveSourceId ?? undefined }),
      ]);

      if (
        cancelled ||
        loadRequestIdRef.current !== requestId ||
        loadTargetKeyRef.current !== requestTargetKey
      ) {
        return;
      }

      if (isOk(tablesResult)) {
        setTables(tablesResult.value.map(tableSummaryToTableInfo));
      }
      if (isOk(documentsResult)) {
        setDocuments(documentsResult.value.map(documentSummaryToDocumentItem));
      }
      if (isOk(knowledgeResult)) {
        setKnowledgeItems(knowledgeResult.value.map(knowledgeSummaryToKnowledgeItem));
      }
      setIsSummaryLoading(false);
    };

    void load();

    return () => {
      cancelled = true;
    };
  }, [
    activeWorkspaceId,
    adapter,
    effectiveSourceId,
    hasConnectTarget,
    scope,
    setDocuments,
    setKnowledgeItems,
    setTables,
  ]);

  if (sources.length === 0) {
    return (
      <EmptyState
        icon={DatabaseIcon}
        title="No data sources configured"
        description="Attach a data_environment to this workspace runtime to enable discovery, profiling, and knowledge ingestion."
      />
    );
  }

  return (
    <div className="flex-1 overflow-y-auto scroll-container">
      <div className="max-w-3xl mx-auto p-6 space-y-6">
        {/* Header */}
        <div>
          <h1 className="text-lg font-semibold">Connect</h1>
          <p className="text-sm text-muted-foreground">
            Manage your data sources, schema, and knowledge
          </p>
        </div>

        {/* Overview stats */}
        {shouldShowOverview && (
          <div className="grid grid-cols-3 gap-3">
            <StatCard value={tableCount} label="Tables" isLoading={isSummaryLoading} />
            <StatCard value={documentCount} label="Documents" isLoading={isSummaryLoading} />
            <StatCard value={knowledgeCount} label="Knowledge Items" isLoading={isSummaryLoading} />
          </div>
        )}

        {/* Quick actions grid */}
        <div className="space-y-2">
          <h2 className="section-label">Quick Actions</h2>
          <div className="grid gap-2">
            {QUICK_ACTIONS.map((action) => (
              <ConnectActionLink key={action.to} {...action} />
            ))}
            <ExploreInChatCTA
              tableCount={tableCount}
              tableNames={tables.map((t) => t.tableName)}
              knowledgeCount={knowledgeCount}
              documentCount={documentCount}
            />
          </div>
        </div>
      </div>
    </div>
  );
}
