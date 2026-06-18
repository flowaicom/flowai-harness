/**
 * Data API functions with Result-based error handling.
 *
 * Reuses get/post/put/del from client.ts and shared SSE reader from sse.ts.
 *
 * @module api/data
 */

import type {
  AddKnowledgeRequest,
  CatalogSearchResult,
  ColumnInfo,
  ConnectionTestResult,
  CreateDataSourceRequest,
  CreateMetricRequest,
  DatabaseStatusSummary,
  DataReadiness,
  DataSearchResult,
  DataSource,
  DocumentItem,
  ImportEvent,
  ImportStage,
  IngestDocumentEntry,
  IngestionEvent,
  IngestionStatus,
  JoinPath,
  KnowledgeIngestEvent,
  KnowledgeItem,
  KnowledgeSourceSpec,
  MetricItem,
  MigrationResult,
  PhysicalTable,
  ProfilingCostEstimate,
  PurgeResult,
  TableInfo,
  TermResolution,
  ToolInfo,
  ToolResult,
  UpdateDataSourceRequest,
} from "~/lib/domain/data";
import type { Result } from "~/lib/domain/result";
import { err, isOk, ok } from "~/lib/domain/result";
import { getFlowAIStudioConfig } from "~/lib/studio-config/flowai-config";
import type { ApiError } from "./client";
import { del, get, getApiConfig, post, put } from "./client";
import { DataReadinessSchema, ImportStageSchema } from "./schemas";
import type { JsonSSEHandlers } from "./sse";
import { readJsonSSEStream, startJsonSSEStream } from "./sse";
import { validateBoundary } from "./validation";

// =============================================================================
// Data Sources CRUD
// =============================================================================

/** List all data sources. */
export const listDataSources = (): Promise<Result<DataSource[], ApiError>> =>
  get<{ sources: DataSource[] }>(workspaceDataPath("sources")).then((result) =>
    isOk(result) ? ok(result.value.sources) : result
  );

/** Get a single data source. */
export const getDataSource = async (id: string): Promise<Result<DataSource, ApiError>> => {
  const result = await listDataSources();
  if (!isOk(result)) return result;
  const source = result.value.find((candidate) => candidate.id === id);
  return source
    ? ok(source)
    : err({ code: "NOT_FOUND", message: `Data source ${id} was not found.`, status: 404 });
};

/** Create a data source. */
export const createDataSource = (
  source: CreateDataSourceRequest
): Promise<Result<DataSource, ApiError>> => post<DataSource>("/data/sources", source);

/** Update a data source. */
export const updateDataSource = (
  id: string,
  source: UpdateDataSourceRequest
): Promise<Result<DataSource, ApiError>> => put<DataSource>(`/data/sources/${id}`, source);

/** Delete a data source. */
export const deleteDataSource = (id: string): Promise<Result<void, ApiError>> =>
  del<void>(`/data/sources/${id}`);

/** Test a data source connection. */
export const testConnection = (id: string): Promise<Result<ConnectionTestResult, ApiError>> =>
  post<ConnectionTestResult>(`/data/sources/${id}/test`, {});

// =============================================================================
// Discovery
// =============================================================================

/** Build a query string from optional params, omitting undefined values. */
function qs(params: Record<string, string | number | undefined>): string {
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== null && v !== "") p.set(k, String(v));
  }
  const s = p.toString();
  return s ? `?${s}` : "";
}

function activeWorkspaceKey(): string {
  const header = getApiConfig().headers["X-Workspace-Id"];
  return header || getFlowAIStudioConfig().defaultWorkspaceKey;
}

function workspaceDataPath(...segments: readonly string[]): string {
  const encoded = [activeWorkspaceKey(), "data", ...segments].map((segment) =>
    encodeURIComponent(segment)
  );
  return `/workspaces/${encoded.join("/")}`;
}

