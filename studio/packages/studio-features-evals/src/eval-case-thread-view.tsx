import {
  ArrowLeftIcon,
  CheckCircleIcon,
  CheckIcon,
  ClipboardCheckIcon,
  ClipboardPlusIcon,
  ExternalLinkIcon,
  GitBranchIcon,
  ListTreeIcon,
  Loader2Icon,
  MessageSquareIcon,
  PencilIcon,
  RefreshCwIcon,
  Trash2Icon,
} from "lucide-react";
import { type ReactNode, useCallback, useEffect, useMemo, useState } from "react";
import { Link } from "react-router";
import {
  deriveEvalCaseContentView,
  type EvalCaseSampleLike,
  type EvalCaseThreadForkLike,
  getSelectedEvalCaseForkId,
  resolveEffectiveEvalCaseViewMode,
  type EvalCaseViewMode as ViewMode,
} from "./eval-case-thread-model";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

export interface SharedEvalCaseSampleLike extends EvalCaseSampleLike {
  readonly sampleIndex: number;
  readonly passed: boolean;
  readonly actualTrajectory: readonly string[];
}

export interface SharedEvalCaseResultLike<
  TSample extends SharedEvalCaseSampleLike = SharedEvalCaseSampleLike,
> {
  readonly aggregateScore: number;
  readonly samples: readonly TSample[];
}

export interface SharedEvalForkLike extends EvalCaseThreadForkLike {
  readonly parentThreadId?: string | null;
  readonly editedContent?: string | null;
  readonly label?: string | null;
}

export interface SharedEvalForkMessageArgs {
  readonly evalId: string;
  readonly testCaseId: string;
  readonly parentThreadId: string;
  readonly messageIndex: number;
  readonly content: string;
  readonly nextLabel: string;
}

export interface SharedEvalSaveForkTestCaseArgs<
  TFork extends SharedEvalForkLike = SharedEvalForkLike,
  TTrajectoryMode extends string = string,
> {
  readonly fork: TFork;
  readonly input: string;
  readonly sampleActualTrajectory: readonly string[] | null;
  readonly expectedTrajectory: readonly string[];
  readonly trajectoryMode: TTrajectoryMode;
  readonly evalId: string;
}

export interface SharedEvalCaseThreadViewProps<
  TSample extends SharedEvalCaseSampleLike = SharedEvalCaseSampleLike,
  TResult extends SharedEvalCaseResultLike<TSample> = SharedEvalCaseResultLike<TSample>,
  TFork extends SharedEvalForkLike = SharedEvalForkLike,
  TTrajectoryMode extends string = string,
> {
  readonly evalId: string;
  readonly testCaseId: string;
  readonly result: TResult;
  readonly input: string;
  readonly expectedTrajectory: readonly string[];
  readonly trajectoryMode: TTrajectoryMode;
  readonly selectedSampleIndex: number;
  readonly onSampleSelect: (index: number) => void;
  readonly onBack: () => void;
  readonly onRerun?: (testCaseId: string) => void;
  readonly forks?: readonly TFork[];
  readonly onForksChange?: (forks: readonly TFork[]) => void;
  readonly onDeleteFork?: (forkId: string) => void;
  readonly onRunChat?: () => void;
  readonly onUpdateExpected?: (actualTrajectory: readonly string[]) => void;
  readonly onCreateTestCase?: (actualTrajectory: readonly string[]) => void;
  readonly passThreshold?: number;
  readonly formatText?: (value: string) => string;
  readonly getSampleScore: (sample: TSample) => number;
  readonly editTestCaseHref: string;
  readonly refineInBuilderHref: string;
  readonly openThreadHref?: (threadId: string) => string;
  readonly openThreadTitle?: string;
  readonly renderSampleAccessory?: (sample: TSample) => ReactNode;
  readonly onCreateForkFromMessage?: (
    args: SharedEvalForkMessageArgs
  ) => Promise<TFork | null | undefined>;
  readonly onSaveForkAsTestCase?: (
    args: SharedEvalSaveForkTestCaseArgs<TFork, TTrajectoryMode>
  ) => Promise<string | null | undefined>;
  readonly buildSavedTestCaseHref?: (testCaseId: string) => string;
  readonly renderChatReplay: (args: {
    readonly threadId: string;
    readonly forkAtIndex?: number;
    readonly onForkMessage?: (messageIndex: number, content: string) => void;
  }) => ReactNode;
  readonly renderTrajectory: (sample: TSample) => ReactNode;
}

