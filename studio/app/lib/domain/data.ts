/**
 * Data domain types — TypeScript mirror of Rust `core::data`.
 *
 * All types are readonly interfaces with discriminated unions.
 * Algebraic sum types with exhaustive matching.
 *
 * @module domain/data
 */

// =============================================================================
// Literal Unions
// =============================================================================

/** Database type. */
export type DatabaseType = "postgresql" | "mysql" | "sqlite";

/** Semantic type for column profiling. */
export type SemanticType =
  | "numeric"
  | "categorical"
  | "text"
  | "temporal"
  | "identifier"
  | "json"
  | "unknown";

/** Table type. */
export type TableType = "base_table" | "view" | "materialized_view" | "foreign";

/** Knowledge type (8 variants). */
export type KnowledgeType =
  | "business_rule"
  | "predicate"
  | "terminology"
  | "constraint"
  | "temporal_rule"
  | "implicit_intent"
  | "data_quality"
  | "custom";

/** Document extraction status. */
export type ExtractionStatus = "pending" | "processing" | "processed" | "failed";

/** Data tab section navigation. */
export type DataSection =
  | "sources"
  | "discovery"
  | "profiling"
  | "import"
  | "knowledge"
  | "search"
  | "tools";

/** Ingestion status discriminant (8 variants). */
export type IngestionStatusKey =
  | "queued"
  | "discovering"
  | "profiling"
  | "enriching"
  | "extracting"
  | "indexing"
  | "completed"
  | "failed";

// =============================================================================
// Data Source
// =============================================================================

export interface DataSource {
  readonly id: string;
  readonly name: string;
  readonly kind?: string;
  readonly status?: string;
  readonly databaseType: DatabaseType;
  readonly host: string;
  readonly port: number;
  readonly databaseName: string;
  readonly schemaName: string;
  readonly encryptedCredentials: string | null;
  readonly isActive: boolean;
  readonly createdAt: string;
  readonly updatedAt: string;
  readonly metadata?: Record<string, unknown>;
}

export interface ConnectionTestResult {
  readonly success: boolean;
  readonly latencyMs: number;
  readonly error?: string;
  readonly serverVersion?: string;
}

export type DataSourceStatus =
  | { readonly status: "connected" }
  | { readonly status: "disconnected" }
  | { readonly status: "error"; readonly message: string };

// =============================================================================
// Discovery Types
// =============================================================================

export interface TableInfo {
  readonly schemaName: string;
  readonly tableName: string;
  readonly tableType: TableType;
  readonly rowCount: number | null;
  readonly columnCount?: number | null;
  readonly description: string | null;
}

export interface ColumnInfo {
  readonly columnName: string;
  readonly dataType: string;
  readonly isNullable: boolean;
  readonly columnDefault: string | null;
  readonly ordinalPosition: number;
  readonly isPrimaryKey: boolean;
  readonly foreignKey: ForeignKeyRef | null;
}

export interface ForeignKeyRef {
  readonly referencedSchema: string;
  readonly referencedTable: string;
  readonly referencedColumn: string;
  readonly constraintName: string;
}

export interface ConstraintInfo {
  readonly name: string;
  readonly constraintType: string;
  readonly columns: readonly string[];
}

export interface IndexInfo {
  readonly name: string;
  readonly columns: readonly string[];
  readonly isUnique: boolean;
}

export interface PhysicalTable {
  readonly schemaName: string;
  readonly tableName: string;
  readonly columns: readonly ColumnInfo[];
  readonly constraints: readonly ConstraintInfo[];
  readonly indexes: readonly IndexInfo[];
  readonly rowCount: number | null;
}

// =============================================================================
// Profiling Types
// =============================================================================

export interface CategoryValue {
  readonly value: string;
  readonly count: number;
  readonly percentage: number;
}

export type TypeSpecificStats =
  | {
      readonly type: "numeric";
      readonly min: number | null;
      readonly max: number | null;
      readonly mean: number | null;
      readonly p25: number | null;
      readonly p50: number | null;
      readonly p75: number | null;
    }
  | {
      readonly type: "categorical";
      readonly topValues: readonly CategoryValue[];
    }
  | { readonly type: "text"; readonly maxLength: number | null }
  | {
      readonly type: "temporal";
      readonly minTime: string | null;
      readonly maxTime: string | null;
    }
  | {
      readonly type: "json";
      readonly typeDistribution: Record<string, number>;
      readonly topKeys: readonly string[];
    }
  | { readonly type: "none" };