/** List schemas. */
export const listSchemas = (sourceId?: string): Promise<Result<string[], ApiError>> =>
  get<{ schemas: string[] }>(workspaceDataPath("discovery", "schemas") + qs({ sourceId })).then(
    (result) => (isOk(result) ? ok(result.value.schemas) : result)
  );

/** List tables in a schema. */
export const listTables = (
  schema?: string,
  sourceId?: string
): Promise<Result<TableInfo[], ApiError>> =>
  get<{ tables: TableInfo[] }>(
    workspaceDataPath("discovery", "tables") + qs({ schema, sourceId })
  ).then((result) => (isOk(result) ? ok(result.value.tables) : result));

/** Get table detail (columns, constraints, indexes). */
export const getTableDetail = (
  tableName: string,
  schema?: string,
  sourceId?: string
): Promise<Result<PhysicalTable, ApiError>> =>
  get<{ table: PhysicalTable }>(
    workspaceDataPath("discovery", "tables", tableName) + qs({ schema, sourceId })
  ).then((result) => (isOk(result) ? ok(result.value.table) : result));

/** Get table columns. */
export const getTableColumns = (
  tableName: string,
  schema?: string,
  sourceId?: string
): Promise<Result<ColumnInfo[], ApiError>> =>
  get<{ columns: ColumnInfo[] }>(
    workspaceDataPath("discovery", "tables", tableName, "columns") + qs({ schema, sourceId })
  ).then((result) => (isOk(result) ? ok(result.value.columns) : result));

/** Sample table rows. */
export const sampleTable = (
  tableName: string,
  limit?: number,
  sourceId?: string
): Promise<Result<unknown[], ApiError>> =>
  get<{ rows: unknown[] }>(
    workspaceDataPath("discovery", "tables", tableName, "sample") + qs({ limit, sourceId })
  ).then((result) => (isOk(result) ? ok(result.value.rows) : result));

// =============================================================================
// Profiling (SSE Streams)
// =============================================================================

export type IngestionStreamHandlers = JsonSSEHandlers<IngestionEvent>;

/** IngestionEvent is terminal when type is "completed" or "error". */
const isIngestionTerminal = (event: IngestionEvent) => {
  if (event.type === "completed") return {};
  if (event.type === "error") return { error: event.message };
  return null;
};

/** Start profiling a single table (SSE stream). */
export const startProfileTable = (
  request: { sourceId: string; tableName: string; schemaName?: string; modelId?: string },
  handlers: IngestionStreamHandlers
): Promise<Result<{ abort: () => void }, ApiError>> =>
  startJsonSSEStream(
    "POST",
    workspaceDataPath("profile", "table"),
    request,
    handlers,
    isIngestionTerminal
  );

/** Start profiling an entire database (SSE stream). */
export const startProfileDatabase = (
  request: { sourceId: string; schemaName?: string; modelId?: string },
  handlers: IngestionStreamHandlers
): Promise<Result<{ abort: () => void }, ApiError>> =>
  startJsonSSEStream(
    "POST",
    workspaceDataPath("profile", "database"),
    request,
    handlers,
    isIngestionTerminal
  );

/** Get profiling job status. */
export const getProfilingStatus = (jobId: string): Promise<Result<IngestionStatus, ApiError>> =>
  get<IngestionStatus>(`/data/profiling/${jobId}`);

/** Cancel a profiling job. */
export const cancelProfiling = (_jobId: string): Promise<Result<void, ApiError>> =>
  Promise.resolve(ok(undefined));

