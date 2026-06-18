import type { LucideIcon } from "lucide-react";
import {
  FileUpIcon,
  Loader2Icon,
  PlayIcon,
  RotateCcwIcon,
  SquareIcon,
  UploadIcon,
  XIcon,
} from "lucide-react";
import { type ReactNode, useCallback, useMemo, useRef, useState } from "react";
import {
  ConnectEmptyState,
  ConnectErrorBanner,
  ConnectSectionCard,
  ConnectSectionHeader,
} from "./connect-page-primitives";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

function formatElapsed(durationMs: number): string {
  if (durationMs < 1000) {
    return "0s";
  }
  const totalSeconds = Math.floor(durationMs / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes === 0) {
    return `${seconds}s`;
  }
  return `${minutes}m ${seconds}s`;
}

function extractDroppedFiles(dataTransfer: DataTransfer): File[] {
  const itemFiles = Array.from(dataTransfer.items ?? [])
    .filter((item) => item.kind === "file")
    .map((item) => item.getAsFile())
    .filter((file): file is File => file !== null);

  return itemFiles.length > 0 ? itemFiles : Array.from(dataTransfer.files ?? []);
}

function stageToIndex(
  stages: readonly ConnectImportPipelineStageLike[],
  stageKey: string | null
): number {
  if (!stageKey) {
    return -1;
  }
  return stages.findIndex((stage) => stage.key === stageKey);
}

function useHighWaterStage(
  stages: readonly ConnectImportPipelineStageLike[],
  currentStage: string | null,
  runKey: string | null | undefined
): number {
  const ref = useRef(-1);
  const lastRunKeyRef = useRef<string | null | undefined>(runKey);
  if (lastRunKeyRef.current !== runKey) {
    ref.current = -1;
    lastRunKeyRef.current = runKey;
  }
  const currentIndex = stageToIndex(stages, currentStage);
  if (currentIndex > ref.current) {
    ref.current = currentIndex;
  }
  return ref.current;
}

type ConnectImportStepVisualState = "pending" | "active" | "completed" | "failed";

const IMPORT_STATUS_COLORS: Record<ConnectImportStepVisualState, string> = {
  pending: "var(--muted-foreground)",
  active: "var(--primary)",
  completed: "var(--dot-emerald)",
  failed: "var(--dot-red)",
};

function resolveStageVisualState({
  index,
  currentStage,
  highWater,
  activeIndex,
  stageCount,
}: {
  readonly index: number;
  readonly currentStage: string | null;
  readonly highWater: number;
  readonly activeIndex: number;
  readonly stageCount: number;
}): ConnectImportStepVisualState {
  if (currentStage === "failed") {
    if (index < highWater) return "completed";
    if (index === highWater) return "failed";
    return "pending";
  }
  if (currentStage === "completed" || activeIndex >= stageCount) {
    return "completed";
  }
  if (index < activeIndex) return "completed";
  if (index === activeIndex) return "active";
  return "pending";
}

function ConnectImportStageProgressBar({
  stages,
  currentStage,
  runKey,
  profilingTotal,
  profilingCompleted,
}: {
  readonly stages: readonly ConnectImportPipelineStageLike[];
  readonly currentStage: string | null;
  readonly runKey?: string | null;
  readonly profilingTotal: number;
  readonly profilingCompleted: number;
}) {
  const highWater = useHighWaterStage(stages, currentStage, runKey);
  const activeIndex = stageToIndex(stages, currentStage);

  return (
    <div className="flex items-center gap-1">
      {stages.map((stage, index) => {
        const status = resolveStageVisualState({
          index,
          currentStage,
          highWater,
          activeIndex,
          stageCount: stages.length,
        });
        const sublabel =
          stage.key === "profiling" && status === "active" && profilingTotal > 0
            ? `${profilingCompleted}/${profilingTotal}`
            : null;

        return (
          <div key={stage.key} className="flex flex-1 flex-col items-center gap-1">
            <div
              className={cx(
                "h-2 w-full rounded-full transition-all duration-300",
                status === "active" && "animate-pulse"
              )}
              style={{ backgroundColor: IMPORT_STATUS_COLORS[status] }}
            />
            <span
              className={cx(
                "text-[10px] font-medium",
                status === "active"
                  ? "text-primary"
                  : status === "completed"
                    ? "text-[var(--dot-emerald)]"
                    : status === "failed"
                      ? "text-destructive"
                      : "text-muted-foreground"
              )}
            >
              {stage.label}
              {sublabel ? <span className="ml-0.5 text-muted-foreground">({sublabel})</span> : null}
            </span>
          </div>
        );
      })}
    </div>
  );
}

