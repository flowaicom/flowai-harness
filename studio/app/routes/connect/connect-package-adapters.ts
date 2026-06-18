import type {
  CatalogSearchResult,
  DocumentSummary,
  KnowledgeItem,
  MetricSummary,
  TableDetail,
  TableSummary,
  ToolExecutionResult,
  ToolSummary,
} from "@studio/core/runtime";
import type {
  ConnectDiscoveryDetailLike,
  ConnectDiscoveryTableLike,
  ConnectDocumentLike,
  ConnectKnowledgeItemLike,
  ConnectKnowledgeType,
  ConnectMetricItemLike,
  ConnectProfilingSummaryLike,
  ConnectProfilingTableStageLike,
  ConnectProfilingTableStageStatus,
  ConnectRelationshipsDataLike,
  ConnectRuntimeResult,
  ConnectSearchResultsLike,
  ConnectToolInfoLike,
  ConnectToolResultLike,
} from "@studio/features-connect";
import type { PhysicalTable, TableInfo, TableType } from "~/lib/domain/data";

export function connectScopeKey(input: unknown): string {
  return JSON.stringify(input);
}

export function mapConnectResult<TValue, TNext>(
  result: ConnectRuntimeResult<TValue>,
  mapValue: (value: TValue) => TNext
): ConnectRuntimeResult<TNext> {
  if (result._tag === "Ok") {
    return { _tag: "Ok", value: mapValue(result.value) };
  }
  return result;
}

export function connectUnavailable(message: string): ConnectRuntimeResult<never> {
  return { _tag: "Err", error: { message } };
}

export function tableSummaryToConnectTable(table: TableSummary): ConnectDiscoveryTableLike {
  return {
    schemaName: table.schemaName ?? "public",
    tableName: table.tableName,
    tableType: table.metadata?.tableType === "view" ? "view" : "base_table",
    rowCount: numberMetadata(table.metadata, "rowCount"),
    columnCount: table.columnCount ?? null,
    description: stringMetadata(table.metadata, "description"),
  };
}

export function connectTableToTableInfo(table: ConnectDiscoveryTableLike): TableInfo {
  return {
    schemaName: table.schemaName,
    tableName: table.tableName,
    tableType: asTableType(table.tableType),
    rowCount: table.rowCount,
    columnCount: table.columnCount ?? null,
    description: table.description,
  };
}

export function tableDetailToConnectDetail(table: TableDetail): ConnectDiscoveryDetailLike {
  return {
    schemaName: table.schemaName ?? "public",
    tableName: table.tableName,
    columns: (table.columns ?? []).map(columnToConnectColumn),
    constraints: [],
    indexes: [],
    rowCount: numberMetadata(table.metadata, "rowCount"),
  };
}

export function connectDetailToPhysicalTable(table: ConnectDiscoveryDetailLike): PhysicalTable {
  return {
    schemaName: table.schemaName,
    tableName: table.tableName,
    rowCount: table.rowCount,
    columns: [...table.columns],
    constraints: [...table.constraints],
    indexes: [...table.indexes],
  };
}

export function toolSummaryToConnectTool(tool: ToolSummary): ConnectToolInfoLike {
  return {
    id: tool.toolId,
    name: tool.name,
    description: tool.description ?? "",
    parameters: tool.inputSchema ?? {},
  };
}

export function toolExecutionToConnectResult(
  execution: ToolExecutionResult
): ConnectToolResultLike {
  const outputObject = objectValue(execution.output);
  return {
    success: execution.status !== "failed" && execution.status !== "error",
    data:
      typeof execution.output === "string"
        ? execution.output
        : JSON.stringify(execution.output ?? {}, null, 2),
    count: typeof outputObject.count === "number" ? outputObject.count : null,
    error:
      typeof outputObject.error === "string"
        ? outputObject.error
        : execution.status === "failed" || execution.status === "error"
          ? execution.status
          : null,
  };
}

export function toolOutputToConnectRelationships(
  tableName: string,
  output: unknown
): ConnectRelationshipsDataLike {
  const value = objectValue(output);
  if (Array.isArray(value.results)) {
    return graphRelationsToRelationships(tableName, value.results);
  }

  const related = Array.isArray(value.relatedTables) ? value.relatedTables : [];
  const references = related.map((item) => {
    const rel = objectValue(item);
    return {
      tableName: String(rel.name ?? rel.tableName ?? rel.id ?? ""),
      relationshipType: String(rel.relationType ?? "related"),
      sourceColumn: "",
      targetColumn: "",
    };
  });
  return {
    tableName,
    references,
    referencedBy: [],
    totalCount: references.length,
  };
}