/** Estimate profiling cost before starting. */
export const estimateProfilingCost = (request: {
  tableCount: number;
  totalColumns: number;
  modelId?: string;
}): Promise<Result<ProfilingCostEstimate, ApiError>> =>
  post<{
    estimate: ProfilingCostEstimate & {
      modelId?: string | null;
      estimatedCostUsd?: number | null;
    };
  }>(workspaceDataPath("profile", "estimate"), request).then((result) => {
    if (!isOk(result)) return result;
    const estimate = result.value.estimate;
    const modelId = estimate.modelId ?? request.modelId ?? "unpriced";
    return ok({
      estimatedInputTokens: estimate.estimatedInputTokens,
      estimatedOutputTokens: estimate.estimatedOutputTokens,
      estimatedCachedTokens: estimate.estimatedCachedTokens,
      estimatedCostUsd: estimate.estimatedCostUsd ?? 0,
      modelId,
      modelDisplayName: modelId,
      inputPerMTok: estimate.inputPerMTok ?? 0,
      outputPerMTok: estimate.outputPerMTok ?? 0,
      cacheReadPerMTok: estimate.cacheReadPerMTok,
    });
  });

// =============================================================================
// Knowledge
// =============================================================================

/** List documents. */
export const listDocuments = (): Promise<Result<DocumentItem[], ApiError>> =>
  get<{ documents: DocumentItem[] }>(workspaceDataPath("knowledge", "documents")).then((result) =>
    isOk(result) ? ok(result.value.documents) : result
  );

/** Ingest documents. */
export const ingestDocuments = (
  documents: IngestDocumentEntry[]
): Promise<Result<DocumentItem[], ApiError>> =>
  post<DocumentItem[]>("/data/knowledge/documents", { documents });

/** Get a document. */
export const getDocument = (docId: string): Promise<Result<DocumentItem, ApiError>> =>
  get<DocumentItem>(`/data/knowledge/documents/${docId}`);

/** Delete a document. */
export const deleteDocument = (docId: string): Promise<Result<void, ApiError>> =>
  del<void>(`/data/knowledge/documents/${docId}`);

/** Extract knowledge from a document (SSE stream). */
export const extractKnowledge = (
  docId: string,
  handlers: IngestionStreamHandlers
): Promise<Result<{ abort: () => void }, ApiError>> =>
  startJsonSSEStream(
    "POST",
    `/data/knowledge/documents/${docId}/extract`,
    undefined,
    handlers,
    isIngestionTerminal
  );

/** Browse knowledge items. */
export const browseKnowledge = (sourceId?: string): Promise<Result<KnowledgeItem[], ApiError>> =>
  get<{ items: KnowledgeItem[] }>(workspaceDataPath("knowledge", "items") + qs({ sourceId })).then(
    (result) => (isOk(result) ? ok(result.value.items) : result)
  );

/** Add a knowledge item. */
export const addKnowledge = (item: AddKnowledgeRequest): Promise<Result<KnowledgeItem, ApiError>> =>
  post<KnowledgeItem>("/data/knowledge/items", item);

/** Get a knowledge item. */
export const getKnowledgeItem = (itemId: string): Promise<Result<KnowledgeItem, ApiError>> =>
  get<KnowledgeItem>(`/data/knowledge/items/${itemId}`);

/** Update a knowledge item. */
export const updateKnowledgeItem = (
  itemId: string,
  item: Partial<AddKnowledgeRequest>
): Promise<Result<KnowledgeItem, ApiError>> =>
  put<KnowledgeItem>(`/data/knowledge/items/${itemId}`, item);

/** Delete a knowledge item. */
export const deleteKnowledgeItem = (itemId: string): Promise<Result<void, ApiError>> =>
  del<void>(`/data/knowledge/items/${itemId}`);

// -- Knowledge Ingestion (directory / S3) --

export type KnowledgeIngestHandlers = JsonSSEHandlers<KnowledgeIngestEvent>;

/** KnowledgeIngestEvent is terminal when type is "completed" or "error". */
const isKnowledgeIngestTerminal = (event: KnowledgeIngestEvent) => {
  if (event.type === "completed") return {};
  if (event.type === "error") return { error: event.message };
  return null;
};