function editPreview(content: string | undefined | null, maxLength = 40): string {
  if (!content) {
    return "";
  }
  const trimmed = content.replace(/\s+/g, " ").trim();
  return trimmed.length > maxLength ? `${trimmed.slice(0, maxLength)}...` : trimmed;
}

function ViewModeToggle({
  viewMode,
  hasChatThread,
  onSetTrajectory,
  onSetChat,
}: {
  readonly viewMode: ViewMode;
  readonly hasChatThread: boolean;
  readonly onSetTrajectory: () => void;
  readonly onSetChat: () => void;
}) {
  return (
    <div className="inline-flex items-center rounded-lg border bg-muted/30 p-0.5">
      <button
        type="button"
        onClick={onSetTrajectory}
        className={cx(
          "flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium transition-colors",
          viewMode.kind === "trajectory"
            ? "bg-background text-foreground shadow-sm"
            : "text-muted-foreground hover:text-foreground"
        )}
      >
        <ListTreeIcon className="size-3.5" />
        Trajectory
        <kbd className="text-[9px] text-muted-foreground/50 bg-muted px-1 rounded hidden sm:inline">
          T
        </kbd>
      </button>
      <button
        type="button"
        onClick={onSetChat}
        disabled={!hasChatThread}
        className={cx(
          "flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium transition-colors",
          viewMode.kind === "sampleChat"
            ? "bg-background text-foreground shadow-sm"
            : "text-muted-foreground hover:text-foreground",
          !hasChatThread && "opacity-40 cursor-not-allowed"
        )}
      >
        <MessageSquareIcon className="size-3.5" />
        Chat
        <kbd className="text-[9px] text-muted-foreground/50 bg-muted px-1 rounded hidden sm:inline">
          C
        </kbd>
      </button>
    </div>
  );
}

function ForkBanner<TFork extends SharedEvalForkLike>({
  fork,
  onSaveAsTestCase,
  savedTestCaseId,
  isSaving,
  formatText,
  buildSavedTestCaseHref,
}: {
  readonly fork: TFork;
  readonly onSaveAsTestCase: () => void;
  readonly savedTestCaseId: string | null;
  readonly isSaving: boolean;
  readonly formatText: (value: string) => string;
  readonly buildSavedTestCaseHref: (testCaseId: string) => string;
}) {
  const preview = fork.editedContent
    ? fork.editedContent.replace(/\s+/g, " ").trim().slice(0, 80)
    : null;

  return (
    <div className="fork-banner rounded-lg px-4 py-2.5 flex items-start gap-3">
      <GitBranchIcon className="size-4 text-[var(--dot-amber)] shrink-0 mt-0.5" />
      <div className="flex-1 min-w-0">
        <div className="text-xs font-medium text-foreground">
          Fork of {fork.parentThreadId ? "conversation" : "sample"}
          {fork.forkAtMessageIndex != null ? (
            <span className="text-muted-foreground">
              {" "}
              at message #{fork.forkAtMessageIndex + 1}
            </span>
          ) : null}
        </div>
        {preview ? (
          <div className="text-[11px] text-muted-foreground mt-0.5 truncate">
            Edited: &ldquo;{formatText(preview)}&rdquo;
          </div>
        ) : null}
      </div>
      {!savedTestCaseId ? (
        <button
          type="button"
          onClick={onSaveAsTestCase}
          disabled={isSaving}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium bg-foreground text-background hover:bg-foreground/90 transition-colors disabled:opacity-50 shrink-0"
        >
          {isSaving ? (
            <Loader2Icon className="size-3 animate-spin" />
          ) : (
            <ClipboardCheckIcon className="size-3" />
          )}
          Save as Test Case
        </button>
      ) : (
        <Link
          to={buildSavedTestCaseHref(savedTestCaseId)}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium text-[var(--dot-emerald)] bg-[var(--accent-emerald)] hover:bg-[var(--dot-emerald)]/15 transition-colors shrink-0"
        >
          <CheckCircleIcon className="size-3" />
          Saved
          <ExternalLinkIcon className="size-3" />
        </Link>
      )}
    </div>
  );
}