export function catalogSearchToConnectResults(
  result: CatalogSearchResult
): ConnectSearchResultsLike {
  const items = result.items.map((item) => {
    const value = objectValue(item);
    return {
      id: String(value.id ?? ""),
      name: String(value.name ?? ""),
      itemType: String(value.itemType ?? value.item_type ?? "unknown"),
      description: typeof value.description === "string" ? value.description : null,
      tags: stringList(value.tags),
      score: typeof value.score === "number" ? value.score : 0,
    };
  });
  return {
    items,
    totalCount: numberMetadata(result.metadata, "totalCount") ?? items.length,
    queryTimeMs: numberMetadata(result.metadata, "queryTimeMs") ?? 0,
  };
}

export function documentSummaryToConnectDocument(document: DocumentSummary): ConnectDocumentLike {
  const metadata = document.metadata ?? {};
  const extractionStatus = metadata.extractionStatus;
  return {
    id: document.documentId,
    name: document.title,
    content: typeof metadata.content === "string" ? metadata.content : "",
    authority: asAuthority(metadata.authority),
    targetDatabaseId: document.sourceId ?? null,
    extractionStatus:
      extractionStatus === "processing" ||
      extractionStatus === "processed" ||
      extractionStatus === "failed"
        ? extractionStatus
        : "pending",
    extractedKnowledgeIds: stringList(metadata.extractedKnowledgeIds),
    createdAt: typeof metadata.createdAt === "string" ? metadata.createdAt : "1970-01-01T00:00:00Z",
  };
}

export function knowledgeItemToConnectKnowledgeItem(item: KnowledgeItem): ConnectKnowledgeItemLike {
  const metadata = item.metadata ?? {};
  const knowledgeType = metadata.knowledgeType;
  return {
    id: item.itemId,
    name: item.title ?? item.itemId,
    description: item.content ?? "",
    authority: asAuthority(metadata.authority),
    knowledgeType: asKnowledgeType(knowledgeType),
    scopeTables: stringList(metadata.scopeTables),
    scopeColumns: stringList(metadata.scopeColumns),
    sqlExpression: typeof metadata.sqlExpression === "string" ? metadata.sqlExpression : null,
    synonyms: stringList(metadata.synonyms),
    sourceDocumentId:
      typeof metadata.sourceDocumentId === "string" ? metadata.sourceDocumentId : null,
  };
}

export function metricSummaryToConnectMetric(metric: MetricSummary): ConnectMetricItemLike {
  const metadata = metric.metadata ?? {};
  return {
    id: metric.metricId,
    name: metric.name,
    displayName: stringMetadata(metadata, "displayName") ?? metric.name,
    formula: stringMetadata(metadata, "formula") ?? "",
    formulaDescription: metric.description ?? stringMetadata(metadata, "formulaDescription") ?? "",
    sourceTables: stringList(metadata.sourceTables),
    sourceColumns: stringList(metadata.sourceColumns),
    aggregationType: stringMetadata(metadata, "aggregationType") ?? metric.metricType ?? "custom",
    timeGrain: stringMetadata(metadata, "timeGrain"),
    outputType: stringMetadata(metadata, "outputType") ?? "number",
  };
}

export function profilingSummaryFromUnknown(input: unknown): ConnectProfilingSummaryLike | null {
  const value = objectValue(input);
  if (Object.keys(value).length === 0) {
    return null;
  }
  return {
    tablesDiscovered: numberValue(value.tablesDiscovered) ?? 0,
    columnsProfiled: numberValue(value.columnsProfiled) ?? 0,
    enumsExtracted: numberValue(value.enumsExtracted) ?? 0,
    relationshipsFound: numberValue(value.relationshipsFound) ?? 0,
    catalogItemsIndexed: numberValue(value.catalogItemsIndexed) ?? 0,
    durationMs: numberValue(value.durationMs) ?? 0,
    enrichmentCacheHits: numberValue(value.enrichmentCacheHits) ?? 0,
    enrichmentFallbacks: numberValue(value.enrichmentFallbacks) ?? 0,
    enrichmentFresh: numberValue(value.enrichmentFresh) ?? 0,
  };
}