function ConnectImportSummarySection({
  title,
  metrics,
}: {
  readonly title: string;
  readonly metrics: readonly ConnectImportSummaryMetric[];
}) {
  return (
    <ConnectSectionCard>
      <ConnectSectionHeader>{title}</ConnectSectionHeader>
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
        {metrics.map((metric) => (
          <div key={metric.label}>
            <p className="text-2xl font-semibold tabular-nums">
              {typeof metric.value === "number" ? metric.value.toLocaleString() : metric.value}
            </p>
            <p className="text-xs text-muted-foreground">{metric.label}</p>
          </div>
        ))}
      </div>
    </ConnectSectionCard>
  );
}

export interface ConnectImportPipelineStageLike {
  readonly key: string;
  readonly label: string;
}

export interface ConnectImportFileLike {
  readonly name: string;
  readonly size: number;
}

export interface ConnectImportSummaryMetric {
  readonly label: string;
  readonly value: string | number;
}

export interface ConnectImportPageProps {
  readonly title?: string;
  readonly subtitle?: ReactNode;
  readonly headingAccessory?: ReactNode;
  readonly targetMeta?: ReactNode;
  readonly startLabel?: string;
  readonly resetLabel?: string;
  readonly retryLabel?: string;
  readonly stopLabel?: string;
  readonly filesSectionTitle?: string;
  readonly fileUploadLabel: ReactNode;
  readonly fileUploadDescription?: ReactNode;
  readonly fileInputAccept?: string;
  readonly files: readonly ConnectImportFileLike[];
  readonly onAddFiles: (files: FileList | File[]) => void;
  readonly onRemoveFile: (fileName: string) => void;
  readonly onStart: () => void;
  readonly onCancel: () => void;
  readonly onReset: () => void;
  readonly onRetry?: () => void;
  readonly startDisabled?: boolean;
  readonly isRunning: boolean;
  readonly isCompleted: boolean;
  readonly isFailed: boolean;
  readonly elapsedMs: number;
  readonly pipelineStages: readonly ConnectImportPipelineStageLike[];
  readonly currentStage: string | null;
  readonly runKey?: string | null;
  readonly profilingTotal: number;
  readonly profilingCompleted: number;
  readonly reconnecting?: {
    readonly attempt?: number | null;
    readonly maxAttempts?: number;
    readonly message?: ReactNode;
  } | null;
  readonly error?: string | null;
  readonly onDismissError: () => void;
  readonly failureMessage?: string | null;
  readonly onDismissFailure?: () => void;
  readonly controls?: ReactNode;
  readonly targetWarning?: string | null;
  readonly onDismissTargetWarning?: () => void;
  readonly tableSection?: ReactNode;
  readonly progressSection?: ReactNode;
  readonly showFileSection?: boolean;
  readonly summaryMetrics?: readonly ConnectImportSummaryMetric[] | null;
  readonly summaryTitle?: string;
  readonly nextSteps?: ReactNode;
  readonly showEmptyState?: boolean;
  readonly emptyState?: {
    readonly title: string;
    readonly description: string;
    readonly actionLabel?: string;
    readonly icon?: LucideIcon;
  };
  readonly formatText?: (value: string) => string;
}