export interface ColumnProfile {
  readonly columnName: string;
  readonly dataType: string;
  readonly nullCount: number;
  readonly distinctCount: number;
  readonly totalCount: number;
  readonly semanticType: SemanticType;
  readonly stats: TypeSpecificStats;
}

export interface TableProfile {
  readonly tableName: string;
  readonly columns: readonly ColumnProfile[];
}

// =============================================================================
// Enrichment Types
// =============================================================================

export interface SemanticTableProfile {
  readonly description: string;
  readonly shortDescription: string;
  readonly columnDescriptions: Record<string, string>;
  readonly relationships: readonly InferredRelationship[];
  readonly qualityNotes: readonly QualityNote[];
}

export interface InferredRelationship {
  readonly sourceTable: string;
  readonly targetTable: string;
  readonly relationshipType: string;
  readonly joinColumns: readonly [string, string][];
  readonly description: string;
}

export interface QualityNote {
  readonly columnName: string;
  readonly notes: string;
  readonly typicalValueRange: string | null;
  readonly validationRules: readonly string[];
}

// =============================================================================
// Ingestion State Machine (Discriminated Union)
// =============================================================================

export type IngestionStatus =
  | { readonly status: "queued" }
  | { readonly status: "discovering"; readonly tablesFound: number }
  | {
      readonly status: "profiling";
      readonly tablesFound: number;
      readonly columnsProfiled: number;
      readonly totalColumns: number;
    }
  | {
      readonly status: "enriching";
      readonly tablesEnriched: number;
      readonly totalTables: number;
    }
  | { readonly status: "extracting"; readonly enumsExtracted: number }
  | { readonly status: "indexing"; readonly itemsIndexed: number }
  | { readonly status: "completed"; readonly summary: IngestionSummary }
  | {
      readonly status: "failed";
      readonly error: string;
      readonly partial: IngestionSummary | null;
    };

export interface IngestionSummary {
  readonly tablesDiscovered: number;
  readonly columnsProfiled: number;
  readonly enumsExtracted: number;
  readonly relationshipsFound: number;
  readonly catalogItemsIndexed: number;
  readonly durationMs: number;
  /** True when any LLM enrichment fell back to physical schema (join-semilattice). */
  readonly enrichmentDegraded?: boolean;
  /** Number of tables whose enrichment was served from cache (additive monoid). */
  readonly enrichmentCacheHits?: number;
  /** Number of tables where enrichment fell back to schema-only descriptions (additive monoid). */
  readonly enrichmentFallbacks?: number;
  /** Number of tables with fresh LLM enrichment (additive monoid). */
  readonly enrichmentFresh?: number;
}

// =============================================================================
// Ingestion Events (Discriminated Union — SSE)
// =============================================================================

/** How a table's semantic enrichment was obtained. */
export type EnrichmentSource = "fresh" | "cached" | "fallback";

export type IngestionEvent =
  | { readonly type: "started"; readonly jobId: string }
  | { readonly type: "progress"; readonly status: IngestionStatus }
  | {
      readonly type: "tableProfiled";
      readonly tableName: string;
      readonly columns: number;
      readonly durationMs: number;
    }
  | {
      readonly type: "tableEnriched";
      readonly tableName: string;
      readonly source: EnrichmentSource;
    }
  | {
      readonly type: "tableCompleted";
      readonly tableName: string;
      readonly summary: IngestionSummary;
    }
  | {
      readonly type: "tableFailed";
      readonly tableName: string;
      readonly error: string;
    }
  | { readonly type: "completed"; readonly summary: IngestionSummary }
  | { readonly type: "error"; readonly message: string };

// =============================================================================
// Profiling Cost Estimate
// =============================================================================

/** Cost estimate for a profiling run. */
export interface ProfilingCostEstimate {
  readonly estimatedInputTokens: number;
  readonly estimatedOutputTokens: number;
  readonly estimatedCachedTokens: number;
  readonly estimatedCostUsd: number;
  readonly modelId: string;
  readonly modelDisplayName: string;
  readonly inputPerMTok: number;
  readonly outputPerMTok: number;
  readonly cacheReadPerMTok?: number;
}

// =============================================================================
// Knowledge Types
// =============================================================================

export interface DocumentItem {
  readonly id: string;
  readonly name: string;
  readonly content: string;
  readonly targetDatabaseId: string;
  readonly extractionStatus: ExtractionStatus;
  readonly extractedKnowledgeIds: readonly string[];
  readonly createdAt: string;
}

