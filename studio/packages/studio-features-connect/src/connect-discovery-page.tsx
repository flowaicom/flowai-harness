import type { AppScope } from "@studio/core/domain/scope";
import {
  ArrowDownLeftIcon,
  ArrowUpRightIcon,
  BarChart3Icon,
  EyeIcon,
  LinkIcon,
  MessageSquareIcon,
  TableIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  buildConnectTableExplorePrompt,
  loadConnectDiscoveryTables,
} from "./connect-discovery-model";
import {
  ConnectCategoryBadge,
  ConnectEmptyState,
  ConnectErrorBanner,
  ConnectPillTabs,
  ConnectSectionCard,
  ConnectSectionHeader,
} from "./connect-page-primitives";
import { type ConnectRuntimeResult, isConnectRuntimeOk } from "./connect-page-types";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

interface ConnectRelatedTableInfoLike {
  readonly tableName: string;
  readonly relationshipType: string;
  readonly sourceColumn: string;
  readonly targetColumn: string;
}

export interface ConnectRelationshipsDataLike {
  readonly tableName: string;
  readonly references: readonly ConnectRelatedTableInfoLike[];
  readonly referencedBy: readonly ConnectRelatedTableInfoLike[];
  readonly totalCount: number;
}

export interface ConnectDiscoveryTableLike {
  readonly schemaName: string;
  readonly tableName: string;
  readonly tableType: string;
  readonly rowCount: number | null;
  readonly columnCount?: number | null;
  readonly description: string | null;
}

interface ConnectDiscoveryColumnLike {
  readonly columnName: string;
  readonly dataType: string;
  readonly isNullable: boolean;
  readonly columnDefault: string | null;
  readonly ordinalPosition: number;
  readonly isPrimaryKey: boolean;
  readonly foreignKey: {
    readonly referencedSchema: string;
    readonly referencedTable: string;
    readonly referencedColumn: string;
    readonly constraintName: string;
  } | null;
}

interface ConnectDiscoveryConstraintLike {
  readonly name: string;
  readonly constraintType: string;
  readonly columns: readonly string[];
}

interface ConnectDiscoveryIndexLike {
  readonly name: string;
  readonly isUnique: boolean;
  readonly columns: readonly string[];
}

export interface ConnectDiscoveryDetailLike {
  readonly schemaName: string;
  readonly tableName: string;
  readonly rowCount: number | null;
  readonly columns: readonly ConnectDiscoveryColumnLike[];
  readonly constraints: readonly ConnectDiscoveryConstraintLike[];
  readonly indexes: readonly ConnectDiscoveryIndexLike[];
}

export interface ConnectDiscoveryRuntimeLike {
  listTables(
    scope: AppScope,
    params?: { readonly schema?: string; readonly signal?: AbortSignal }
  ): Promise<ConnectRuntimeResult<readonly ConnectDiscoveryTableLike[]>>;
  getTableDetail(
    scope: AppScope,
    tableName: string,
    params?: { readonly schema?: string }
  ): Promise<ConnectRuntimeResult<ConnectDiscoveryDetailLike>>;
}

interface ConnectDiscoveryEmptyState {
  readonly title: string;
  readonly description: string;
  readonly action?: { readonly label: string; readonly onClick: () => void };
}

type DetailTab = "columns" | "relationships";

export interface ConnectDiscoveryPageProps {
  readonly scope: AppScope;
  readonly scopeKey: string;
  readonly hasTarget: boolean;
  readonly runtime: ConnectDiscoveryRuntimeLike;
  readonly tables: readonly ConnectDiscoveryTableLike[];
  readonly tableDetail: ConnectDiscoveryDetailLike | null;
  readonly setTables: (tables: ConnectDiscoveryTableLike[]) => void;
  readonly setTableDetail: (tableDetail: ConnectDiscoveryDetailLike | null) => void;
  readonly loadRelationships: (
    scope: AppScope,
    tableName: string
  ) => Promise<ConnectRuntimeResult<ConnectRelationshipsDataLike>>;
  readonly emptyState: ConnectDiscoveryEmptyState;
  readonly onExploreTable?: (args: {
    readonly detail: ConnectDiscoveryDetailLike;
    readonly title: string;
    readonly prompt: string;
  }) => Promise<void> | void;
  readonly exploreLabel?: string;
  readonly headerAccessory?: ReactNode;
  readonly subtitle?: ReactNode;
  readonly targetMeta?: ReactNode;
  readonly detailSecondaryAction?: ReactNode | ((detail: ConnectDiscoveryDetailLike) => ReactNode);
  readonly formatText?: (value: string) => string;
}