/** Ingest knowledge from a local directory or S3 bucket (SSE stream). */
export const ingestKnowledgeFromSource = (
  source: KnowledgeSourceSpec,
  handlers: KnowledgeIngestHandlers,
  extractKnowledgeFlag = false
): Promise<Result<{ abort: () => void }, ApiError>> =>
  startJsonSSEStream(
    "POST",
    workspaceDataPath("knowledge", "ingest"),
    { source, extractKnowledge: extractKnowledgeFlag },
    handlers,
    isKnowledgeIngestTerminal
  );

// =============================================================================
// Search
// =============================================================================

interface StudioSearchResult {
  readonly id: string;
  readonly category: string;
  readonly name: string;
  readonly qualifiedName?: string | null;
  readonly description: string;
  readonly score: number;
}

interface StudioSearchResponse {
  readonly results: readonly StudioSearchResult[];
  readonly query: string;
  readonly totalCount: number;
}

/** Unified search across all types. */
export const unifiedSearch = async (
  query: string,
  limit?: number,
  mode?: "keyword" | "vector" | "hybrid",
  sourceId?: string
): Promise<Result<DataSearchResult, ApiError>> => {
  const result = await get<StudioSearchResponse>(
    `/data/search?query=${encodeURIComponent(query)}${limit ? `&limit=${limit}` : ""}${mode ? `&mode=${mode}` : ""}${sourceId ? `&sourceId=${encodeURIComponent(sourceId)}` : ""}`
  );
  if (!isOk(result)) return result;

  const items: CatalogSearchResult[] = result.value.results.map((entry) => ({
    id: entry.id,
    name: entry.name,
    itemType: entry.category,
    description: entry.description,
    score: entry.score,
    tags: entry.qualifiedName ? [entry.qualifiedName] : [],
  }));

  return ok({
    items,
    totalCount: result.value.totalCount,
    queryTimeMs: 0,
  });
};

/** Resolve a term across types. */
export const resolveTerm = (
  term: string,
  limit?: number
): Promise<Result<TermResolution, ApiError>> =>
  post<TermResolution>("/data/resolve/term", { term, limit });

/** Find join path between tables. */
export const findJoinPath = (
  fromTable: string,
  toTable: string
): Promise<Result<JoinPath | null, ApiError>> =>
  post<JoinPath | null>("/data/resolve/join-path", { fromTable, toTable });

// =============================================================================
// Tools
// =============================================================================

/** List all available tools. */
export const listTools = (): Promise<Result<ToolInfo[], ApiError>> =>
  get<ToolInfo[]>("/data/tools");

/** Execute a tool by ID. */
export const executeTool = (
  toolId: string,
  input: Record<string, unknown>,
  sourceId?: string
): Promise<Result<ToolResult, ApiError>> =>
  post<ToolResult>(`/data/tools/${toolId}${qs({ sourceId })}`, input);

// =============================================================================
// Metrics
// =============================================================================

/** List all metrics. */
export const listMetrics = (): Promise<Result<MetricItem[], ApiError>> =>
  get<MetricItem[]>("/data/metrics");

/** Create a metric. */
export const createMetric = (metric: CreateMetricRequest): Promise<Result<MetricItem, ApiError>> =>
  post<MetricItem>("/data/metrics", metric);

/** Get a metric. */
export const getMetric = (metricId: string): Promise<Result<MetricItem, ApiError>> =>
  get<MetricItem>(`/data/metrics/${metricId}`);

/** Update a metric. */
export const updateMetric = (
  metricId: string,
  metric: Partial<CreateMetricRequest>
): Promise<Result<MetricItem, ApiError>> => put<MetricItem>(`/data/metrics/${metricId}`, metric);

/** Delete a metric. */
export const deleteMetric = (metricId: string): Promise<Result<void, ApiError>> =>
  del<void>(`/data/metrics/${metricId}`);

// =============================================================================
// Import Pipeline
// =============================================================================

export type ImportStreamHandlers = JsonSSEHandlers<ImportEvent>;