export interface KnowledgeItem {
  readonly id: string;
  readonly name: string;
  readonly description: string;
  readonly knowledgeType: KnowledgeType;
  readonly scopeTables: readonly string[];
  readonly scopeColumns: readonly string[];
  readonly sqlExpression: string | null;
  readonly synonyms: readonly string[];
  readonly sourceDocumentId: string | null;
}

export interface MetricItem {
  readonly id: string;
  readonly name: string;
  readonly displayName: string;
  readonly formula: string;
  readonly formulaDescription: string;
  readonly sourceTables: readonly string[];
  readonly sourceColumns: readonly string[];
  readonly aggregationType: string;
  readonly timeGrain: string | null;
  readonly outputType: string;
}

// =============================================================================
// Search Types
// =============================================================================

export interface DataSearchResult {
  readonly items: readonly CatalogSearchResult[];
  readonly totalCount: number;
  readonly queryTimeMs: number;
}

export interface CatalogSearchResult {
  readonly id: string;
  readonly name: string;
  readonly itemType: string;
  readonly description: string | null;
  readonly score: number;
  readonly tags: readonly string[];
}

export interface TermResolution {
  readonly tables: readonly CatalogSearchResult[];
  readonly columns: readonly CatalogSearchResult[];
  readonly enums: readonly CatalogSearchResult[];
  readonly metrics: readonly CatalogSearchResult[];
}

export interface JoinPath {
  readonly fromTable: string;
  readonly toTable: string;
  readonly steps: readonly JoinStep[];
}

export interface JoinStep {
  readonly fromTable: string;
  readonly toTable: string;
  readonly fromColumn: string;
  readonly toColumn: string;
  readonly joinType: string;
}

// =============================================================================
// Knowledge Ingestion (Directory / S3)
// =============================================================================

/** Source specification for knowledge ingestion — closed sum type. */
export type KnowledgeSourceSpec =
  | { readonly type: "localDirectory"; readonly path: string }
  | {
      readonly type: "s3Bucket";
      readonly bucket: string;
      readonly prefix?: string;
      readonly region?: string;
    };

/** SSE events during knowledge ingestion from a directory or S3 bucket. */
export type KnowledgeIngestEvent =
  | { readonly type: "discovered"; readonly totalFiles: number }
  | {
      readonly type: "ingesting";
      readonly current: number;
      readonly total: number;
      readonly fileName: string;
    }
  | {
      readonly type: "extracting";
      readonly current: number;
      readonly total: number;
      readonly fileName: string;
    }
  | {
      readonly type: "completed";
      readonly documentsIngested: number;
      readonly documentsSkipped: number;
      readonly knowledgeItemsExtracted: number;
    }
  | { readonly type: "error"; readonly message: string };

// =============================================================================
// Request Types
// =============================================================================

export interface CreateDataSourceRequest {
  readonly name: string;
  readonly databaseType: DatabaseType;
  readonly host: string;
  readonly port: number;
  readonly databaseName: string;
  readonly schemaName?: string;
  readonly username?: string;
  readonly password?: string;
}

export interface UpdateDataSourceRequest {
  readonly name?: string;
  readonly host?: string;
  readonly port?: number;
  readonly databaseName?: string;
  readonly schemaName?: string;
  readonly username?: string;
  readonly password?: string;
  readonly isActive?: boolean;
}

export interface ProfileTableRequest {
  readonly sourceId: string;
  readonly tableName: string;
  readonly schemaName?: string;
  readonly modelId?: string;
  readonly sampleSize?: number;
}

export interface ProfileDatabaseRequest {
  readonly sourceId: string;
  readonly schemaName?: string;
  readonly tables?: readonly string[];
  readonly modelId?: string;
  readonly sampleSize?: number;
}

export interface AddKnowledgeRequest {
  readonly name: string;
  readonly description: string;
  readonly knowledgeType: KnowledgeType;
  readonly scopeTables: readonly string[];
  readonly scopeColumns: readonly string[];
  readonly sqlExpression?: string;
  readonly synonyms: readonly string[];
}

export interface IngestDocumentEntry {
  readonly name: string;
  readonly content: string;
  readonly targetDatabaseId: string;
}

// =============================================================================
// Type Guards
// =============================================================================

export const isIngestionStarted = (
  e: IngestionEvent
): e is Extract<IngestionEvent, { type: "started" }> => e.type === "started";

export const isIngestionProgress = (
  e: IngestionEvent
): e is Extract<IngestionEvent, { type: "progress" }> => e.type === "progress";

export const isIngestionTableProfiled = (
  e: IngestionEvent
): e is Extract<IngestionEvent, { type: "tableProfiled" }> => e.type === "tableProfiled";