export function ConnectDiscoveryPage({
  scope,
  scopeKey,
  hasTarget,
  runtime,
  tables,
  tableDetail,
  setTables,
  setTableDetail,
  loadRelationships,
  emptyState,
  onExploreTable,
  exploreLabel = "Explore",
  headerAccessory,
  subtitle = "Browse tables and columns in your database",
  targetMeta,
  detailSecondaryAction,
  formatText = (value) => value,
}: ConnectDiscoveryPageProps) {
  const [selectedTable, setSelectedTable] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [detailTab, setDetailTab] = useState<DetailTab>("columns");
  const [error, setError] = useState<string | null>(null);
  const exploreBusyRef = useRef(false);
  const detailRequestIdRef = useRef(0);

  useEffect(() => {
    detailRequestIdRef.current += 1;
    const controller = new AbortController();
    setIsLoading(true);
    setError(null);
    setSelectedTable(null);
    setDetailTab("columns");
    setTables([]);
    setTableDetail(null);
    void loadConnectDiscoveryTables({
      hasTarget,
      signal: controller.signal,
      loadTables: ({ signal }) => runtime.listTables(scope, { signal }),
      errorMessage: "Failed to load tables",
    }).then((outcome) => {
      if (outcome.kind === "aborted") {
        return;
      }
      setIsLoading(false);
      if (outcome.kind === "success") {
        setTables([...outcome.tables]);
        return;
      }
      if (outcome.kind === "error") {
        setError(outcome.message);
      }
    });

    return () => {
      controller.abort();
    };
  }, [hasTarget, runtime, scope, scopeKey, setTableDetail, setTables]);

  const handleViewDetail = useCallback(
    async (tableName: string) => {
      const requestId = detailRequestIdRef.current + 1;
      detailRequestIdRef.current = requestId;
      setSelectedTable(tableName);
      setDetailTab("columns");
      const result = await runtime.getTableDetail(scope, tableName);
      if (!isConnectRuntimeOk(result)) {
        return;
      }
      if (detailRequestIdRef.current !== requestId) {
        return;
      }
      setTableDetail(result.value);
    },
    [runtime, scope, setTableDetail]
  );

  const detailTabs = useMemo(
    () => [
      { id: "columns" as const, label: "Columns", count: tableDetail?.columns.length },
      { id: "relationships" as const, label: "Relationships" },
    ],
    [tableDetail?.columns.length]
  );

  const renderDetailSecondaryAction = useMemo(() => {
    if (!tableDetail) {
      return null;
    }
    return typeof detailSecondaryAction === "function"
      ? detailSecondaryAction(tableDetail)
      : detailSecondaryAction;
  }, [detailSecondaryAction, tableDetail]);

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      <div className="px-6 py-4 border-b">
        <div className={headerAccessory ? "flex items-center justify-between mb-1" : undefined}>
          <h1 className="text-lg font-semibold">Discovery</h1>
          {headerAccessory}
        </div>
        <p className="text-sm text-muted-foreground">{subtitle}</p>
        {targetMeta ? <div className="text-xs text-muted-foreground mt-1">{targetMeta}</div> : null}
      </div>

      {error ? (
        <div className="px-6 pt-3">
          <ConnectErrorBanner message={error} onDismiss={() => setError(null)} />
        </div>
      ) : null}

      <div className="flex flex-1 overflow-hidden">
        <div className="w-72 border-r overflow-y-auto scroll-container">
          {isLoading ? (
            <div className="p-3 space-y-1">
              {Array.from({ length: 8 }, (_, index) => (
                <div
                  key={`discovery-skeleton-${index}`}
                  className="flex items-center gap-2 px-3 py-2"
                  style={{ animationDelay: `${index * 50}ms` }}
                >
                  <div className="size-4 rounded bg-muted animate-pulse" />
                  <div className="flex-1 space-y-1">
                    <div
                      className="h-3.5 bg-muted rounded animate-pulse"
                      style={{ width: `${75 - index * 5}%` }}
                    />
                    <div className="h-2.5 bg-muted/50 rounded animate-pulse w-1/3" />
                  </div>
                </div>
              ))}
            </div>
          ) : tables.length === 0 ? (
            <div className="p-6">
              <ConnectEmptyState
                icon={TableIcon}
                title={emptyState.title}
                description={emptyState.description}
                action={emptyState.action}
              />
            </div>
          ) : (
            <div className="p-2 space-y-0.5">
              {tables.map((table) => (
                <button
                  key={`${table.schemaName}.${table.tableName}`}
                  type="button"
                  onClick={() => handleViewDetail(table.tableName)}
                  aria-label={`View ${table.tableName} schema${table.rowCount != null ? ` (${table.rowCount.toLocaleString()} rows)` : ""}`}
                  aria-current={selectedTable === table.tableName ? "true" : undefined}
                  className={cx(
                    "w-full flex items-center gap-2 px-3 py-2 rounded-md text-left transition-colors focus-visible:ring-2 focus-visible:ring-primary/50 focus-visible:outline-none",
                    selectedTable === table.tableName
                      ? "bg-primary/10 text-primary"
                      : "hover:bg-muted text-foreground"
                  )}
                >
                  <TableIcon className="size-3.5 text-muted-foreground flex-shrink-0" />
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-mono truncate">{formatText(table.tableName)}</div>
                    <div className="text-[10px] text-muted-foreground">
                      {table.tableType}
                      {table.rowCount != null ? (
                        <span className="ml-1 font-mono tabular-nums">
                          {table.rowCount.toLocaleString()} rows
                        </span>
                      ) : null}
                    </div>
                  </div>
                </button>
              ))}
            </div>
          )}
        </div>

        <div className="flex-1 overflow-y-auto scroll-container">
          {tableDetail ? (
            <div>
              <div className="px-6 py-4 border-b">
                <div className="flex items-center justify-between mb-3">
                  <div>
                    <h2 className="text-base font-semibold font-mono">
                      {formatText(tableDetail.schemaName)}.{formatText(tableDetail.tableName)}
                    </h2>
                    {tableDetail.rowCount != null ? (
                      <p className="text-xs text-muted-foreground font-mono tabular-nums">
                        {tableDetail.rowCount.toLocaleString()} rows
                      </p>
                    ) : null}
                  </div>
                  <div className="flex items-center gap-2">
                    {onExploreTable ? (
                      <button
                        type="button"
                        onClick={async () => {
                          if (exploreBusyRef.current) {
                            return;
                          }
                          exploreBusyRef.current = true;
                          try {
                            const title = `Explore: ${tableDetail.tableName}`;
                            const prompt = buildConnectTableExplorePrompt({
                              schemaName: tableDetail.schemaName,
                              tableName: tableDetail.tableName,
                              columnNames: tableDetail.columns.map((column) => column.columnName),
                              totalColumnCount: tableDetail.columns.length,
                            });
                            await onExploreTable({ detail: tableDetail, title, prompt });
                          } finally {
                            exploreBusyRef.current = false;
                          }
                        }}
                        className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs text-muted-foreground hover:bg-muted hover:text-foreground transition-colors border"
                      >
                        <MessageSquareIcon className="size-3.5" />
                        {exploreLabel}
                      </button>
                    ) : null}
                    {renderDetailSecondaryAction}
                  </div>
                </div>
                <ConnectPillTabs tabs={detailTabs} active={detailTab} onChange={setDetailTab} />
              </div>

              <div className="max-w-4xl p-6 space-y-6">
                {detailTab === "columns" ? (
                  <ColumnsView table={tableDetail} formatText={formatText} />
                ) : (
                  <RelationshipsView
                    scope={scope}
                    tableName={tableDetail.tableName}
                    currentTable={tableDetail.tableName}
                    loadRelationships={loadRelationships}
                    onNavigate={handleViewDetail}
                    formatText={formatText}
                  />
                )}
              </div>
            </div>
          ) : (
            <ConnectEmptyState
              icon={EyeIcon}
              title="Select a table"
              description="Choose a table from the list to view its schema"
              className="!items-start pt-10"
            />
          )}
        </div>
      </div>
    </div>
  );
}

