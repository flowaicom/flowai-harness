/**
 * Zod schemas for runtime validation of high-risk Studio API responses.
 *
 * These schemas validate the **critical** fields of the five highest-risk
 * response types at the API boundary. Each schema uses `.passthrough()` so
 * that new backend fields are preserved without breaking existing clients.
 *
 * Schemas match the camelCase frontend types (after endpoint-specific normalization).
 *
 * @module api/schemas
 */

import { z } from "zod";

const prototypePollutionKeys = new Set(["__proto__", "constructor", "prototype"]);

function sanitizeApiBoundaryValue(
  value: unknown,
  seen: WeakMap<object, unknown> = new WeakMap()
): unknown {
  if (value === null || typeof value !== "object") {
    return value;
  }

  if (seen.has(value)) {
    return seen.get(value);
  }

  if (Array.isArray(value)) {
    const sanitizedArray: unknown[] = [];
    seen.set(value, sanitizedArray);

    let changed = false;
    value.forEach((item, index) => {
      const sanitizedItem = sanitizeApiBoundaryValue(item, seen);
      sanitizedArray[index] = sanitizedItem;
      if (sanitizedItem !== item) {
        changed = true;
      }
    });

    if (!changed) {
      seen.set(value, value);
      return value;
    }

    return sanitizedArray;
  }

  const source = value as Record<string, unknown>;
  const sanitizedObject: Record<string, unknown> = {};
  seen.set(value, sanitizedObject);

  let changed = false;
  for (const [key, child] of Object.entries(source)) {
    if (prototypePollutionKeys.has(key)) {
      changed = true;
      continue;
    }

    const sanitizedChild = sanitizeApiBoundaryValue(child, seen);
    sanitizedObject[key] = sanitizedChild;
    if (sanitizedChild !== child) {
      changed = true;
    }
  }

  if (!changed) {
    seen.set(value, value);
    return value;
  }

  return sanitizedObject;
}

// ============================================================================
// Shared enums
// ============================================================================

export const WorkspaceRoleSchema = z.enum(["target", "catalog", "embeddings", "workspace"]);

export const WorkspaceDatabaseTypeSchema = z.enum(["sqlite", "neondb", "external"]);

export const WorkspaceBundleStatusSchema = z.enum(["complete", "degraded", "empty"]);

export const EnrichmentSourceSchema = z.enum(["fresh", "cached", "fallback"]);

// ============================================================================
// Sub-schemas
// ============================================================================

export const WorkspaceDatabaseSchema = z
  .object({
    id: z.string(),
    workspaceId: z.string(),
    role: WorkspaceRoleSchema,
    displayName: z.string(),
    databaseUrl: z.string().optional(),
    createdAt: z.string(),
  })
  .passthrough();

export const WorkspaceBundleSchema = z
  .object({
    requiredRoles: z.array(WorkspaceRoleSchema),
    configuredRoles: z.array(WorkspaceRoleSchema),
    missingRoles: z.array(WorkspaceRoleSchema),
    status: WorkspaceBundleStatusSchema,
    complete: z.boolean(),
  })
  .passthrough();

export const WorkspaceSchema = z
  .object({
    id: z.string(),
    displayName: z.string(),
    createdAt: z.string(),
    updatedAt: z.string().optional(),
    databaseType: WorkspaceDatabaseTypeSchema.optional(),
    databases: z.array(WorkspaceDatabaseSchema),
    bundle: WorkspaceBundleSchema.optional(),
  })
  .passthrough();

export const WorkspaceListSchema = z.array(WorkspaceSchema);

export const IngestionSummarySchema = z
  .object({
    tablesDiscovered: z.number(),
    columnsProfiled: z.number(),
    enumsExtracted: z.number(),
    relationshipsFound: z.number(),
    catalogItemsIndexed: z.number(),
    durationMs: z.number(),
    enrichmentDegraded: z.boolean().optional(),
    enrichmentCacheHits: z.number().optional(),
    enrichmentFallbacks: z.number().optional(),
    enrichmentFresh: z.number().optional(),
  })
  .passthrough();

const IngestionStatusQueuedSchema = z
  .object({
    status: z.literal("queued"),
  })
  .passthrough();

const IngestionStatusDiscoveringSchema = z
  .object({
    status: z.literal("discovering"),
    tablesFound: z.number(),
  })
  .passthrough();

const IngestionStatusProfilingSchema = z
  .object({
    status: z.literal("profiling"),
    tablesFound: z.number(),
    columnsProfiled: z.number(),
    totalColumns: z.number(),
  })
  .passthrough();

const IngestionStatusEnrichingSchema = z
  .object({
    status: z.literal("enriching"),
    tablesEnriched: z.number(),
    totalTables: z.number(),
  })
  .passthrough();