export const isIngestionCompleted = (
  e: IngestionEvent
): e is Extract<IngestionEvent, { type: "completed" }> => e.type === "completed";

export const isIngestionError = (
  e: IngestionEvent
): e is Extract<IngestionEvent, { type: "error" }> => e.type === "error";

// =============================================================================
// Pipeline Stage Types (Per-Table Tracking)
// =============================================================================

export type PipelineStageKey =
  | "discovering"
  | "profiling"
  | "enriching"
  | "extracting"
  | "indexing";
export type TableStageStatus = "queued" | "active" | "completed" | "failed";

export interface TablePipelineState {
  readonly tableName: string;
  readonly stages: Record<PipelineStageKey, TableStageStatus>;
  readonly columns: number;
  readonly durationMs: number;
  /** How enrichment was obtained — populated by tableEnriched SSE event. */
  readonly enrichmentSource?: EnrichmentSource;
}

export const PIPELINE_STAGES: readonly {
  key: PipelineStageKey;
  label: string;
}[] = [
  { key: "discovering", label: "Discover" },
  { key: "profiling", label: "Profile" },
  { key: "enriching", label: "Enrich" },
  { key: "extracting", label: "Extract" },
  { key: "indexing", label: "Index" },
];

// =============================================================================
// Status Color Maps
// =============================================================================

export const INGESTION_STATUS_COLORS: Record<IngestionStatusKey, string> = {
  queued: "var(--muted-foreground)",
  discovering: "var(--primary)",
  profiling: "var(--primary)",
  enriching: "var(--primary)",
  extracting: "var(--primary)",
  indexing: "var(--primary)",
  completed: "var(--dot-emerald)",
  failed: "var(--dot-red)",
};

export const EXTRACTION_STATUS_COLORS: Record<ExtractionStatus, string> = {
  pending: "var(--muted-foreground)",
  processing: "var(--primary)",
  processed: "var(--dot-emerald)",
  failed: "var(--dot-red)",
};

export const EXTRACTION_STATUS_LABELS: Record<ExtractionStatus, string> = {
  pending: "Pending",
  processing: "Processing",
  processed: "Processed",
  failed: "Failed",
};

// =============================================================================
// Tool Types
// =============================================================================

export interface ToolInfo {
  readonly id: string;
  readonly name: string;
  readonly description: string;
  readonly parameters: Record<string, unknown>;
}

export interface ToolResult {
  readonly success: boolean;
  readonly data: string;
  readonly count: number | null;
  readonly ids: readonly string[];
  readonly error: string | null;
}

// =============================================================================
// Metric Request Types
// =============================================================================

export interface CreateMetricRequest {
  readonly name: string;
  readonly displayName: string;
  readonly formula: string;
  readonly formulaDescription: string;
  readonly sourceTables: readonly string[];
  readonly sourceColumns: readonly string[];
  readonly aggregationType: string;
  readonly timeGrain?: string;
  readonly outputType: string;
}

// =============================================================================
// Import Pipeline Types
// =============================================================================

/** Import pipeline stage discriminant (6 stages, no dim/fact split). */
export type ImportStageKey =
  | "uploading"
  | "parsing"
  | "schema"
  | "loading"
  | "validation"
  | "profiling";

/**
 * Import stage state machine (discriminated union, 8 variants).
 *
 * Merges the upstream loadingDimensions + loadingFacts stages into a single
 * loadingTables stage carrying both table-level and row-level progress.
 */
export type ImportStage =
  | { readonly stage: "uploading" }
  | { readonly stage: "parsing" }
  | { readonly stage: "creatingSchema" }
  | {
      readonly stage: "loadingTables";
      readonly tablesCompleted: number;
      readonly totalTables: number;
      readonly currentBatchRows?: number;
      readonly totalBatchRows?: number;
    }
  | { readonly stage: "validating" }
  | { readonly stage: "profiling" }
  | { readonly stage: "completed"; readonly summary: ImportSummary }
  | { readonly stage: "failed"; readonly error: string };

/**
 * Summary of a completed import.
 *
 * tableRowCounts is Record<string, number> — the free monoid over (table, count).
 * Total = Object.values(tableRowCounts).reduce((a, b) => a + b, 0).
 */
export interface ImportSummary {
  readonly sourceRowCount: number;
  readonly tableRowCounts: Record<string, number>;
  readonly documentsIngested?: number;
  readonly knowledgeItemsExtracted?: number;
  readonly durationMs: number;
}