function ColumnsView({
  table,
  formatText,
}: {
  readonly table: ConnectDiscoveryDetailLike;
  readonly formatText: (value: string) => string;
}) {
  return (
    <>
      <ConnectSectionCard>
        <ConnectSectionHeader>Columns ({table.columns.length})</ConnectSectionHeader>
        <div className="divide-y">
          {table.columns.map((column) => {
            const badges = columnKeyBadges(column);
            return (
              <div
                key={column.columnName}
                className="flex items-center gap-3 py-2.5 first:pt-0 last:pb-0"
              >
                <span className="font-mono text-xs w-40 shrink-0 truncate">
                  {formatText(column.columnName)}
                </span>
                <span className="text-xs text-muted-foreground w-28 shrink-0 truncate">
                  {column.dataType}
                </span>
                <span className="text-xs text-muted-foreground w-8 shrink-0">
                  {column.isNullable ? "null" : ""}
                </span>
                <div className="flex items-center gap-1 flex-1 justify-end">
                  {badges.map((badge) => (
                    <ConnectCategoryBadge
                      key={badge.label}
                      label={badge.label}
                      category={badge.category}
                    />
                  ))}
                  {column.foreignKey ? (
                    <span className="text-[10px] text-muted-foreground font-mono ml-1 truncate max-w-[160px]">
                      → {formatText(column.foreignKey.referencedTable)}.
                      {formatText(column.foreignKey.referencedColumn)}
                    </span>
                  ) : null}
                </div>
              </div>
            );
          })}
        </div>
      </ConnectSectionCard>

      {table.constraints.length > 0 ? (
        <ConnectSectionCard>
          <ConnectSectionHeader>Constraints ({table.constraints.length})</ConnectSectionHeader>
          <div className="space-y-1.5">
            {table.constraints.map((constraint) => (
              <div key={constraint.name} className="flex items-center gap-2 text-xs">
                <span className="font-medium truncate">{formatText(constraint.name)}</span>
                <ConnectCategoryBadge label={constraint.constraintType} category="planning" />
                <span className="text-muted-foreground font-mono truncate">
                  ({constraint.columns.map(formatText).join(", ")})
                </span>
              </div>
            ))}
          </div>
        </ConnectSectionCard>
      ) : null}

      {table.indexes.length > 0 ? (
        <ConnectSectionCard>
          <ConnectSectionHeader>Indexes ({table.indexes.length})</ConnectSectionHeader>
          <div className="space-y-1.5">
            {table.indexes.map((index) => (
              <div key={index.name} className="flex items-center gap-2 text-xs">
                <span className="font-medium truncate">{formatText(index.name)}</span>
                {index.isUnique ? (
                  <ConnectCategoryBadge label="UNIQUE" category="knowledge" />
                ) : null}
                <span className="text-muted-foreground font-mono truncate">
                  ({index.columns.map(formatText).join(", ")})
                </span>
              </div>
            ))}
          </div>
        </ConnectSectionCard>
      ) : null}
    </>
  );
}

