/**
 * Eval case thread view — main container for drill-down into a test case.
 *
 * View state is modeled as a discriminated union (`ViewMode`), eliminating
 * the invalid states that arise from independent boolean + nullable encoding.
 * Content derivation is a pure function (`deriveContentView`) separated from
 * rendering — testable and exhaustive.
 *
 * View mode toggle: visible pill buttons (Trajectory | Chat) with keyboard
 * shortcuts (T / C) shown as hints.
 *
 * @module components/eval/eval-case-thread-view
 */

import {
  ArrowLeftIcon,
  CheckIcon,
  ClipboardPlusIcon,
  ExternalLinkIcon,
  ListTreeIcon,
  Loader2Icon,
  MessageSquareIcon,
  NetworkIcon,
  PencilIcon,
  RefreshCwIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Link } from "react-router";
import type { SampleResult, TestCaseResult, TrajectoryMode } from "~/lib/domain/eval";
import { synthesizeTrajectorySteps } from "~/lib/domain/eval";
import { useScramble } from "~/lib/scramble";
import { assertNever, cn } from "~/lib/utils";
import { ChatReplayView } from "./chat-replay-view";
import { SampleTabBar } from "./sample-tab-bar";
import { TrajectoryThread } from "./trajectory-thread";

// =============================================================================
// View Mode (Discriminated Union)
// =============================================================================

type ViewMode = { readonly kind: "trajectory" } | { readonly kind: "sampleChat" };

// =============================================================================
// Content View (Pure Derivation)
// =============================================================================

type ContentView =
  | { readonly view: "trajectory"; readonly sample: SampleResult }
  | { readonly view: "chat"; readonly threadId: string }
  | { readonly view: "empty" };

function deriveContentView(viewMode: ViewMode, sample: SampleResult | undefined): ContentView {
  switch (viewMode.kind) {
    case "trajectory":
      return sample ? { view: "trajectory", sample } : { view: "empty" };
    case "sampleChat":
      if (sample?.threadId) return { view: "chat", threadId: sample.threadId };
      return sample ? { view: "trajectory", sample } : { view: "empty" };
    default:
      return assertNever(viewMode);
  }
}