export function ConnectImportPage({
  title = "Import",
  subtitle = "Upload data files for import and profiling",
  headingAccessory,
  targetMeta,
  startLabel = "Start Import",
  resetLabel = "Reset",
  retryLabel = "Retry",
  stopLabel = "Stop",
  filesSectionTitle = "Files",
  fileUploadLabel,
  fileUploadDescription,
  fileInputAccept,
  files,
  onAddFiles,
  onRemoveFile,
  onStart,
  onCancel,
  onReset,
  onRetry,
  startDisabled = false,
  isRunning,
  isCompleted,
  isFailed,
  elapsedMs,
  pipelineStages,
  currentStage,
  runKey,
  profilingTotal,
  profilingCompleted,
  reconnecting,
  error,
  onDismissError,
  failureMessage,
  onDismissFailure,
  controls,
  targetWarning,
  onDismissTargetWarning,
  tableSection,
  progressSection,
  showFileSection = true,
  summaryMetrics,
  summaryTitle = "Summary",
  nextSteps,
  showEmptyState = false,
  emptyState,
  formatText = (value) => value,
}: ConnectImportPageProps) {
  const [isDragOver, setIsDragOver] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const dragDepthRef = useRef(0);

  const reconnectingMessage = useMemo(() => {
    if (!reconnecting) {
      return null;
    }
    if (reconnecting.message) {
      return reconnecting.message;
    }
    if (reconnecting.attempt != null && reconnecting.maxAttempts != null) {
      return `Reconnecting to import stream (attempt ${reconnecting.attempt}/${reconnecting.maxAttempts}) ...`;
    }
    return "Reconnecting to import stream...";
  }, [reconnecting]);

  const handleDrop = useCallback(
    (event: React.DragEvent<HTMLButtonElement>) => {
      event.preventDefault();
      event.stopPropagation();
      dragDepthRef.current = 0;
      setIsDragOver(false);
      const droppedFiles = extractDroppedFiles(event.dataTransfer);
      if (droppedFiles.length > 0) {
        onAddFiles(droppedFiles);
      }
    },
    [onAddFiles]
  );

  const handleDragEnter = useCallback((event: React.DragEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    dragDepthRef.current += 1;
    setIsDragOver(true);
  }, []);

  const handleDragOver = useCallback((event: React.DragEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    event.dataTransfer.dropEffect = "copy";
    setIsDragOver(true);
  }, []);

  const handleDragLeave = useCallback((event: React.DragEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    dragDepthRef.current = Math.max(0, dragDepthRef.current - 1);
    if (dragDepthRef.current === 0) {
      setIsDragOver(false);
    }
  }, []);

  const handleDropZoneKeyDown = useCallback((event: React.KeyboardEvent<HTMLButtonElement>) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      fileInputRef.current?.click();
    }
  }, []);

  const canShowPipeline = isRunning || isCompleted || isFailed;
  const canShowControls = !isRunning && !isCompleted && files.length > 0 && controls != null;
  const canShowFileSection = showFileSection && !isRunning && !isCompleted;
  const canShowProfilingSection = currentStage === "profiling" && profilingTotal > 0;

  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      <div className="flex items-center justify-between border-b px-6 py-4">
        <div>
          <div className="mb-1 flex items-center gap-3">
            <h1 className="text-lg font-semibold">{title}</h1>
            {headingAccessory}
          </div>
          <div className="text-sm text-muted-foreground">{subtitle}</div>
          {targetMeta ? (
            <div className="mt-1 text-xs text-muted-foreground">{targetMeta}</div>
          ) : null}
        </div>
        <div className="flex items-center gap-2">
          {!isRunning && !isCompleted && !isFailed ? (
            <button
              type="button"
              onClick={onStart}
              disabled={startDisabled}
              className="flex items-center gap-2 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-50"
            >
              <PlayIcon className="size-4" />
              {startLabel}
            </button>
          ) : null}
          {isCompleted || isFailed ? (
            <>
              <button
                type="button"
                onClick={onReset}
                className="flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-sm transition-colors hover:bg-muted"
              >
                <XIcon className="size-3.5" />
                {resetLabel}
              </button>
              {isFailed && onRetry ? (
                <button
                  type="button"
                  onClick={onRetry}
                  className="flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-sm transition-colors hover:bg-muted"
                >
                  <RotateCcwIcon className="size-3.5" />
                  {retryLabel}
                </button>
              ) : null}
            </>
          ) : null}
          {isRunning ? (
            <button
              type="button"
              onClick={onCancel}
              className="flex items-center gap-1.5 rounded-md border border-destructive/50 px-3 py-1.5 text-sm text-destructive transition-colors hover:bg-destructive/10"
            >
              <SquareIcon className="size-3.5" />
              {stopLabel}
            </button>
          ) : null}
          {isRunning ? (
            <span className="flex items-center gap-1.5 font-mono text-xs tabular-nums text-muted-foreground">
              <Loader2Icon className="size-3.5 animate-spin" />
              {formatElapsed(elapsedMs)}
            </span>
          ) : null}
        </div>
      </div>

      {reconnectingMessage ? (
        <div className="flex items-center gap-2 border-b bg-[var(--accent-amber)] px-6 py-2 text-xs text-[var(--dot-amber)]">
          <Loader2Icon className="size-3.5 animate-spin" />
          {reconnectingMessage}
        </div>
      ) : null}

      {canShowPipeline ? (
        <div className="sticky top-0 z-10 border-b bg-background px-6 py-3">
          <ConnectImportStageProgressBar
            stages={pipelineStages}
            currentStage={currentStage}
            runKey={runKey}
            profilingTotal={profilingTotal}
            profilingCompleted={profilingCompleted}
          />
        </div>
      ) : null}

      <div className="flex-1 overflow-y-auto">
        <div className="mx-auto max-w-5xl space-y-6 p-6">
          {error ? <ConnectErrorBanner message={error} onDismiss={onDismissError} /> : null}
          {isFailed && failureMessage ? (
            <ConnectErrorBanner
              message={failureMessage}
              onDismiss={onDismissFailure ?? onDismissError}
            />
          ) : null}
          {targetWarning ? (
            <ConnectErrorBanner
              message={targetWarning}
              onDismiss={onDismissTargetWarning ?? (() => {})}
            />
          ) : null}

          {canShowControls ? <div className="pb-2">{controls}</div> : null}

          {canShowFileSection ? (
            <ConnectSectionCard>
              <ConnectSectionHeader>{filesSectionTitle}</ConnectSectionHeader>

              <button
                type="button"
                onDragEnter={handleDragEnter}
                onDragOver={handleDragOver}
                onDragLeave={handleDragLeave}
                onDrop={handleDrop}
                onClick={() => fileInputRef.current?.click()}
                onKeyDown={handleDropZoneKeyDown}
                className={cx(
                  "w-full cursor-pointer rounded-lg border-2 border-dashed p-8 text-center transition-colors",
                  isDragOver
                    ? "border-primary bg-primary/5"
                    : "border-muted-foreground/20 hover:border-muted-foreground/40"
                )}
              >
                <UploadIcon className="mx-auto mb-3 size-8 text-muted-foreground/40" />
                <p className="text-sm text-muted-foreground">{fileUploadLabel}</p>
                {fileUploadDescription ? (
                  <p className="mt-1 text-xs text-muted-foreground/60">{fileUploadDescription}</p>
                ) : null}
              </button>
              <input
                ref={fileInputRef}
                type="file"
                multiple
                accept={fileInputAccept}
                className="hidden"
                onChange={(event) => {
                  if (event.target.files) {
                    onAddFiles(event.target.files);
                  }
                  event.target.value = "";
                }}
              />

              {files.length > 0 ? (
                <div className="divide-y">
                  {files.map((file) => (
                    <div
                      key={file.name}
                      className="flex items-center gap-3 py-2 first:pt-0 last:pb-0"
                    >
                      <FileUpIcon className="size-4 shrink-0 text-muted-foreground" />
                      <span className="flex-1 truncate font-mono text-xs">
                        {formatText(file.name)}
                      </span>
                      <span className="shrink-0 font-mono text-xs tabular-nums text-muted-foreground">
                        {(file.size / 1024 / 1024).toFixed(1)} MB
                      </span>
                      <button
                        type="button"
                        onClick={() => onRemoveFile(file.name)}
                        className="shrink-0 rounded p-0.5 transition-colors hover:bg-destructive/10 hover:text-destructive"
                        aria-label={`Remove ${formatText(file.name)}`}
                      >
                        <XIcon className="size-3.5" />
                      </button>
                    </div>
                  ))}
                </div>
              ) : null}
            </ConnectSectionCard>
          ) : null}

          {tableSection}
          {progressSection}

          {canShowProfilingSection ? (
            <ConnectSectionCard>
              <ConnectSectionHeader>Profiling</ConnectSectionHeader>
              <div className="space-y-2">
                <div className="h-2 w-full overflow-hidden rounded-full bg-muted">
                  <div
                    className="h-full rounded-full bg-primary transition-all duration-300"
                    style={{
                      width: `${Math.min(100, (profilingCompleted / profilingTotal) * 100)}%`,
                    }}
                  />
                </div>
                <p className="font-mono text-xs tabular-nums text-muted-foreground">
                  {profilingCompleted} / {profilingTotal} tables profiled
                </p>
              </div>
            </ConnectSectionCard>
          ) : null}

          {isCompleted && summaryMetrics && summaryMetrics.length > 0 ? (
            <ConnectImportSummarySection title={summaryTitle} metrics={summaryMetrics} />
          ) : null}

          {isCompleted && nextSteps ? (
            <ConnectSectionCard>
              <ConnectSectionHeader>Next Steps</ConnectSectionHeader>
              <div className="flex flex-wrap gap-2">{nextSteps}</div>
            </ConnectSectionCard>
          ) : null}

          {showEmptyState && emptyState ? (
            <ConnectEmptyState
              icon={emptyState.icon ?? UploadIcon}
              title={emptyState.title}
              description={emptyState.description}
              action={
                emptyState.actionLabel
                  ? {
                      label: emptyState.actionLabel,
                      onClick: () => fileInputRef.current?.click(),
                    }
                  : undefined
              }
            />
          ) : null}
        </div>
      </div>
    </div>
  );
}