function RelationshipsView({
  scope,
  tableName,
  currentTable,
  loadRelationships,
  onNavigate,
  formatText,
}: {
  readonly scope: AppScope;
  readonly tableName: string;
  readonly currentTable: string;
  readonly loadRelationships: (
    scope: AppScope,
    tableName: string
  ) => Promise<ConnectRuntimeResult<ConnectRelationshipsDataLike>>;
  readonly onNavigate: (tableName: string) => void;
  readonly formatText: (value: string) => string;
}) {
  const [data, setData] = useState<ConnectRelationshipsDataLike | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestIdRef = useRef(0);

  useEffect(() => {
    const requestId = requestIdRef.current + 1;
    requestIdRef.current = requestId;
    setIsLoading(true);
    setError(null);
    setData(null);

    void loadRelationships(scope, tableName).then((result) => {
      if (requestIdRef.current !== requestId) {
        return;
      }
      setIsLoading(false);
      if (isConnectRuntimeOk(result)) {
        setData(result.value);
      } else {
        setError(result.error.message);
      }
    });
  }, [loadRelationships, scope, tableName]);

  if (isLoading) {
    return (
      <ConnectSectionCard>
        <ConnectSectionHeader>Relationships</ConnectSectionHeader>
        <div className="space-y-2">
          {Array.from({ length: 3 }, (_, index) => (
            <div
              key={`relationship-skeleton-${index}`}
              className="flex items-center gap-3 py-2"
              style={{ animationDelay: `${index * 75}ms` }}
            >
              <div className="size-4 rounded bg-muted animate-pulse" />
              <div className="flex-1 space-y-1">
                <div
                  className="h-3.5 bg-muted rounded animate-pulse"
                  style={{ width: `${60 - index * 10}%` }}
                />
                <div className="h-2.5 bg-muted/50 rounded animate-pulse w-2/5" />
              </div>
            </div>
          ))}
        </div>
      </ConnectSectionCard>
    );
  }

  if (error) {
    return <ConnectErrorBanner message={error} onDismiss={() => setError(null)} />;
  }

  const hasReferences = data?.references.length ?? 0;
  const hasReferencedBy = data?.referencedBy.length ?? 0;
  if (!data || (!hasReferences && !hasReferencedBy)) {
    return (
      <ConnectEmptyState
        icon={LinkIcon}
        title="No relationships"
        description="This table has no foreign key relationships"
      />
    );
  }

  return (
    <>
      {hasReferencedBy > 0 ? (
        <ConnectSectionCard>
          <ConnectSectionHeader>Referenced By ({data.referencedBy.length})</ConnectSectionHeader>
          <div className="space-y-1.5">
            {data.referencedBy.map((relationship) => (
              <RelationshipRow
                key={`by-${relationship.tableName}-${relationship.sourceColumn}`}
                rel={relationship}
                direction="inbound"
                currentTable={currentTable}
                onNavigate={onNavigate}
                formatText={formatText}
              />
            ))}
          </div>
        </ConnectSectionCard>
      ) : null}

      {hasReferences > 0 ? (
        <ConnectSectionCard>
          <ConnectSectionHeader>References ({data.references.length})</ConnectSectionHeader>
          <div className="space-y-1.5">
            {data.references.map((relationship) => (
              <RelationshipRow
                key={`ref-${relationship.tableName}-${relationship.sourceColumn}`}
                rel={relationship}
                direction="outbound"
                currentTable={currentTable}
                onNavigate={onNavigate}
                formatText={formatText}
              />
            ))}
          </div>
        </ConnectSectionCard>
      ) : null}
    </>
  );
}