const IngestionStatusExtractingSchema = z
  .object({
    status: z.literal("extracting"),
    enumsExtracted: z.number(),
  })
  .passthrough();

const IngestionStatusIndexingSchema = z
  .object({
    status: z.literal("indexing"),
    itemsIndexed: z.number(),
  })
  .passthrough();

const IngestionStatusCompletedSchema = z
  .object({
    status: z.literal("completed"),
    summary: IngestionSummarySchema,
  })
  .passthrough();

const IngestionStatusFailedSchema = z
  .object({
    status: z.literal("failed"),
    error: z.string(),
    partial: IngestionSummarySchema.nullable(),
  })
  .passthrough();

export const IngestionStatusSchema = z.discriminatedUnion("status", [
  IngestionStatusQueuedSchema,
  IngestionStatusDiscoveringSchema,
  IngestionStatusProfilingSchema,
  IngestionStatusEnrichingSchema,
  IngestionStatusExtractingSchema,
  IngestionStatusIndexingSchema,
  IngestionStatusCompletedSchema,
  IngestionStatusFailedSchema,
]);

const IngestionStartedEventSchema = z
  .object({
    type: z.literal("started"),
    jobId: z.string(),
  })
  .passthrough();

const IngestionProgressEventSchema = z
  .object({
    type: z.literal("progress"),
    status: IngestionStatusSchema,
  })
  .passthrough();

const IngestionTableProfiledEventSchema = z
  .object({
    type: z.literal("tableProfiled"),
    tableName: z.string(),
    columns: z.number(),
    durationMs: z.number(),
  })
  .passthrough();

const IngestionTableEnrichedEventSchema = z
  .object({
    type: z.literal("tableEnriched"),
    tableName: z.string(),
    source: EnrichmentSourceSchema,
  })
  .passthrough();

const IngestionTableCompletedEventSchema = z
  .object({
    type: z.literal("tableCompleted"),
    tableName: z.string(),
    summary: IngestionSummarySchema,
  })
  .passthrough();

const IngestionTableFailedEventSchema = z
  .object({
    type: z.literal("tableFailed"),
    tableName: z.string(),
    error: z.string(),
  })
  .passthrough();

const IngestionCompletedEventSchema = z
  .object({
    type: z.literal("completed"),
    summary: IngestionSummarySchema,
  })
  .passthrough();

const IngestionErrorEventSchema = z
  .object({
    type: z.literal("error"),
    message: z.string(),
  })
  .passthrough();

export const IngestionEventSchema = z.discriminatedUnion("type", [
  IngestionStartedEventSchema,
  IngestionProgressEventSchema,
  IngestionTableProfiledEventSchema,
  IngestionTableEnrichedEventSchema,
  IngestionTableCompletedEventSchema,
  IngestionTableFailedEventSchema,
  IngestionCompletedEventSchema,
  IngestionErrorEventSchema,
]);

export const ImportSummarySchema = z
  .object({
    sourceRowCount: z.number(),
    tableRowCounts: z.record(z.string(), z.number()),
    documentsIngested: z.number().optional(),
    knowledgeItemsExtracted: z.number().optional(),
    durationMs: z.number(),
  })
  .passthrough();

export const DataReadinessSchema = z
  .object({
    workspaceId: z.string(),
    ready: z.boolean(),
    status: z.enum(["empty", "ready"]),
    sourceId: z.string().nullable().optional(),
    importJobId: z.string().nullable().optional(),
    profileJobId: z.string().nullable().optional(),
    dataBundle: z
      .object({
        status: WorkspaceBundleStatusSchema.optional(),
        complete: z.boolean().optional(),
        configuredRoles: z.array(z.string()).optional(),
        missingRoles: z.array(z.string()).optional(),
      })
      .passthrough()
      .optional(),
    tableRowCounts: z.record(z.string(), z.number()),
    targetTables: z.array(z.object({ name: z.string(), rowCount: z.number() })),
    documents: z.object({ ingested: z.number() }),
    knowledge: z.object({
      itemsExtracted: z.number(),
      itemIds: z.array(z.string()),
    }),
    catalogProfile: z
      .object({
        summary: z.record(z.string(), z.unknown()),
        profiledTables: z.array(z.string()),
      })
      .passthrough(),
    generatedAt: z.string(),
  })
  .passthrough();

export const ValidationCheckSchema = z
  .object({
    name: z.string(),
    passed: z.boolean(),
    message: z.string(),
  })
  .passthrough();

const ImportUploadingStageSchema = z
  .object({
    stage: z.literal("uploading"),
  })
  .passthrough();

const ImportParsingStageSchema = z
  .object({
    stage: z.literal("parsing"),
  })
  .passthrough();