function SampleTabBar<
  TSample extends SharedEvalCaseSampleLike = SharedEvalCaseSampleLike,
  TFork extends SharedEvalForkLike = SharedEvalForkLike,
>({
  samples,
  selectedIndex,
  onSelect,
  forks,
  selectedForkId,
  onSelectFork,
  onDeleteFork,
  onRunChat,
  onCreateTestCase,
  getSampleScore,
}: {
  readonly samples: readonly TSample[];
  readonly selectedIndex: number;
  readonly onSelect: (index: number) => void;
  readonly forks?: readonly TFork[];
  readonly selectedForkId?: string | null;
  readonly onSelectFork?: (forkId: string) => void;
  readonly onDeleteFork?: (forkId: string) => void;
  readonly onRunChat?: () => void;
  readonly onCreateTestCase?: () => void;
  readonly getSampleScore: (sample: TSample) => number;
}) {
  return (
    <div className="flex items-center gap-1 overflow-x-auto pb-1">
      {samples.map((sample) => {
        const score = getSampleScore(sample);
        const isActive = selectedForkId == null && selectedIndex === sample.sampleIndex;

        return (
          <button
            key={sample.sampleIndex}
            type="button"
            onClick={() => onSelect(sample.sampleIndex)}
            className={cx(
              "flex items-center gap-1.5 px-3 py-1.5 rounded-full text-xs font-medium transition-colors whitespace-nowrap",
              isActive
                ? "bg-primary text-primary-foreground"
                : "bg-muted/60 text-muted-foreground hover:bg-muted"
            )}
          >
            <span
              className="w-2 h-2 rounded-full shrink-0"
              style={{
                backgroundColor: sample.passed ? "var(--dot-emerald)" : "var(--dot-red)",
              }}
            />
            Sample {sample.sampleIndex + 1}
            <span className="tabular-nums">{Math.round(score * 100)}%</span>
          </button>
        );
      })}

      {forks && forks.length > 0 ? (
        <>
          <div className="w-px h-5 bg-border mx-1" />
          {forks.map((fork, index) => {
            const isActive = selectedForkId === fork.id;
            const preview = editPreview(fork.editedContent);
            return (
              <span key={fork.id} className="group/fork flex items-center">
                <button
                  type="button"
                  onClick={() => onSelectFork?.(fork.id)}
                  title={preview ? `Edited: "${preview}"` : fork.label || `Fork ${index + 1}`}
                  className={cx(
                    "flex items-center gap-1.5 px-3 py-1.5 rounded-full text-xs font-medium transition-colors whitespace-nowrap",
                    isActive
                      ? "bg-[var(--dot-amber)] text-white"
                      : "bg-[var(--accent-amber)] text-[var(--dot-amber)] hover:bg-[var(--dot-amber)]/20"
                  )}
                >
                  <GitBranchIcon className="size-3" />
                  {fork.label || `Fork ${index + 1}`}
                </button>
                {onDeleteFork ? (
                  <button
                    type="button"
                    onClick={(event) => {
                      event.stopPropagation();
                      onDeleteFork(fork.id);
                    }}
                    className="opacity-0 group-hover/fork:opacity-100 p-0.5 -ml-1 rounded hover:bg-destructive/10 hover:text-destructive transition-all shrink-0"
                    aria-label={`Delete fork ${fork.label || `Fork ${index + 1}`}`}
                  >
                    <Trash2Icon className="size-3" />
                  </button>
                ) : null}
              </span>
            );
          })}
        </>
      ) : null}

      {onRunChat || onCreateTestCase ? (
        <>
          <div className="w-px h-5 bg-border mx-1" />
          {onRunChat ? (
            <button
              type="button"
              onClick={onRunChat}
              className="flex items-center gap-1 px-3 py-1.5 rounded-full text-xs font-medium text-muted-foreground hover:bg-muted transition-colors whitespace-nowrap border border-dashed border-muted-foreground/30"
            >
              + Run Chat
            </button>
          ) : null}
          {onCreateTestCase ? (
            <button
              type="button"
              onClick={onCreateTestCase}
              className="flex items-center gap-1 px-3 py-1.5 rounded-full text-xs font-medium text-muted-foreground hover:bg-muted transition-colors whitespace-nowrap border border-dashed border-muted-foreground/30"
            >
              + Create Test
            </button>
          ) : null}
        </>
      ) : null}
    </div>
  );
}