/** ImportEvent is terminal when type is "completed" or "error". */
const isImportTerminal = (event: ImportEvent) => {
  if (event.type === "completed") return {};
  if (event.type === "error") return { error: event.message };
  // Backend sends "resync" when broadcast subscriber falls behind.
  if ((event as { type: string }).type === "resync")
    return { error: "Stream lagged, reconnecting..." };
  return null;
};

/** Start data import pipeline (multipart upload + SSE stream). */
export async function startImport(
  sourceId: string,
  files: File[],
  handlers: ImportStreamHandlers,
  modelId?: string
): Promise<Result<{ abort: () => void }, ApiError>> {
  const apiConfig = getApiConfig();
  const url = `${apiConfig.baseUrl}/data/import`;
  const controller = new AbortController();

  const formData = new FormData();
  formData.append("sourceId", sourceId);
  if (modelId) formData.append("modelId", modelId);
  for (const file of files) {
    formData.append("files", file);
  }

  try {
    const response = await fetch(url, {
      method: "POST",
      headers: {
        // Do NOT set Content-Type — browser sets it with boundary for multipart
        ...Object.fromEntries(
          Object.entries(apiConfig.headers).filter(([k]) => k.toLowerCase() !== "content-type")
        ),
        Accept: "text/event-stream",
      },
      body: formData,
      signal: controller.signal,
    });

    if (!response.ok) {
      const errorText = await response.text().catch(() => "");
      return err({
        code: response.status === 404 ? "NOT_FOUND" : "SERVER_ERROR",
        message: errorText || response.statusText,
        status: response.status,
      });
    }

    const reader = response.body?.getReader();
    if (!reader) {
      return err({ code: "SERVER_ERROR", message: "No response body" });
    }

    readJsonSSEStream(reader, handlers, isImportTerminal, controller.signal);

    return ok({ abort: () => controller.abort() });
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") {
      return ok({ abort: () => {} });
    }
    return err({
      code: "NETWORK_ERROR",
      message: error instanceof Error ? error.message : "Failed to connect",
    });
  }
}

/** Reconnect to an import stream (replays snapshot + live events). */
export const connectImportStream = (
  jobId: string,
  handlers: ImportStreamHandlers
): Promise<Result<{ abort: () => void }, ApiError>> =>
  startJsonSSEStream("GET", `/data/import/${jobId}/stream`, undefined, handlers, isImportTerminal);

/** Get import job status. */
export const getImportStatus = async (jobId: string): Promise<Result<ImportStage, ApiError>> => {
  const result = await get<ImportStage>(`/data/import/${jobId}`);
  if (!isOk(result)) return result;
  return validateBoundary(ImportStageSchema, result.value, "getImportStatus");
};

/** Cancel an import job. */
export const cancelImport = (jobId: string): Promise<Result<void, ApiError>> =>
  post<void>(`/data/import/${jobId}/cancel`, {});

/** Get current workspace data readiness. */
export const getDataReadiness = async (): Promise<Result<DataReadiness, ApiError>> => {
  const result = await get<DataReadiness>("/data/readiness");
  if (!isOk(result)) return result;
  return validateBoundary(DataReadinessSchema, result.value, "getDataReadiness");
};

// =============================================================================
// Admin (Database Management)
// =============================================================================

/** Get status of all databases. */
export const getAdminStatus = (): Promise<Result<DatabaseStatusSummary, ApiError>> =>
  get<DatabaseStatusSummary>("/data/admin/status");

/** Run migrations on specified databases. */
export const runMigrations = (databases: string[]): Promise<Result<MigrationResult[], ApiError>> =>
  post<MigrationResult[]>("/data/admin/migrate", { databases });

/** Purge and re-migrate a database (requires confirmation). */
export const purgeDatabase = (role: string): Promise<Result<PurgeResult, ApiError>> =>
  post<PurgeResult>(`/data/admin/purge/${encodeURIComponent(role)}`, { confirm: true });