function RelationshipRow({
  rel,
  direction,
  currentTable,
  onNavigate,
  formatText,
}: {
  readonly rel: ConnectRelatedTableInfoLike;
  readonly direction: "inbound" | "outbound";
  readonly currentTable: string;
  readonly onNavigate: (tableName: string) => void;
  readonly formatText: (value: string) => string;
}) {
  const Icon = direction === "inbound" ? ArrowDownLeftIcon : ArrowUpRightIcon;
  const category = direction === "inbound" ? "discovery" : "knowledge";

  return (
    <div className="flex items-center gap-2.5 py-1.5">
      <Icon className="size-3.5 text-muted-foreground shrink-0" />
      <button
        type="button"
        onClick={() => onNavigate(rel.tableName)}
        className="text-sm font-medium text-primary hover:underline truncate focus-visible:ring-2 focus-visible:ring-primary/50 focus-visible:outline-none rounded"
      >
        {formatText(rel.tableName)}
      </button>
      <ConnectCategoryBadge label={rel.relationshipType} category={category} />
      <span className="text-[10px] text-muted-foreground font-mono truncate ml-auto">
        {direction === "inbound"
          ? `${formatText(rel.tableName)}.${formatText(rel.targetColumn)} → ${formatText(currentTable)}.${formatText(rel.sourceColumn)}`
          : `${formatText(currentTable)}.${formatText(rel.sourceColumn)} → ${formatText(rel.tableName)}.${formatText(rel.targetColumn)}`}
      </span>
    </div>
  );
}

function columnKeyBadges(
  column: ConnectDiscoveryDetailLike["columns"][number]
): Array<{ readonly label: string; readonly category: string }> {
  const badges: Array<{ readonly label: string; readonly category: string }> = [];
  if (column.isPrimaryKey) {
    badges.push({ label: "PK", category: "execution" });
  }
  if (column.foreignKey) {
    badges.push({ label: "FK", category: "discovery" });
  }
  return badges;
}