/** Workspace-scoped ingested-data readiness emitted by the Python runtime. */
export interface DataReadiness {
  readonly workspaceId: string;
  readonly ready: boolean;
  readonly status: "empty" | "ready";
  readonly sourceId?: string | null;
  readonly importJobId?: string | null;
  readonly profileJobId?: string | null;
  readonly dataBundle?: {
    readonly status?: "complete" | "degraded" | "empty";
    readonly complete?: boolean;
    readonly configuredRoles?: readonly string[];
    readonly missingRoles?: readonly string[];
  };
  readonly tableRowCounts: Record<string, number>;
  readonly targetTables: readonly { readonly name: string; readonly rowCount: number }[];
  readonly documents: { readonly ingested: number };
  readonly knowledge: {
    readonly itemsExtracted: number;
    readonly itemIds: readonly string[];
  };
  readonly catalogProfile: {
    readonly summary: Readonly<Record<string, unknown>>;
    readonly profiledTables: readonly string[];
  };
  readonly generatedAt: string;
}

/** Validation check result. */
export interface ValidationCheck {
  readonly name: string;
  readonly passed: boolean;
  readonly message: string;
}

/**
 * SSE events emitted during import pipeline (discriminated union, 9 variants).
 *
 * tableLoaded replaces dimensionLoaded (a table finished loading).
 * batchProgress replaces factBatchLoaded (row-level progress for large tables).
 * profilingEvent nests the existing IngestionEvent for the profiling sub-step.
 */
export type ImportEvent =
  | { readonly type: "started"; readonly jobId: string }
  | { readonly type: "stageProgress"; readonly stage: ImportStage }
  | {
      readonly type: "tableLoaded";
      readonly tableName: string;
      readonly rowCount: number;
    }
  | {
      readonly type: "batchProgress";
      readonly tableName: string;
      readonly rowsLoaded: number;
      readonly totalRows: number;
    }
  | { readonly type: "schemaCreated"; readonly tables: readonly string[] }
  | { readonly type: "knowledgeExtractionStarted"; readonly documents: number }
  | {
      readonly type: "documentExtracted";
      readonly documentId: string;
      readonly documentName: string;
      readonly knowledgeItemsExtracted: number;
    }
  | {
      readonly type: "validationPassed";
      readonly checks: readonly ValidationCheck[];
    }
  | { readonly type: "profilingEvent"; readonly inner: IngestionEvent }
  | { readonly type: "completed"; readonly summary: ImportSummary }
  | { readonly type: "error"; readonly message: string };

/** Import pipeline stages for progress bar display. */
export const IMPORT_STAGES: readonly { key: ImportStageKey; label: string }[] = [
  { key: "uploading", label: "Upload" },
  { key: "parsing", label: "Parse" },
  { key: "schema", label: "Schema" },
  { key: "loading", label: "Loading" },
  { key: "validation", label: "Validate" },
  { key: "profiling", label: "Profile" },
];

/** Status colors for import stages. */
export const IMPORT_STATUS_COLORS: Record<"pending" | "active" | "completed" | "failed", string> = {
  pending: "var(--muted-foreground)",
  active: "var(--primary)",
  completed: "var(--dot-emerald)",
  failed: "var(--dot-red)",
};

/** Type guards for import events. */
export const isImportStarted = (e: ImportEvent): e is Extract<ImportEvent, { type: "started" }> =>
  e.type === "started";
export const isImportCompleted = (
  e: ImportEvent
): e is Extract<ImportEvent, { type: "completed" }> => e.type === "completed";
export const isImportError = (e: ImportEvent): e is Extract<ImportEvent, { type: "error" }> =>
  e.type === "error";

// =============================================================================
// Database Admin Types
// =============================================================================

/**
 * Status of a single database.
 *
 * role is string (not closed union) — the backend determines
 * available roles at runtime. label comes from the backend too.
 */
export interface DatabaseInfo {
  readonly role: string;
  readonly label: string;
  readonly configured: boolean;
  readonly isPurgeable: boolean;
  readonly host: string | null;
  readonly databaseName: string | null;
}

/** Status summary for all databases. */
export interface DatabaseStatusSummary {
  readonly databases: readonly DatabaseInfo[];
}

/** Result of running migrations on a single database. */
export interface MigrationResult {
  readonly role: string;
  readonly success: boolean;
  readonly migrationsRun: readonly string[];
  readonly error: string | null;
}

/** Result of purging a database. */
export interface PurgeResult {
  readonly role: string;
  readonly success: boolean;
  readonly error: string | null;
}