export function SharedEvalCaseThreadView<
  TSample extends SharedEvalCaseSampleLike = SharedEvalCaseSampleLike,
  TResult extends SharedEvalCaseResultLike<TSample> = SharedEvalCaseResultLike<TSample>,
  TFork extends SharedEvalForkLike = SharedEvalForkLike,
  TTrajectoryMode extends string = string,
>({
  evalId,
  testCaseId,
  result,
  input,
  expectedTrajectory,
  trajectoryMode,
  selectedSampleIndex,
  onSampleSelect,
  onBack,
  onRerun,
  forks = [],
  onForksChange,
  onDeleteFork,
  onRunChat,
  onUpdateExpected,
  onCreateTestCase,
  passThreshold = 0.7,
  formatText = (value) => value,
  getSampleScore,
  editTestCaseHref,
  refineInBuilderHref,
  openThreadHref,
  openThreadTitle = "Open thread",
  renderSampleAccessory,
  onCreateForkFromMessage,
  onSaveForkAsTestCase,
  buildSavedTestCaseHref = (testCaseIdValue) => `/tests/${testCaseIdValue}`,
  renderChatReplay,
  renderTrajectory,
}: SharedEvalCaseThreadViewProps<TSample, TResult, TFork, TTrajectoryMode>) {
  const [viewMode, setViewMode] = useState<ViewMode>({ kind: "trajectory" });
  const [isRerunning, setIsRerunning] = useState(false);
  const [savingForkId, setSavingForkId] = useState<string | null>(null);
  const [savedTestCases, setSavedTestCases] = useState<Map<string, string>>(new Map());

  const forkIdSet = useMemo(() => new Set(forks.map((fork) => fork.id)), [forks]);
  const effectiveViewMode = resolveEffectiveEvalCaseViewMode(viewMode, forkIdSet);

  const sample = result.samples.find((entry) => entry.sampleIndex === selectedSampleIndex);

  const contentView = useMemo(
    () => deriveEvalCaseContentView(effectiveViewMode, sample, forks),
    [effectiveViewMode, sample, forks]
  );

  const currentFork = useMemo(
    () =>
      effectiveViewMode.kind === "fork"
        ? forks.find((fork) => fork.id === effectiveViewMode.forkId)
        : undefined,
    [effectiveViewMode, forks]
  );

  const handleSampleSelect = useCallback(
    (index: number) => {
      if (effectiveViewMode.kind === "fork") {
        setViewMode({ kind: "trajectory" });
      }
      onSampleSelect(index);
    },
    [effectiveViewMode.kind, onSampleSelect]
  );

  const handleSetTrajectory = useCallback(() => {
    setViewMode({ kind: "trajectory" });
  }, []);

  const handleSetChat = useCallback(() => {
    if (sample?.threadId) {
      setViewMode({ kind: "sampleChat" });
    }
  }, [sample?.threadId]);

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement;
      if (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable) {
        return;
      }

      if ((event.key === "c" || event.key === "C") && sample?.threadId) {
        setViewMode({ kind: "sampleChat" });
      } else if (event.key === "t" || event.key === "T") {
        setViewMode({ kind: "trajectory" });
      }
    };

    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [sample?.threadId]);

  const handleSelectFork = useCallback((forkId: string) => {
    setViewMode({ kind: "fork", forkId });
  }, []);

  const handleForkMessage = useCallback(
    async (messageIndex: number, content: string) => {
      if (!onCreateForkFromMessage) {
        return;
      }
      const parentThreadId =
        effectiveViewMode.kind === "fork"
          ? forks.find((fork) => fork.id === effectiveViewMode.forkId)?.threadId
          : sample?.threadId;
      if (!parentThreadId) {
        return;
      }

      const nextFork = await onCreateForkFromMessage({
        evalId,
        testCaseId,
        parentThreadId,
        messageIndex,
        content,
        nextLabel: `Fork ${forks.length + 1}`,
      });

      if (nextFork) {
        onForksChange?.([...forks, nextFork]);
        setViewMode({ kind: "fork", forkId: nextFork.id });
      }
    },
    [
      effectiveViewMode,
      evalId,
      forks,
      onCreateForkFromMessage,
      onForksChange,
      sample?.threadId,
      testCaseId,
    ]
  );

  const handleSaveAsTestCase = useCallback(async () => {
    if (!currentFork || !onSaveForkAsTestCase) {
      return;
    }

    setSavingForkId(currentFork.id);
    try {
      const savedTestCaseId = await onSaveForkAsTestCase({
        fork: currentFork,
        input,
        sampleActualTrajectory: sample?.actualTrajectory ?? null,
        expectedTrajectory,
        trajectoryMode,
        evalId,
      });

      if (savedTestCaseId) {
        setSavedTestCases((previous) => new Map(previous).set(currentFork.id, savedTestCaseId));
      }
    } finally {
      setSavingForkId(null);
    }
  }, [
    currentFork,
    evalId,
    expectedTrajectory,
    input,
    onSaveForkAsTestCase,
    sample?.actualTrajectory,
    trajectoryMode,
  ]);

  return (
    <div className="max-w-3xl mx-auto p-8 space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={onBack}
            className="text-sm text-muted-foreground hover:text-foreground flex items-center gap-1"
          >
            <ArrowLeftIcon className="size-4" />
            Back to Eval Run
            <kbd className="text-[9px] text-muted-foreground/40 bg-muted px-1 rounded hidden sm:inline ml-1">
              Esc
            </kbd>
          </button>
          <span className="hidden sm:inline-flex items-center gap-1 ml-2 text-[9px] text-muted-foreground/40">
            <kbd className="bg-muted px-1 rounded">J</kbd>/
            <kbd className="bg-muted px-1 rounded">K</kbd> prev/next
          </span>
        </div>
        <div className="flex items-center gap-3">
          <Link
            to={editTestCaseHref}
            className="text-xs font-mono text-muted-foreground hover:text-primary hover:underline inline-flex items-center gap-1"
            title="Edit in Tests tab"
          >
            {formatText(testCaseId)}
            <ExternalLinkIcon className="size-3" />
          </Link>
          <span
            className={cx(
              "text-xs font-medium px-2.5 py-0.5 rounded-md tabular-nums",
              result.aggregateScore >= passThreshold
                ? "bg-[var(--accent-emerald)] text-[var(--dot-emerald)]"
                : "bg-[var(--accent-red)] text-[var(--dot-red)]"
            )}
          >
            {result.aggregateScore >= passThreshold ? "PASS" : "FAIL"}{" "}
            {Math.round(result.aggregateScore * 100)}%
          </span>
        </div>
      </div>

      <div className="text-sm text-foreground bg-muted/30 rounded-lg px-4 py-3 whitespace-pre-wrap break-words line-clamp-3 border border-border/40">
        {formatText(input)}
      </div>

      <div className="flex items-center justify-between gap-3 flex-wrap">
        <SampleTabBar
          samples={result.samples}
          selectedIndex={selectedSampleIndex}
          onSelect={handleSampleSelect}
          forks={forks}
          selectedForkId={getSelectedEvalCaseForkId(effectiveViewMode)}
          onSelectFork={handleSelectFork}
          onDeleteFork={onDeleteFork}
          onRunChat={onRunChat}
          onCreateTestCase={
            onCreateTestCase && sample ? () => onCreateTestCase(sample.actualTrajectory) : undefined
          }
          getSampleScore={getSampleScore}
        />

        <div className="flex items-center gap-2">
          {effectiveViewMode.kind !== "fork" ? (
            <ViewModeToggle
              viewMode={effectiveViewMode}
              hasChatThread={!!sample?.threadId}
              onSetTrajectory={handleSetTrajectory}
              onSetChat={handleSetChat}
            />
          ) : null}
          {sample?.threadId && openThreadHref ? (
            <Link
              to={openThreadHref(sample.threadId)}
              className="flex items-center gap-1 px-2 py-1.5 rounded-md text-xs text-muted-foreground hover:text-foreground hover:bg-muted transition-colors"
              title={openThreadTitle}
            >
              <ExternalLinkIcon className="size-3" />
              <span className="hidden sm:inline">Open Thread</span>
            </Link>
          ) : null}
          {sample && renderSampleAccessory ? renderSampleAccessory(sample) : null}
        </div>
      </div>

      {contentView.view === "chat" ? (
        <>
          {effectiveViewMode.kind === "fork" && currentFork ? (
            <ForkBanner
              fork={currentFork}
              onSaveAsTestCase={handleSaveAsTestCase}
              savedTestCaseId={savedTestCases.get(currentFork.id) ?? null}
              isSaving={savingForkId === currentFork.id}
              formatText={formatText}
              buildSavedTestCaseHref={buildSavedTestCaseHref}
            />
          ) : null}

          {effectiveViewMode.kind === "fork" ? (
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={handleSetTrajectory}
                className="text-xs text-muted-foreground hover:text-foreground flex items-center gap-1.5 px-2.5 py-1 rounded-md border hover:bg-muted transition-colors"
              >
                <ListTreeIcon className="size-3" />
                Back to Trajectory
                <kbd className="text-[9px] text-muted-foreground/50 bg-muted px-1 rounded">T</kbd>
              </button>
            </div>
          ) : null}

          {renderChatReplay({
            threadId: contentView.threadId,
            forkAtIndex: contentView.forkAtIndex,
            onForkMessage: onCreateForkFromMessage ? handleForkMessage : undefined,
          })}
        </>
      ) : contentView.view === "trajectory" ? (
        <>
          <div className="flex justify-end gap-2">
            {onCreateTestCase && !contentView.sample.passed ? (
              <button
                type="button"
                onClick={() => onCreateTestCase(contentView.sample.actualTrajectory)}
                className="text-xs px-3 py-1.5 rounded-md border border-[var(--dot-amber)]/30 text-[var(--dot-amber)] hover:bg-[var(--accent-amber)] flex items-center gap-1.5"
              >
                <ClipboardPlusIcon className="size-3.5" />
                Create Test Case
              </button>
            ) : null}
            {onUpdateExpected && contentView.sample.passed ? (
              <button
                type="button"
                onClick={() => onUpdateExpected(contentView.sample.actualTrajectory)}
                className="text-xs px-3 py-1.5 rounded-md border border-[var(--dot-emerald)]/30 text-[var(--dot-emerald)] hover:bg-[var(--accent-emerald)] flex items-center gap-1.5"
              >
                <CheckIcon className="size-3.5" />
                Update Expected
              </button>
            ) : null}
          </div>
          {renderTrajectory(contentView.sample)}
        </>
      ) : (
        <div className="text-sm text-muted-foreground text-center py-8">
          No data for sample {selectedSampleIndex + 1}
        </div>
      )}

      <div className="flex items-center gap-2">
        <Link
          to={editTestCaseHref}
          className="flex-1 flex items-center justify-center gap-1.5 text-sm font-medium px-3 py-2 rounded-md border hover:bg-muted transition-colors"
        >
          <PencilIcon className="size-3.5" />
          Edit Test Case
        </Link>
        <Link
          to={refineInBuilderHref}
          className="flex-1 flex items-center justify-center gap-1.5 text-sm font-medium px-3 py-2 rounded-md border hover:bg-muted transition-colors"
          title="Create a refined test case via the Builder agent"
        >
          <MessageSquareIcon className="size-3.5" />
          Refine in Builder
        </Link>
        {onRerun ? (
          <button
            type="button"
            disabled={isRerunning}
            onClick={() => {
              setIsRerunning(true);
              onRerun(testCaseId);
            }}
            className="flex-1 flex items-center justify-center gap-1.5 text-sm font-medium px-3 py-2 rounded-md border hover:bg-muted transition-colors disabled:opacity-50"
          >
            {isRerunning ? (
              <Loader2Icon className="size-3.5 animate-spin" />
            ) : (
              <RefreshCwIcon className="size-3.5" />
            )}
            {isRerunning ? "Re-running..." : "Re-run this case"}
          </button>
        ) : null}
      </div>
    </div>
  );
}
