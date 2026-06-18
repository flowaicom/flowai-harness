import type { AppScope } from "@studio/core/domain/scope";
import {
  BookOpenIcon,
  CalculatorIcon,
  FileTextIcon,
  FolderOpenIcon,
  Loader2Icon,
  PlusIcon,
  TrashIcon,
  UploadIcon,
  XIcon,
} from "lucide-react";
import type { ChangeEvent, DragEvent, KeyboardEvent, ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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

export type ConnectIngestionStatusKey =
  | "queued"
  | "discovering"
  | "profiling"
  | "enriching"
  | "extracting"
  | "indexing"
  | "completed"
  | "failed";

export type ConnectKnowledgeType =
  | "business_rule"
  | "predicate"
  | "terminology"
  | "constraint"
  | "temporal_rule"
  | "implicit_intent"
  | "data_quality"
  | "custom";

export type ConnectExtractionStatus = "pending" | "processing" | "processed" | "failed";

export type ConnectDataAuthority = "workspace" | "catalog";

export interface ConnectDocumentLike {
  readonly id: string;
  readonly name: string;
  readonly content: string;
  readonly authority?: ConnectDataAuthority;
  readonly targetDatabaseId: string | null;
  readonly extractionStatus: ConnectExtractionStatus;
  readonly extractedKnowledgeIds: readonly string[];
  readonly createdAt: string;
}

export interface ConnectKnowledgeItemLike {
  readonly id: string;
  readonly name: string;
  readonly description: string;
  readonly authority?: ConnectDataAuthority;
  readonly knowledgeType: ConnectKnowledgeType;
  readonly scopeTables: readonly string[];
  readonly scopeColumns: readonly string[];
  readonly sqlExpression: string | null;
  readonly synonyms: readonly string[];
  readonly sourceDocumentId: string | null;
}

export interface ConnectMetricItemLike {
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

export type ConnectKnowledgeSourceSpecLike =
  | { readonly type: "localDirectory"; readonly path: string }
  | {
      readonly type: "s3Bucket";
      readonly bucket: string;
      readonly prefix?: string;
      readonly region?: string;
    };

export interface ConnectIngestDocumentEntryLike {
  readonly name: string;
  readonly content: string;
  readonly targetDatabaseId: string;
}

export type ConnectKnowledgeIngestEventLike =
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

export type ConnectIngestionEventLike =
  | { readonly type: "started"; readonly jobId: string }
  | { readonly type: "progress"; readonly status: { readonly status: ConnectIngestionStatusKey } }
  | {
      readonly type: "tableProfiled";
      readonly tableName: string;
      readonly columns: number;
      readonly durationMs: number;
    }
  | { readonly type: "tableEnriched"; readonly tableName: string; readonly source: string }
  | { readonly type: "tableCompleted"; readonly tableName: string; readonly summary: unknown }
  | { readonly type: "tableFailed"; readonly tableName: string; readonly error: string }
  | { readonly type: "completed"; readonly summary: unknown }
  | { readonly type: "error"; readonly message: string };

interface ConnectAsyncErrorLike {
  readonly message: string;
}

interface ConnectIngestionHandlersLike {
  readonly onEvent: (event: ConnectIngestionEventLike) => void;
  readonly onComplete: () => void | Promise<void>;
  readonly onError: (error: ConnectAsyncErrorLike) => void;
}

interface ConnectKnowledgeIngestHandlersLike {
  readonly onEvent: (event: ConnectKnowledgeIngestEventLike) => void;
  readonly onComplete: () => void | Promise<void>;
  readonly onError: (error: ConnectAsyncErrorLike) => void;
}

export interface ConnectKnowledgeRuntimeLike {
  listDocuments(scope: AppScope): Promise<ConnectRuntimeResult<readonly ConnectDocumentLike[]>>;
  browseKnowledge(
    scope: AppScope
  ): Promise<ConnectRuntimeResult<readonly ConnectKnowledgeItemLike[]>>;
  listMetrics(scope: AppScope): Promise<ConnectRuntimeResult<readonly ConnectMetricItemLike[]>>;
  deleteDocument(scope: AppScope, id: string): Promise<ConnectRuntimeResult<unknown>>;
  deleteKnowledgeItem(scope: AppScope, id: string): Promise<ConnectRuntimeResult<unknown>>;
  deleteMetric(scope: AppScope, id: string): Promise<ConnectRuntimeResult<unknown>>;
  extractKnowledge(
    scope: AppScope,
    docId: string,
    handlers: ConnectIngestionHandlersLike
  ): Promise<ConnectRuntimeResult<{ readonly abort: () => void }>>;
  ingestDocuments(
    scope: AppScope,
    entries: readonly ConnectIngestDocumentEntryLike[]
  ): Promise<ConnectRuntimeResult<readonly ConnectDocumentLike[]>>;
  ingestKnowledgeFromSource(
    scope: AppScope,
    source: ConnectKnowledgeSourceSpecLike,
    handlers: ConnectKnowledgeIngestHandlersLike
  ): Promise<ConnectRuntimeResult<{ readonly abort: () => void }>>;
}

type ConnectKnowledgeTab = "documents" | "knowledge" | "metrics";

const CONNECT_KNOWLEDGE_TABS: readonly {
  readonly id: ConnectKnowledgeTab;
  readonly label: string;
}[] = [
  { id: "documents", label: "Documents" },
  { id: "knowledge", label: "Knowledge" },
  { id: "metrics", label: "Metrics" },
];

const CONNECT_KNOWLEDGE_TYPE_LABELS: Record<ConnectKnowledgeType, string> = {
  business_rule: "Business Rule",
  predicate: "Predicate",
  terminology: "Terminology",
  constraint: "Constraint",
  temporal_rule: "Temporal Rule",
  implicit_intent: "Implicit Intent",
  data_quality: "Data Quality",
  custom: "Custom",
};

const CONNECT_KNOWLEDGE_TYPE_CATEGORY: Record<ConnectKnowledgeType, string> = {
  business_rule: "planning",
  constraint: "planning",
  predicate: "execution",
  data_quality: "execution",
  terminology: "knowledge",
  implicit_intent: "knowledge",
  temporal_rule: "discovery",
  custom: "knowledge",
};

const CONNECT_AUTHORITY_BADGE: Record<
  ConnectDataAuthority,
  { readonly label: string; readonly category: string }
> = {
  workspace: { label: "Workspace", category: "discovery" },
  catalog: { label: "Catalog", category: "knowledge" },
};

const CONNECT_INGESTION_STATUS_COLORS: Record<ConnectIngestionStatusKey, string> = {
  queued: "#67748a",
  discovering: "#4f43dd",
  profiling: "#4f43dd",
  enriching: "#4f43dd",
  extracting: "#4f43dd",
  indexing: "#4f43dd",
  completed: "#2ecc81",
  failed: "#c72a1c",
};

const CONNECT_EXTRACTION_STATUS_COLORS: Record<ConnectExtractionStatus, string> = {
  pending: "#67748a",
  processing: "#4f43dd",
  processed: "#2ecc81",
  failed: "#c72a1c",
};

const CONNECT_EXTRACTION_STATUS_LABELS: Record<ConnectExtractionStatus, string> = {
  pending: "Pending",
  processing: "Processing",
  processed: "Processed",
  failed: "Failed",
};

const CONNECT_EXTRACTION_STAGE_PERCENT: Record<ConnectIngestionStatusKey, number> = {
  queued: 0,
  discovering: 20,
  profiling: 40,
  enriching: 60,
  extracting: 80,
  indexing: 95,
  completed: 100,
  failed: 0,
};

const CONNECT_EXTRACTION_STAGE_LABEL: Record<ConnectIngestionStatusKey, string> = {
  queued: "Queued",
  discovering: "Discovering...",
  profiling: "Profiling...",
  enriching: "Enriching...",
  extracting: "Extracting...",
  indexing: "Indexing...",
  completed: "Completed",
  failed: "Failed",
};

export interface ConnectKnowledgePageProps {
  readonly scope: AppScope;
  readonly scopeKey: string;
  readonly runtime: ConnectKnowledgeRuntimeLike;
  readonly documents: readonly ConnectDocumentLike[];
  readonly knowledgeItems: readonly ConnectKnowledgeItemLike[];
  readonly setDocuments: (documents: ConnectDocumentLike[]) => void;
  readonly setKnowledgeItems: (items: ConnectKnowledgeItemLike[]) => void;
  readonly addDocument: (document: ConnectDocumentLike) => void;
  readonly removeDocument: (id: string) => void;
  readonly removeKnowledgeItem: (id: string) => void;
  readonly uploadTargetId: string | null;
  readonly reloadKey?: string | number;
  readonly onRetryLoad?: () => void;
  readonly headerAccessory?: ReactNode;
  readonly subtitle?: ReactNode;
  readonly targetMeta?: ReactNode;
  readonly formatText?: (value: string) => string;
}

export function ConnectKnowledgePage({
  scope,
  scopeKey,
  runtime,
  documents,
  knowledgeItems,
  setDocuments,
  setKnowledgeItems,
  addDocument,
  removeDocument,
  removeKnowledgeItem,
  uploadTargetId,
  reloadKey = 0,
  onRetryLoad,
  headerAccessory,
  subtitle = "Manage documents, extracted knowledge items, and metrics",
  targetMeta,
  formatText = (value) => value,
}: ConnectKnowledgePageProps) {
  const [activeTab, setActiveTab] = useState<ConnectKnowledgeTab>("documents");
  const [isExtracting, setIsExtracting] = useState<string | null>(null);
  const [metrics, setMetrics] = useState<ConnectMetricItemLike[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [extractionStage, setExtractionStage] = useState<ConnectIngestionStatusKey | null>(null);
  const [isIngesting, setIsIngesting] = useState(false);
  const [ingestProgress, setIngestProgress] = useState<ConnectKnowledgeIngestEventLike | null>(
    null
  );
  const loadRequestIdRef = useRef(0);

  useEffect(() => {
    const requestId = loadRequestIdRef.current + 1;
    loadRequestIdRef.current = requestId;
    let cancelled = false;

    const load = async () => {
      setIsLoading(true);
      setError(null);
      setDocuments([]);
      setKnowledgeItems([]);
      setMetrics([]);

      const [docsResult, knowledgeResult, metricsResult] = await Promise.all([
        runtime.listDocuments(scope),
        runtime.browseKnowledge(scope),
        runtime.listMetrics(scope),
      ]);

      if (cancelled || loadRequestIdRef.current !== requestId) {
        return;
      }

      const errors: string[] = [];
      if (isConnectRuntimeOk(docsResult)) {
        setDocuments([...docsResult.value]);
      } else {
        errors.push(`Documents: ${docsResult.error.message}`);
      }

      if (isConnectRuntimeOk(knowledgeResult)) {
        setKnowledgeItems([...knowledgeResult.value]);
      } else {
        errors.push(`Knowledge: ${knowledgeResult.error.message}`);
      }

      if (isConnectRuntimeOk(metricsResult)) {
        setMetrics([...metricsResult.value]);
      } else {
        errors.push(`Metrics: ${metricsResult.error.message}`);
      }

      setError(errors.length > 0 ? `Failed to load: ${errors.join("; ")}` : null);
      setIsLoading(false);
    };

    void load();

    return () => {
      cancelled = true;
    };
  }, [reloadKey, runtime, scope, scopeKey, setDocuments, setKnowledgeItems]);

  const handleDeleteDocument = useCallback(
    async (id: string) => {
      if (!window.confirm("Delete this document?")) {
        return;
      }
      const result = await runtime.deleteDocument(scope, id);
      if (isConnectRuntimeOk(result)) {
        removeDocument(id);
      } else {
        setError(`Failed to delete document: ${result.error.message}`);
      }
    },
    [removeDocument, runtime, scope]
  );

  const handleDeleteKnowledgeItem = useCallback(
    async (id: string) => {
      if (!window.confirm("Delete this knowledge item?")) {
        return;
      }
      const result = await runtime.deleteKnowledgeItem(scope, id);
      if (isConnectRuntimeOk(result)) {
        removeKnowledgeItem(id);
      } else {
        setError(`Failed to delete knowledge item: ${result.error.message}`);
      }
    },
    [removeKnowledgeItem, runtime, scope]
  );

  const handleDeleteMetric = useCallback(
    async (id: string) => {
      if (!window.confirm("Delete this metric?")) {
        return;
      }
      const result = await runtime.deleteMetric(scope, id);
      if (isConnectRuntimeOk(result)) {
        setMetrics((previous) => previous.filter((metric) => metric.id !== id));
      } else {
        setError(`Failed to delete metric: ${result.error.message}`);
      }
    },
    [runtime, scope]
  );

  const refreshKnowledgeData = useCallback(async () => {
    const [knowledgeResult, documentResult] = await Promise.all([
      runtime.browseKnowledge(scope),
      runtime.listDocuments(scope),
    ]);

    if (isConnectRuntimeOk(knowledgeResult)) {
      setKnowledgeItems([...knowledgeResult.value]);
    } else {
      setError(`Failed to refresh knowledge: ${knowledgeResult.error.message}`);
    }

    if (isConnectRuntimeOk(documentResult)) {
      setDocuments([...documentResult.value]);
    } else {
      setError(
        (previous) => previous ?? `Failed to refresh documents: ${documentResult.error.message}`
      );
    }
  }, [runtime, scope, setDocuments, setKnowledgeItems]);

  const handleExtract = useCallback(
    async (documentId: string) => {
      setIsExtracting(documentId);
      setError(null);
      setExtractionStage(null);

      const result = await runtime.extractKnowledge(scope, documentId, {
        onEvent: (event) => {
          if (event.type === "progress") {
            setExtractionStage(event.status.status);
          }
        },
        onComplete: async () => {
          try {
            await refreshKnowledgeData();
          } catch (caught) {
            setError(
              `Failed to refresh extracted content: ${
                caught instanceof Error ? caught.message : "Unknown error"
              }`
            );
          } finally {
            setIsExtracting(null);
            setExtractionStage(null);
          }
        },
        onError: (asyncError) => {
          setIsExtracting(null);
          setExtractionStage(null);
          setError(`Extraction failed: ${asyncError.message}`);
        },
      });

      if (!isConnectRuntimeOk(result)) {
        setIsExtracting(null);
        setExtractionStage(null);
        setError(`Extraction failed: ${result.error.message}`);
      }
    },
    [refreshKnowledgeData, runtime, scope]
  );

  const handleUpload = useCallback(
    async (entries: readonly ConnectIngestDocumentEntryLike[]) => {
      setError(null);
      const result = await runtime.ingestDocuments(scope, entries);
      if (isConnectRuntimeOk(result)) {
        for (const document of result.value) {
          addDocument(document);
        }
      } else {
        setError(`Failed to upload documents: ${result.error.message}`);
      }
    },
    [addDocument, runtime, scope]
  );

  const handleIngestFromSource = useCallback(
    async (source: ConnectKnowledgeSourceSpecLike) => {
      setIsIngesting(true);
      setIngestProgress(null);
      setError(null);

      const result = await runtime.ingestKnowledgeFromSource(scope, source, {
        onEvent: (event) => {
          setIngestProgress(event);
        },
        onComplete: async () => {
          setIsIngesting(false);
          try {
            await refreshKnowledgeData();
          } catch (caught) {
            setError(
              `Failed to refresh knowledge data: ${
                caught instanceof Error ? caught.message : "Unknown error"
              }`
            );
          }
        },
        onError: (asyncError) => {
          setIsIngesting(false);
          setError(`Knowledge ingestion failed: ${asyncError.message}`);
        },
      });

      if (!isConnectRuntimeOk(result)) {
        setIsIngesting(false);
        setError(`Knowledge ingestion failed: ${result.error.message}`);
      }
    },
    [refreshKnowledgeData, runtime, scope]
  );

  const tabsWithCounts = useMemo(
    () =>
      CONNECT_KNOWLEDGE_TABS.map((tab) => ({
        ...tab,
        count:
          tab.id === "documents"
            ? documents.length
            : tab.id === "knowledge"
              ? knowledgeItems.length
              : metrics.length,
      })),
    [documents.length, knowledgeItems.length, metrics.length]
  );

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      <div className="px-6 py-4 border-b">
        <div className={headerAccessory ? "flex items-center justify-between mb-1" : undefined}>
          <h1 className="text-lg font-semibold">Knowledge</h1>
          {headerAccessory}
        </div>
        <p className="text-sm text-muted-foreground">{subtitle}</p>
        {targetMeta ? <div className="text-xs text-muted-foreground mt-1">{targetMeta}</div> : null}
      </div>

      <div className="px-6 py-3 border-b">
        <ConnectPillTabs tabs={tabsWithCounts} active={activeTab} onChange={setActiveTab} />
      </div>

      <div className="flex-1 overflow-y-auto scroll-container">
        <div className="max-w-3xl mx-auto p-6 space-y-6">
          {error ? (
            <ConnectErrorBanner
              message={error}
              onDismiss={() => setError(null)}
              onRetry={onRetryLoad}
            />
          ) : null}

          {isLoading ? (
            <ConnectSectionCard>
              <ConnectSectionHeader>Loading...</ConnectSectionHeader>
              <div className="space-y-2">
                {Array.from({ length: 4 }, (_, index) => (
                  <div
                    key={`knowledge-skeleton-${index}`}
                    className="flex items-center gap-3 py-2.5"
                    style={{ animationDelay: `${index * 75}ms` }}
                  >
                    <div
                      className="h-4 bg-muted rounded animate-pulse flex-1"
                      style={{ maxWidth: `${75 - index * 8}%` }}
                    />
                    <div className="h-5 w-16 bg-muted/60 rounded-full animate-pulse" />
                  </div>
                ))}
              </div>
            </ConnectSectionCard>
          ) : activeTab === "documents" ? (
            <DocumentsTab
              documents={documents}
              uploadTargetId={uploadTargetId}
              extractingId={isExtracting}
              extractionStage={extractionStage}
              onExtract={handleExtract}
              onDelete={handleDeleteDocument}
              onUpload={handleUpload}
              onIngestFromSource={handleIngestFromSource}
              isIngesting={isIngesting}
              ingestProgress={ingestProgress}
              formatText={formatText}
            />
          ) : activeTab === "knowledge" ? (
            <KnowledgeTab
              items={knowledgeItems}
              onDelete={handleDeleteKnowledgeItem}
              formatText={formatText}
            />
          ) : (
            <MetricsTab items={metrics} onDelete={handleDeleteMetric} formatText={formatText} />
          )}
        </div>
      </div>
    </div>
  );
}

function IngestProgressDisplay({
  progress,
}: {
  readonly progress: ConnectKnowledgeIngestEventLike;
}) {
  switch (progress.type) {
    case "discovered":
      return (
        <p className="text-xs text-muted-foreground">
          Discovered {progress.totalFiles} file{progress.totalFiles !== 1 ? "s" : ""}
        </p>
      );
    case "ingesting":
      return (
        <ProgressBar
          label={progress.fileName}
          value={progress.current}
          total={progress.total}
          prefix={null}
        />
      );
    case "extracting":
      return (
        <ProgressBar
          label={progress.fileName}
          value={progress.current}
          total={progress.total}
          prefix="Extracting"
        />
      );
    case "completed":
      return (
        <p className="text-xs text-[var(--dot-emerald)]">
          Done: {progress.documentsIngested} ingested, {progress.documentsSkipped} skipped
          {progress.knowledgeItemsExtracted > 0
            ? `, ${progress.knowledgeItemsExtracted} knowledge items extracted`
            : ""}
        </p>
      );
    case "error":
      return <p className="text-xs text-destructive">{progress.message}</p>;
  }
}

function ProgressBar({
  label,
  value,
  total,
  prefix,
}: {
  readonly label: string;
  readonly value: number;
  readonly total: number;
  readonly prefix: string | null;
}) {
  return (
    <div className="space-y-1">
      <div className="flex justify-between text-xs text-muted-foreground gap-2">
        <span className="truncate">{prefix ? `${prefix}: ${label}` : label}</span>
        <span className="shrink-0 font-mono tabular-nums">
          {value}/{total}
        </span>
      </div>
      <div className="h-1.5 bg-muted rounded-full overflow-hidden">
        <div
          className="h-full rounded-full transition-all duration-300"
          style={{
            width: `${(value / total) * 100}%`,
            backgroundColor: "#4f43dd",
          }}
        />
      </div>
    </div>
  );
}

interface DocumentsTabProps {
  readonly documents: readonly ConnectDocumentLike[];
  readonly uploadTargetId: string | null;
  readonly extractingId: string | null;
  readonly extractionStage: ConnectIngestionStatusKey | null;
  readonly onExtract: (id: string) => void;
  readonly onDelete: (id: string) => void;
  readonly onUpload: (entries: readonly ConnectIngestDocumentEntryLike[]) => Promise<void> | void;
  readonly onIngestFromSource: (source: ConnectKnowledgeSourceSpecLike) => Promise<void> | void;
  readonly isIngesting: boolean;
  readonly ingestProgress: ConnectKnowledgeIngestEventLike | null;
  readonly formatText: (value: string) => string;
}

function DocumentsTab({
  documents,
  uploadTargetId,
  extractingId,
  extractionStage,
  onExtract,
  onDelete,
  onUpload,
  onIngestFromSource,
  isIngesting,
  ingestProgress,
  formatText,
}: DocumentsTabProps) {
  const [showUpload, setShowUpload] = useState(false);
  const [showIngest, setShowIngest] = useState(false);
  const [ingestPath, setIngestPath] = useState("");
  const [isUploading, setIsUploading] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const [pendingFiles, setPendingFiles] = useState<
    Array<{ readonly name: string; readonly content: string }>
  >([]);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const dragDepthRef = useRef(0);
  const canUpload = uploadTargetId !== null;

  const readFiles = useCallback((files: FileList | File[]) => {
    for (const file of Array.from(files)) {
      const reader = new FileReader();
      reader.onload = () => {
        setPendingFiles((previous) => [
          ...previous,
          { name: file.name, content: String(reader.result ?? "") },
        ]);
      };
      reader.readAsText(file);
    }
  }, []);

  const handleFileSelect = useCallback(
    (event: ChangeEvent<HTMLInputElement>) => {
      if (event.target.files) {
        readFiles(event.target.files);
      }
      event.target.value = "";
    },
    [readFiles]
  );

  const handleDragEnter = useCallback((event: DragEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    dragDepthRef.current += 1;
    setIsDragging(true);
  }, []);

  const handleDragOver = useCallback((event: DragEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    event.dataTransfer.dropEffect = "copy";
    setIsDragging(true);
  }, []);

  const handleDragLeave = useCallback((event: DragEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    dragDepthRef.current = Math.max(0, dragDepthRef.current - 1);
    if (dragDepthRef.current === 0) {
      setIsDragging(false);
    }
  }, []);

  const handleDrop = useCallback(
    (event: DragEvent<HTMLButtonElement>) => {
      event.preventDefault();
      event.stopPropagation();
      dragDepthRef.current = 0;
      setIsDragging(false);
      const droppedFiles = extractDroppedFiles(event.dataTransfer);
      if (droppedFiles.length > 0) {
        readFiles(droppedFiles);
      }
    },
    [readFiles]
  );

  const handleDropZoneKeyDown = useCallback((event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      fileInputRef.current?.click();
    }
  }, []);

  const handleSubmitIngest = useCallback(() => {
    if (!ingestPath.trim()) {
      return;
    }

    const source: ConnectKnowledgeSourceSpecLike = ingestPath.startsWith("s3://")
      ? (() => {
          const withoutProtocol = ingestPath.slice(5);
          const slashIndex = withoutProtocol.indexOf("/");
          return {
            type: "s3Bucket" as const,
            bucket: slashIndex > 0 ? withoutProtocol.slice(0, slashIndex) : withoutProtocol,
            prefix: slashIndex > 0 ? withoutProtocol.slice(slashIndex + 1) : undefined,
          };
        })()
      : { type: "localDirectory" as const, path: ingestPath.trim() };

    void onIngestFromSource(source);
  }, [ingestPath, onIngestFromSource]);

  const handleRemovePending = useCallback((index: number) => {
    setPendingFiles((previous) => previous.filter((_, candidate) => candidate !== index));
  }, []);

  const handleSubmitUpload = useCallback(async () => {
    if (pendingFiles.length === 0 || !uploadTargetId) {
      return;
    }
    setIsUploading(true);
    const entries: ConnectIngestDocumentEntryLike[] = pendingFiles.map((file) => ({
      name: file.name,
      content: file.content,
      targetDatabaseId: uploadTargetId,
    }));
    await onUpload(entries);
    setIsUploading(false);
    setPendingFiles([]);
    setShowUpload(false);
  }, [onUpload, pendingFiles, uploadTargetId]);

  return (
    <>
      {showUpload ? (
        <ConnectSectionCard>
          <div className="flex items-center justify-between">
            <ConnectSectionHeader>Upload Documents</ConnectSectionHeader>
            <button
              type="button"
              onClick={() => {
                setShowUpload(false);
                setPendingFiles([]);
              }}
              className="p-1 rounded hover:bg-muted transition-colors"
            >
              <XIcon className="size-4" />
            </button>
          </div>

          <button
            type="button"
            onClick={() => fileInputRef.current?.click()}
            onKeyDown={handleDropZoneKeyDown}
            onDragEnter={handleDragEnter}
            onDragOver={handleDragOver}
            onDragLeave={handleDragLeave}
            onDrop={handleDrop}
            className={cx(
              "w-full border-2 border-dashed rounded-lg p-6 text-center transition-colors",
              isDragging
                ? "border-primary bg-primary/5"
                : "hover:border-primary/50 hover:bg-muted/50"
            )}
          >
            <UploadIcon className="size-6 mx-auto mb-2 text-muted-foreground" />
            <p className="text-sm text-muted-foreground">
              {isDragging ? "Drop files here" : "Click or drag files here"}
            </p>
            <p className="text-xs text-muted-foreground/60 mt-1">.txt, .md, .csv, .json, .sql</p>
          </button>
          <input
            ref={fileInputRef}
            type="file"
            multiple
            accept=".txt,.md,.csv,.json,.sql,.tsv,.xml,.yaml,.yml"
            onChange={handleFileSelect}
            className="hidden"
          />

          {pendingFiles.length > 0 ? (
            <div className="space-y-1.5">
              {pendingFiles.map((file, index) => (
                <div
                  key={`${file.name}-${index}`}
                  className="flex items-center gap-2 px-2.5 py-1.5 rounded-md border text-xs"
                >
                  <FileTextIcon className="size-3.5 text-muted-foreground shrink-0" />
                  <span className="flex-1 truncate">{formatText(file.name)}</span>
                  <span className="text-muted-foreground font-mono tabular-nums shrink-0">
                    {(file.content.length / 1024).toFixed(1)} KB
                  </span>
                  <button
                    type="button"
                    onClick={() => handleRemovePending(index)}
                    className="p-0.5 rounded hover:bg-destructive/10 hover:text-destructive transition-colors"
                  >
                    <XIcon className="size-3" />
                  </button>
                </div>
              ))}
              <button
                type="button"
                onClick={handleSubmitUpload}
                disabled={isUploading || !canUpload}
                className="w-full py-2 text-xs font-medium bg-primary text-primary-foreground rounded-md hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed transition-colors flex items-center justify-center gap-2"
              >
                {isUploading ? <Loader2Icon className="size-3 animate-spin" /> : null}
                {isUploading
                  ? "Uploading..."
                  : `Upload ${pendingFiles.length} document${pendingFiles.length > 1 ? "s" : ""}`}
              </button>
            </div>
          ) : null}
        </ConnectSectionCard>
      ) : (
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => setShowUpload(true)}
            disabled={!canUpload}
            className="flex items-center gap-2 px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed transition-colors text-sm font-medium focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          >
            <PlusIcon className="size-4" />
            Upload Documents
          </button>
          <button
            type="button"
            onClick={() => setShowIngest(true)}
            className="flex items-center gap-2 px-4 py-2 border rounded-md hover:bg-muted transition-colors text-sm font-medium focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          >
            <FolderOpenIcon className="size-4" />
            Ingest from Source
          </button>
        </div>
      )}

      {showIngest ? (
        <ConnectSectionCard>
          <div className="flex items-center justify-between">
            <ConnectSectionHeader>Ingest from Directory or S3</ConnectSectionHeader>
            <button
              type="button"
              onClick={() => {
                setShowIngest(false);
                setIngestPath("");
              }}
              className="p-1 rounded hover:bg-muted transition-colors"
            >
              <XIcon className="size-4" />
            </button>
          </div>
          <div className="space-y-3">
            <div>
              <label htmlFor="ingest-path" className="block text-xs text-muted-foreground mb-1">
                Path (local directory or s3://bucket/prefix)
              </label>
              <input
                id="ingest-path"
                type="text"
                value={ingestPath}
                onChange={(event) => setIngestPath(event.target.value)}
                placeholder="/path/to/knowledge or s3://my-bucket/docs/"
                className="w-full px-3 py-2 text-sm border rounded-md bg-background focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                disabled={isIngesting}
              />
            </div>
            <button
              type="button"
              onClick={handleSubmitIngest}
              disabled={isIngesting || !ingestPath.trim()}
              className="w-full py-2 text-xs font-medium bg-primary text-primary-foreground rounded-md hover:bg-primary/90 disabled:opacity-50 transition-colors flex items-center justify-center gap-2"
            >
              {isIngesting ? <Loader2Icon className="size-3 animate-spin" /> : null}
              {isIngesting ? "Ingesting..." : "Start Ingestion"}
            </button>
          </div>

          {isIngesting && ingestProgress ? (
            <IngestProgressDisplay progress={ingestProgress} />
          ) : null}
        </ConnectSectionCard>
      ) : null}

      {documents.length === 0 && !showUpload ? (
        <ConnectEmptyState
          icon={UploadIcon}
          title={canUpload ? "No documents uploaded" : "No target selected"}
          description={
            canUpload
              ? "Upload text documents to extract knowledge items and enrich the agent's understanding"
              : "Select a source or switch to a non-default workspace to attach uploaded documents to a concrete database context"
          }
          action={
            canUpload
              ? { label: "Upload Documents", onClick: () => setShowUpload(true) }
              : undefined
          }
        />
      ) : documents.length > 0 ? (
        <ConnectSectionCard>
          <ConnectSectionHeader>Documents ({documents.length})</ConnectSectionHeader>
          <div className="divide-y">
            {documents.map((document) => {
              const isExtractingThis = extractingId === document.id;
              const extractionPercent =
                isExtractingThis && extractionStage
                  ? CONNECT_EXTRACTION_STAGE_PERCENT[extractionStage]
                  : 0;
              const isCatalogDocument = document.authority === "catalog";
              const authorityBadge = document.authority
                ? CONNECT_AUTHORITY_BADGE[document.authority]
                : null;

              return (
                <div key={document.id} className="py-2.5 first:pt-0 last:pb-0 space-y-2">
                  <div className="flex items-center gap-3">
                    <FileTextIcon className="size-4 text-muted-foreground shrink-0" />
                    <div className="flex-1 min-w-0">
                      <div className="font-medium text-sm truncate">
                        {formatText(document.name)}
                      </div>
                      <div className="flex items-center gap-2 mt-0.5 flex-wrap">
                        {authorityBadge ? (
                          <ConnectCategoryBadge
                            label={authorityBadge.label}
                            category={authorityBadge.category}
                          />
                        ) : null}
                        <StatusPill status={document.extractionStatus} />
                        {document.extractedKnowledgeIds.length > 0 ? (
                          <span className="text-xs text-muted-foreground font-mono tabular-nums">
                            {document.extractedKnowledgeIds.length} items
                          </span>
                        ) : null}
                      </div>
                    </div>
                    <button
                      type="button"
                      onClick={() => onExtract(document.id)}
                      disabled={
                        isCatalogDocument ||
                        isExtractingThis ||
                        document.extractionStatus === "processing"
                      }
                      className={cx(
                        "px-2.5 py-1 text-xs rounded-md transition-colors shrink-0",
                        "text-muted-foreground hover:text-foreground hover:bg-muted",
                        "disabled:opacity-40 disabled:cursor-not-allowed"
                      )}
                    >
                      {isCatalogDocument
                        ? "Read-only"
                        : isExtractingThis
                          ? "Extracting..."
                          : "Extract"}
                    </button>
                    {!isCatalogDocument ? (
                      <button
                        type="button"
                        onClick={() => onDelete(document.id)}
                        className="p-1 rounded hover:bg-destructive/10 hover:text-destructive transition-colors shrink-0 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                      >
                        <TrashIcon className="size-3.5" />
                      </button>
                    ) : null}
                  </div>

                  {!isCatalogDocument && isExtractingThis && extractionStage ? (
                    <div className="ml-7">
                      <div className="h-1.5 bg-muted rounded-full overflow-hidden">
                        <div
                          className="h-full animate-pulse rounded-full transition-all duration-700"
                          style={{
                            width: `${extractionPercent}%`,
                            backgroundColor: CONNECT_INGESTION_STATUS_COLORS.profiling,
                          }}
                        />
                      </div>
                      <div className="text-[10px] text-muted-foreground mt-0.5">
                        {CONNECT_EXTRACTION_STAGE_LABEL[extractionStage]}
                      </div>
                    </div>
                  ) : null}
                </div>
              );
            })}
          </div>
        </ConnectSectionCard>
      ) : null}
    </>
  );
}

function StatusPill({ status }: { readonly status: ConnectExtractionStatus }) {
  return (
    <span
      className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-xs border"
      style={{
        color: CONNECT_EXTRACTION_STATUS_COLORS[status],
        borderColor: `${CONNECT_EXTRACTION_STATUS_COLORS[status]}33`,
        backgroundColor: `${CONNECT_EXTRACTION_STATUS_COLORS[status]}12`,
      }}
    >
      {CONNECT_EXTRACTION_STATUS_LABELS[status]}
    </span>
  );
}

function KnowledgeTab({
  items,
  onDelete,
  formatText,
}: {
  readonly items: readonly ConnectKnowledgeItemLike[];
  readonly onDelete: (id: string) => void;
  readonly formatText: (value: string) => string;
}) {
  if (items.length === 0) {
    return (
      <ConnectEmptyState
        icon={BookOpenIcon}
        title="No knowledge items"
        description="Extract knowledge from documents to populate business rules, predicates, and terminology"
      />
    );
  }

  return (
    <ConnectSectionCard>
      <ConnectSectionHeader>Knowledge Items ({items.length})</ConnectSectionHeader>
      <div className="divide-y">
        {items.map((item) => (
          <div key={item.id} className="py-2.5 first:pt-0 last:pb-0 space-y-1.5">
            <div className="flex items-center gap-2">
              <span className="font-medium text-sm truncate flex-1">{formatText(item.name)}</span>
              {item.authority ? (
                <ConnectCategoryBadge
                  label={CONNECT_AUTHORITY_BADGE[item.authority].label}
                  category={CONNECT_AUTHORITY_BADGE[item.authority].category}
                />
              ) : null}
              <ConnectCategoryBadge
                label={CONNECT_KNOWLEDGE_TYPE_LABELS[item.knowledgeType]}
                category={CONNECT_KNOWLEDGE_TYPE_CATEGORY[item.knowledgeType]}
              />
              {item.authority !== "catalog" ? (
                <button
                  type="button"
                  onClick={() => onDelete(item.id)}
                  className="p-1 rounded hover:bg-destructive/10 hover:text-destructive transition-colors shrink-0 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                >
                  <TrashIcon className="size-3.5" />
                </button>
              ) : null}
            </div>

            <p className="text-xs text-muted-foreground">{formatText(item.description)}</p>

            {item.scopeTables.length > 0 ? (
              <div className="flex gap-1 flex-wrap">
                {item.scopeTables.map((table) => (
                  <ConnectCategoryBadge
                    key={table}
                    label={formatText(table)}
                    category="discovery"
                    className="font-mono"
                  />
                ))}
              </div>
            ) : null}

            {item.sqlExpression ? (
              <pre className="p-2.5 bg-muted/50 rounded-md text-xs font-mono overflow-x-auto border">
                {formatText(item.sqlExpression)}
              </pre>
            ) : null}
          </div>
        ))}
      </div>
    </ConnectSectionCard>
  );
}

function MetricsTab({
  items,
  onDelete,
  formatText,
}: {
  readonly items: readonly ConnectMetricItemLike[];
  readonly onDelete: (id: string) => void;
  readonly formatText: (value: string) => string;
}) {
  if (items.length === 0) {
    return (
      <ConnectEmptyState
        icon={CalculatorIcon}
        title="No metrics defined"
        description="Create metrics to define business KPIs and computed measures"
      />
    );
  }

  return (
    <ConnectSectionCard>
      <ConnectSectionHeader>Metrics ({items.length})</ConnectSectionHeader>
      <div className="divide-y">
        {items.map((item) => (
          <div key={item.id} className="py-2.5 first:pt-0 last:pb-0 space-y-1.5">
            <div className="flex items-center gap-2">
              <span className="font-medium text-sm truncate flex-1">
                {formatText(item.displayName)}
              </span>
              <ConnectCategoryBadge label={item.aggregationType} category="execution" />
              {item.timeGrain ? (
                <ConnectCategoryBadge label={item.timeGrain} category="discovery" />
              ) : null}
              <button
                type="button"
                onClick={() => onDelete(item.id)}
                className="p-1 rounded hover:bg-destructive/10 hover:text-destructive transition-colors shrink-0 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              >
                <TrashIcon className="size-3.5" />
              </button>
            </div>

            <p className="text-xs text-muted-foreground">{formatText(item.formulaDescription)}</p>

            <pre className="p-2.5 bg-muted/50 rounded-md text-xs font-mono overflow-x-auto border">
              {formatText(item.formula)}
            </pre>

            {item.sourceTables.length > 0 ? (
              <div className="flex gap-1 flex-wrap">
                {item.sourceTables.map((table) => (
                  <ConnectCategoryBadge
                    key={table}
                    label={formatText(table)}
                    category="discovery"
                    className="font-mono"
                  />
                ))}
              </div>
            ) : null}
          </div>
        ))}
      </div>
    </ConnectSectionCard>
  );
}

function extractDroppedFiles(dataTransfer: DataTransfer): File[] {
  const itemFiles = Array.from(dataTransfer.items ?? [])
    .filter((item) => item.kind === "file")
    .map((item) => item.getAsFile())
    .filter((file): file is File => file !== null);

  return itemFiles.length > 0 ? itemFiles : Array.from(dataTransfer.files ?? []);
}