const ImportCreatingSchemaStageSchema = z
  .object({
    stage: z.literal("creatingSchema"),
  })
  .passthrough();

const ImportLoadingTablesStageSchema = z
  .object({
    stage: z.literal("loadingTables"),
    tablesCompleted: z.number(),
    totalTables: z.number(),
    currentBatchRows: z.number().optional(),
    totalBatchRows: z.number().optional(),
  })
  .passthrough();

const ImportValidatingStageSchema = z
  .object({
    stage: z.literal("validating"),
  })
  .passthrough();

const ImportProfilingStageSchema = z
  .object({
    stage: z.literal("profiling"),
  })
  .passthrough();

const ImportCompletedStageSchema = z
  .object({
    stage: z.literal("completed"),
    summary: ImportSummarySchema,
  })
  .passthrough();

const ImportFailedStageSchema = z
  .object({
    stage: z.literal("failed"),
    error: z.string(),
  })
  .passthrough();

export const ImportStageSchema = z.discriminatedUnion("stage", [
  ImportUploadingStageSchema,
  ImportParsingStageSchema,
  ImportCreatingSchemaStageSchema,
  ImportLoadingTablesStageSchema,
  ImportValidatingStageSchema,
  ImportProfilingStageSchema,
  ImportCompletedStageSchema,
  ImportFailedStageSchema,
]);

const ImportStartedEventSchema = z
  .object({
    type: z.literal("started"),
    jobId: z.string(),
  })
  .passthrough();

const ImportStageProgressEventSchema = z
  .object({
    type: z.literal("stageProgress"),
    stage: ImportStageSchema,
  })
  .passthrough();

const ImportTableLoadedEventSchema = z
  .object({
    type: z.literal("tableLoaded"),
    tableName: z.string(),
    rowCount: z.number(),
  })
  .passthrough();

const ImportBatchProgressEventSchema = z
  .object({
    type: z.literal("batchProgress"),
    tableName: z.string(),
    rowsLoaded: z.number(),
    totalRows: z.number(),
  })
  .passthrough();

const ImportSchemaCreatedEventSchema = z
  .object({
    type: z.literal("schemaCreated"),
    tables: z.array(z.string()),
  })
  .passthrough();

const ImportKnowledgeExtractionStartedEventSchema = z
  .object({
    type: z.literal("knowledgeExtractionStarted"),
    documents: z.number(),
  })
  .passthrough();

const ImportDocumentExtractedEventSchema = z
  .object({
    type: z.literal("documentExtracted"),
    documentId: z.string(),
    documentName: z.string(),
    knowledgeItemsExtracted: z.number(),
  })
  .passthrough();

const ImportValidationPassedEventSchema = z
  .object({
    type: z.literal("validationPassed"),
    checks: z.array(ValidationCheckSchema),
  })
  .passthrough();

const ImportProfilingEventSchema = z
  .object({
    type: z.literal("profilingEvent"),
    inner: IngestionEventSchema,
  })
  .passthrough();

const ImportCompletedEventSchema = z
  .object({
    type: z.literal("completed"),
    summary: ImportSummarySchema,
  })
  .passthrough();

const ImportErrorEventSchema = z
  .object({
    type: z.literal("error"),
    message: z.string(),
  })
  .passthrough();

export const ImportEventSchema = z.discriminatedUnion("type", [
  ImportStartedEventSchema,
  ImportStageProgressEventSchema,
  ImportTableLoadedEventSchema,
  ImportBatchProgressEventSchema,
  ImportSchemaCreatedEventSchema,
  ImportKnowledgeExtractionStartedEventSchema,
  ImportDocumentExtractedEventSchema,
  ImportValidationPassedEventSchema,
  ImportProfilingEventSchema,
  ImportCompletedEventSchema,
  ImportErrorEventSchema,
]);

// ============================================================================
// Validation utility
// ============================================================================

/**
 * Validate an API response against a Zod schema.
 *
 * @param schema - The Zod schema to validate against.
 * @param data - The raw (already normalized) response data.
 * @param context - A human-readable label for error messages.
 * @returns The validated data.
 */
export class ResponseValidationError extends Error {
  readonly context: string;
  readonly issues: z.ZodIssue[];

  constructor(context: string, issues: z.ZodIssue[]) {
    super(`${context} validation failed`);
    this.name = "ResponseValidationError";
    this.context = context;
    this.issues = issues;
  }
}

export function validateResponse<T>(schema: z.ZodType<T>, data: unknown, context: string): T {
  const result = schema.safeParse(sanitizeApiBoundaryValue(data));
  if (!result.success) {
    console.error(`[api] ${context} validation failed:`, result.error.issues);
    throw new ResponseValidationError(context, result.error.issues);
  }
  return result.data;
}