// =============================================================================
// View Mode Toggle
// =============================================================================

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
        className={cn(
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
        className={cn(
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

// =============================================================================
// Component
// =============================================================================

interface EvalCaseThreadViewProps {
  readonly evalId: string;
  readonly testCaseId: string;
  readonly result: TestCaseResult;
  readonly input: string;
  readonly expectedTrajectory: readonly string[];
  readonly trajectoryMode: TrajectoryMode;
  readonly selectedSampleIndex: number;
  readonly onSampleSelect: (index: number) => void;
  readonly onBack: () => void;
  readonly onRerun?: (testCaseId: string) => void;
  readonly onUpdateExpected?: (actualTrajectory: readonly string[]) => void;
  readonly onCreateTestCase?: (actualTrajectory: readonly string[]) => void;
  readonly passThreshold?: number;
}

export function EvalCaseThreadView({
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
  onUpdateExpected,
  onCreateTestCase,
  passThreshold = 0.7,
}: EvalCaseThreadViewProps) {
  const { s } = useScramble();
  const [viewMode, setViewMode] = useState<ViewMode>({ kind: "trajectory" });
  const [isRerunning, setIsRerunning] = useState(false);

  const sample = result.samples.find((sm) => sm.sampleIndex === selectedSampleIndex);

  const steps = useMemo(
    () =>
      sample
        ? synthesizeTrajectorySteps(sample.actualTrajectory, expectedTrajectory, trajectoryMode)
        : [],
    [sample, expectedTrajectory, trajectoryMode]
  );

  const contentView = useMemo(() => deriveContentView(viewMode, sample), [viewMode, sample]);

  const handleSampleSelect = useCallback(
    (index: number) => {
      setViewMode({ kind: "trajectory" });
      onSampleSelect(index);
    },
    [onSampleSelect]
  );

  const handleSetTrajectory = useCallback(() => {
    setViewMode({ kind: "trajectory" });
  }, []);

  const handleSetChat = useCallback(() => {
    if (sample?.threadId) {
      setViewMode({ kind: "sampleChat" });
    }
  }, [sample?.threadId]);

  // Keyboard shortcut: C toggles to sampleChat, T toggles to trajectory
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      if (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)
        return;

      if ((e.key === "c" || e.key === "C") && sample?.threadId) {
        setViewMode({ kind: "sampleChat" });
      } else if (e.key === "t" || e.key === "T") {
        setViewMode({ kind: "trajectory" });
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [sample?.threadId]);

  return (
    <div className="max-w-3xl mx-auto p-8 space-y-6">
      {/* Breadcrumb header */}
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
            to={`/tests/${testCaseId}`}
            className="text-xs font-mono text-muted-foreground hover:text-primary hover:underline inline-flex items-center gap-1"
            title="Edit in Tests tab"
          >
            {s(testCaseId)}
            <ExternalLinkIcon className="size-3" />
          </Link>
          <span
            className={cn(
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

      {/* User query */}
      <div className="text-sm text-foreground bg-muted/30 rounded-lg px-4 py-3 whitespace-pre-wrap break-words line-clamp-3 border border-border/40">
        {s(input)}
      </div>

      {/* Controls: sample tabs + view toggle */}
      <div className="flex items-center justify-between gap-3 flex-wrap">
        <SampleTabBar
          samples={result.samples}
          selectedIndex={selectedSampleIndex}
          onSelect={handleSampleSelect}
          onCreateTestCase={
            onCreateTestCase && sample ? () => onCreateTestCase(sample.actualTrajectory) : undefined
          }
        />

        <div className="flex items-center gap-2">
          <ViewModeToggle
            viewMode={viewMode}
            hasChatThread={!!sample?.threadId}
            onSetTrajectory={handleSetTrajectory}
            onSetChat={handleSetChat}
          />
          {sample?.trace?.traceId && (
            <Link
              to={`/evals/${evalId}/cases/${testCaseId}/traces/${sample.trace.traceId}?sample=${selectedSampleIndex}`}
              className="flex items-center gap-1 px-2 py-1.5 rounded-md text-xs text-muted-foreground hover:text-foreground hover:bg-muted transition-colors"
              title="Open persisted trace"
            >
              <NetworkIcon className="size-3" />
              <span className="hidden sm:inline">Open Trace</span>
            </Link>
          )}
        </div>
      </div>

      {/* Content — exhaustive on ContentView */}
      {contentView.view === "chat" ? (
        <ChatReplayView threadId={contentView.threadId} />
      ) : contentView.view === "trajectory" ? (
        <>
          {/* Action buttons above trajectory */}
          <div className="flex justify-end gap-2">
            {onCreateTestCase && !contentView.sample.passed && (
              <button
                type="button"
                onClick={() => onCreateTestCase(contentView.sample.actualTrajectory)}
                className="text-xs px-3 py-1.5 rounded-md border border-[var(--dot-amber)]/30 text-[var(--dot-amber)] hover:bg-[var(--accent-amber)] flex items-center gap-1.5"
              >
                <ClipboardPlusIcon className="size-3.5" />
                Create Test Case
              </button>
            )}
            {onUpdateExpected && contentView.sample.passed && (
              <button
                type="button"
                onClick={() => onUpdateExpected(contentView.sample.actualTrajectory)}
                className="text-xs px-3 py-1.5 rounded-md border border-[var(--dot-emerald)]/30 text-[var(--dot-emerald)] hover:bg-[var(--accent-emerald)] flex items-center gap-1.5"
              >
                <CheckIcon className="size-3.5" />
                Update Expected
              </button>
            )}
          </div>
          <TrajectoryThread
            input={input}
            steps={steps}
            sample={contentView.sample}
            passThreshold={passThreshold}
          />
        </>
      ) : (
        <div className="text-sm text-muted-foreground text-center py-8">
          No data for sample {selectedSampleIndex + 1}
        </div>
      )}

      {/* Action bar — edit test case, refine in builder, re-run */}
      <div className="flex items-center gap-2">
        <Link
          to={`/tests/${testCaseId}`}
          className="flex-1 flex items-center justify-center gap-1.5 text-sm font-medium px-3 py-2 rounded-md border hover:bg-muted transition-colors"
        >
          <PencilIcon className="size-3.5" />
          Edit Test Case
        </Link>
        <Link
          to={`/tests/new?prefill=${encodeURIComponent(input)}`}
          className="flex-1 flex items-center justify-center gap-1.5 text-sm font-medium px-3 py-2 rounded-md border hover:bg-muted transition-colors"
          title="Edit this prompt into a new test case"
        >
          <MessageSquareIcon className="size-3.5" />
          Edit
        </Link>
        {onRerun && (
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
        )}
      </div>
    </div>
  );
}