export function tableStagesToConnectTableStages(
  tableStages: ReadonlyMap<
    string,
    {
      readonly tableName: string;
      readonly stages: Readonly<Record<string, string>>;
      readonly columns: number;
      readonly durationMs: number;
      readonly enrichmentSource?: string;
    }
  >
): ReadonlyMap<string, ConnectProfilingTableStageLike> {
  const next = new Map<string, ConnectProfilingTableStageLike>();
  for (const [key, entry] of tableStages.entries()) {
    next.set(key, {
      tableName: entry.tableName,
      columns: entry.columns,
      durationMs: entry.durationMs,
      enrichmentSource:
        entry.enrichmentSource === "fresh" ||
        entry.enrichmentSource === "cached" ||
        entry.enrichmentSource === "fallback"
          ? entry.enrichmentSource
          : undefined,
      stages: {
        discovering: asStageStatus(entry.stages.discovering),
        profiling: asStageStatus(entry.stages.profiling),
        enriching: asStageStatus(entry.stages.enriching),
        extracting: asStageStatus(entry.stages.extracting),
        indexing: asStageStatus(entry.stages.indexing),
      },
    });
  }
  return next;
}

function columnToConnectColumn(input: unknown, index: number) {
  const value = objectValue(input);
  const foreignKey = objectValue(value.foreignKey);
  return {
    columnName: String(value.columnName ?? value.name ?? `column_${index + 1}`),
    dataType: String(value.dataType ?? value.data_type ?? "unknown"),
    isNullable: Boolean(value.isNullable ?? value.nullable ?? false),
    columnDefault: typeof value.columnDefault === "string" ? value.columnDefault : null,
    ordinalPosition: typeof value.ordinalPosition === "number" ? value.ordinalPosition : index + 1,
    isPrimaryKey: Boolean(value.isPrimaryKey ?? value.primaryKey ?? false),
    foreignKey:
      Object.keys(foreignKey).length > 0
        ? {
            referencedSchema: String(foreignKey.referencedSchema ?? "public"),
            referencedTable: String(foreignKey.referencedTable ?? ""),
            referencedColumn: String(foreignKey.referencedColumn ?? ""),
            constraintName: String(foreignKey.constraintName ?? ""),
          }
        : null,
  };
}

function graphRelationsToRelationships(
  tableName: string,
  results: readonly unknown[]
): ConnectRelationshipsDataLike {
  const references: ConnectRelationshipsDataLike["references"][number][] = [];
  const referencedBy: ConnectRelationshipsDataLike["referencedBy"][number][] = [];
  for (const result of results) {
    const relations = objectValue(result).relations;
    if (!Array.isArray(relations)) {
      continue;
    }
    for (const item of relations) {
      const rel = objectValue(item);
      const row = graphRelationToRow(rel);
      if (String(rel.direction ?? "outgoing") === "incoming") {
        referencedBy.push(row);
      } else {
        references.push(row);
      }
    }
  }
  return {
    tableName,
    references,
    referencedBy,
    totalCount: references.length + referencedBy.length,
  };
}

function graphRelationToRow(rel: Record<string, unknown>) {
  const target = objectValue(rel.target);
  return {
    tableName: String(target.name ?? target.qualified_name ?? target.id ?? ""),
    relationshipType: String(rel.relation_kind ?? rel.relationKind ?? "related"),
    sourceColumn: "",
    targetColumn: "",
  };
}

function objectValue(input: unknown): Record<string, unknown> {
  return typeof input === "object" && input !== null ? (input as Record<string, unknown>) : {};
}

function stringMetadata(metadata: Record<string, unknown> | undefined, key: string): string | null {
  const value = metadata?.[key];
  return typeof value === "string" ? value : null;
}

function numberMetadata(metadata: Record<string, unknown> | undefined, key: string): number | null {
  return numberValue(metadata?.[key]);
}

function numberValue(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function stringList(value: unknown): readonly string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string")
    : [];
}

function asTableType(value: string): TableType {
  if (
    value === "base_table" ||
    value === "view" ||
    value === "materialized_view" ||
    value === "foreign"
  ) {
    return value;
  }
  return "base_table";
}

function asAuthority(value: unknown): "workspace" | "catalog" | undefined {
  return value === "workspace" || value === "catalog" ? value : undefined;
}

function asKnowledgeType(value: unknown): ConnectKnowledgeType {
  if (
    value === "business_rule" ||
    value === "predicate" ||
    value === "terminology" ||
    value === "constraint" ||
    value === "temporal_rule" ||
    value === "implicit_intent" ||
    value === "data_quality" ||
    value === "custom"
  ) {
    return value;
  }
  return "custom";
}

function asStageStatus(value: unknown): ConnectProfilingTableStageStatus {
  if (value === "active" || value === "completed" || value === "failed") {
    return value;
  }
  return "queued";
}
