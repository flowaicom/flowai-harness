import {
  CheckCircle2Icon,
  CircleDotIcon,
  CircleIcon,
  ClipboardListIcon,
  ListOrderedIcon,
  RotateCcwIcon,
  SaveIcon,
  TargetIcon,
} from "lucide-react";
import { type ReactNode, type Ref, useMemo } from "react";
import type { TestBuilderSessionLike } from "./domain";
import {
  type BuilderWorkflowStep,
  isBuilderWorkflowStepComplete,
  summarizeBuilderGroundTruth,
  TEST_BUILDER_WORKFLOW_STEPS,
} from "./models";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

const CATEGORY_COLORS: Record<
  string,
  { readonly bg: string; readonly text: string; readonly border: string }
> = {
  discovery: {
    bg: "bg-[var(--accent-blue)]",
    text: "text-[var(--dot-blue)]",
    border: "border-[var(--dot-blue)]/25",
  },
  planning: {
    bg: "bg-[var(--accent-purple)]",
    text: "text-[var(--dot-purple)]",
    border: "border-[var(--dot-purple)]/25",
  },
  execution: {
    bg: "bg-[var(--accent-amber)]",
    text: "text-[var(--dot-amber)]",
    border: "border-[var(--dot-amber)]/25",
  },
  knowledge: {
    bg: "bg-[var(--accent-emerald)]",
    text: "text-[var(--dot-emerald)]",
    border: "border-[var(--dot-emerald)]/25",
  },
  delegation: {
    bg: "bg-primary/8",
    text: "text-primary",
    border: "border-primary/20",
  },
};

const FALLBACK_CATEGORY_COLOR = {
  bg: "bg-primary/8",
  text: "text-primary",
  border: "border-primary/20",
};

function categoryColor(category: string | undefined) {
  return (category && CATEGORY_COLORS[category]) || FALLBACK_CATEGORY_COLOR;
}

function getTrajectoryToolName(step: unknown): string {
  if (typeof step === "string" && step.trim().length > 0) return step;
  if (
    step &&
    typeof step === "object" &&
    "toolName" in step &&
    typeof step.toolName === "string" &&
    step.toolName.trim().length > 0
  ) {
    return step.toolName;
  }
  return "unknown";
}

export interface SharedBuilderToolLike {
  readonly name: string;
  readonly category: string;
}

function WorkflowStepper({ currentStep }: { readonly currentStep: BuilderWorkflowStep }) {
  return (
    <div className="flex items-center gap-1">
      {TEST_BUILDER_WORKFLOW_STEPS.map((step, index) => {
        const isActive = step.key === currentStep;
        const isComplete = isBuilderWorkflowStepComplete(step.key, currentStep);

        return (
          <div key={step.key} className="flex items-center gap-1">
            {index > 0 ? (
              <div
                className={cx("h-px w-4", isComplete ? "bg-[var(--dot-emerald)]" : "bg-border")}
              />
            ) : null}
            <div
              className={cx(
                "flex items-center gap-1.5 rounded-md px-2 py-1 text-[10px] font-medium transition-colors",
                isComplete && "text-[var(--dot-emerald)]",
                isActive && "bg-primary/10 text-primary",
                !isComplete && !isActive && "text-muted-foreground/50"
              )}
              title={step.description}
            >
              {isComplete ? (
                <CheckCircle2Icon className="size-3 text-[var(--dot-emerald)]" />
              ) : isActive ? (
                <CircleDotIcon className="size-3" />
              ) : (
                <CircleIcon className="size-3" />
              )}
              <span className="hidden sm:inline">{step.label}</span>
            </div>
          </div>
        );
      })}
    </div>
  );
}

function SessionStatePanel<TSession extends TestBuilderSessionLike>({
  session,
  currentStep,
  availableTools,
  panelBodyClassName,
}: {
  readonly session: TSession | null;
  readonly currentStep: BuilderWorkflowStep;
  readonly availableTools: readonly SharedBuilderToolLike[];
  readonly panelBodyClassName?: string;
}) {
  const toolCategoryMap = useMemo(() => {
    const map = new Map<string, string>();
    for (const tool of availableTools) map.set(tool.name, tool.category);
    return map;
  }, [availableTools]);

  const trajectory = session?.composedTrajectory ?? [];
  const gt = session?.structuredGroundTruth ?? null;
  const legacyGt = session?.groundTruth;
  const mode = session?.trajectoryMode ?? "unordered";

  return (
    <div className="flex min-h-0 w-72 shrink-0 flex-col border-l">
      <div className="border-b px-4 py-3">
        <div className="section-label">Session State</div>
        <WorkflowStepper currentStep={currentStep} />
      </div>

      <div className={cx("flex-1 overflow-y-auto p-4 space-y-4", panelBodyClassName)}>
        <div className="space-y-2">
          <div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
            <ListOrderedIcon className="size-3" />
            Trajectory
            {trajectory.length > 0 ? (
              <span className="ml-auto text-[10px] font-mono tabular-nums">
                {trajectory.length} step{trajectory.length !== 1 ? "s" : ""}
              </span>
            ) : null}
          </div>

          {trajectory.length === 0 ? (
            <div className="pl-0.5 text-[10px] text-muted-foreground/50">Not yet composed</div>
          ) : (
            <div className="space-y-1">
              {trajectory.map((step, index) => {
                const toolName = getTrajectoryToolName(step);
                const color = categoryColor(toolCategoryMap.get(toolName));
                return (
                  <div
                    key={`${index}-${toolName}`}
                    className={cx(
                      "flex items-center gap-1.5 rounded border px-2 py-1 text-[11px] font-mono",
                      color.bg,
                      color.text,
                      color.border
                    )}
                  >
                    <span className="w-3 shrink-0 text-center text-[9px] tabular-nums opacity-50">
                      {index + 1}
                    </span>
                    <span className="truncate">{toolName}</span>
                  </div>
                );
              })}
              <div className="pl-0.5 text-[10px] text-muted-foreground/50">Mode: {mode}</div>
            </div>
          )}
        </div>

        <div className="space-y-2">
          <div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
            <TargetIcon className="size-3" />
            Ground Truth
          </div>

          {gt ? (
            <div className="rounded-md border border-[var(--dot-emerald)]/20 bg-[var(--accent-emerald)] px-2.5 py-2 text-[11px] text-[var(--dot-emerald)]">
              <div className="font-medium capitalize">{gt.kind}</div>
              <div className="mt-0.5 text-[var(--dot-emerald)]/70">
                {summarizeBuilderGroundTruth(gt)}
              </div>
            </div>
          ) : legacyGt ? (
            <div className="rounded-md border bg-muted/30 px-2.5 py-2 text-[11px] text-muted-foreground">
              <div className="font-medium">Text</div>
              <div className="mt-0.5 truncate">{legacyGt}</div>
            </div>
          ) : (
            <div className="pl-0.5 text-[10px] text-muted-foreground/50">Not yet defined</div>
          )}
        </div>

        {currentStep === "review" ? (
          <div className="rounded-md border border-[var(--dot-emerald)]/30 bg-[var(--accent-emerald)] px-3 py-2.5 text-[11px]">
            <div className="flex items-center gap-1.5 font-medium text-[var(--dot-emerald)]">
              <CheckCircle2Icon className="size-3.5" />
              Ready to save
            </div>
            <div className="mt-1 text-[var(--dot-emerald)]/60">
              Trajectory and ground truth are set. Click "Save as Test Case" to persist.
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function SaveBar<TSession extends TestBuilderSessionLike>({
  session,
  saving,
  saveError,
  saveHintWhenNoGroundTruth,
  onSave,
}: {
  readonly session: TSession | null;
  readonly saving: boolean;
  readonly saveError?: string | null;
  readonly saveHintWhenNoGroundTruth?: string;
  readonly onSave: () => void;
}) {
  const trajectory = session?.composedTrajectory ?? [];
  const gt = session?.structuredGroundTruth ?? null;
  const legacyGt = session?.groundTruth;

  return (
    <div className="space-y-2 border-t px-4 py-3">
      <div className="flex items-center gap-3 text-[10px] text-muted-foreground">
        <span className="flex items-center gap-1">
          <ClipboardListIcon className="size-3" />
          {trajectory.length} trajectory step{trajectory.length !== 1 ? "s" : ""}
        </span>
        <span className="text-muted-foreground/30">|</span>
        <span className="flex items-center gap-1">
          <TargetIcon className="size-3" />
          {gt ? summarizeBuilderGroundTruth(gt) : legacyGt ? "Text GT" : "No ground truth"}
        </span>
      </div>

      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={onSave}
          disabled={saving}
          className="flex items-center gap-2 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <SaveIcon className="size-4" />
          {saving ? "Saving..." : "Save as Test Case"}
        </button>
        {!gt && !legacyGt && saveHintWhenNoGroundTruth ? (
          <span className="text-[10px] leading-relaxed text-[var(--dot-amber)]">
            {saveHintWhenNoGroundTruth}
          </span>
        ) : null}
        {saveError ? <span className="text-sm text-destructive">{saveError}</span> : null}
      </div>
    </div>
  );
}

export interface SharedTestCaseBuilderShellProps<
  TSession extends TestBuilderSessionLike = TestBuilderSessionLike,
> {
  readonly sessionId?: string | null;
  readonly session: TSession | null;
  readonly hasMessages: boolean;
  readonly isStreaming: boolean;
  readonly currentStep: BuilderWorkflowStep;
  readonly availableTools: readonly SharedBuilderToolLike[];
  readonly examples: readonly string[];
  readonly introTitle?: string;
  readonly introDescription: string;
  readonly saveHintWhenNoGroundTruth?: string;
  readonly saving: boolean;
  readonly saveError?: string | null;
  readonly onExampleClick: (example: string) => void;
  readonly onSave: () => void;
  readonly onReset: () => void;
  readonly showResetButton: boolean;
  readonly resetButtonTitle?: string;
  readonly messageList: ReactNode;
  readonly streamingMessage?: ReactNode;
  readonly errorBanner?: ReactNode;
  readonly inputArea: ReactNode;
  readonly messageAreaRef?: Ref<HTMLDivElement>;
  readonly messageAreaClassName?: string;
  readonly sessionPanelBodyClassName?: string;
}

export function SharedTestCaseBuilderShell<
  TSession extends TestBuilderSessionLike = TestBuilderSessionLike,
>({
  sessionId,
  session,
  hasMessages,
  isStreaming,
  currentStep,
  availableTools,
  examples,
  introTitle = "Build a Test Case",
  introDescription,
  saveHintWhenNoGroundTruth,
  saving,
  saveError,
  onExampleClick,
  onSave,
  onReset,
  showResetButton,
  resetButtonTitle = "Reset session",
  messageList,
  streamingMessage,
  errorBanner,
  inputArea,
  messageAreaRef,
  messageAreaClassName,
  sessionPanelBodyClassName,
}: SharedTestCaseBuilderShellProps<TSession>) {
  return (
    <div className="flex flex-1 min-h-0">
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <div
          ref={messageAreaRef}
          className={cx("flex-1 overflow-y-auto p-4 space-y-4", messageAreaClassName)}
        >
          {!hasMessages && !isStreaming ? (
            <div className="mx-auto mt-12 max-w-md text-center">
              <div className="mb-4 inline-flex h-12 w-12 items-center justify-center rounded-full bg-primary/10">
                <ClipboardListIcon className="size-6 text-primary" />
              </div>
              <h2 className="mb-1 text-sm font-semibold">{introTitle}</h2>
              <p className="text-xs leading-relaxed text-muted-foreground">{introDescription}</p>
              <div className="mt-4 flex flex-wrap justify-center gap-2">
                {examples.map((example) => (
                  <button
                    key={example}
                    type="button"
                    onClick={() => onExampleClick(example)}
                    className="rounded-full border border-dashed border-muted-foreground/30 px-3 py-1.5 text-[11px] text-muted-foreground transition-colors hover:border-primary/50 hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    {example}
                  </button>
                ))}
              </div>
            </div>
          ) : null}

          {messageList}
          {streamingMessage}
          {errorBanner}
        </div>

        {!isStreaming && hasMessages && sessionId ? (
          <SaveBar
            session={session}
            saving={saving}
            saveError={saveError}
            saveHintWhenNoGroundTruth={saveHintWhenNoGroundTruth}
            onSave={onSave}
          />
        ) : null}

        <div className="border-t p-4">
          <div className="flex items-end gap-2">
            <div className="flex-1">{inputArea}</div>
            {showResetButton ? (
              <button
                type="button"
                onClick={onReset}
                className="rounded-md border p-2 text-muted-foreground transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                title={resetButtonTitle}
              >
                <RotateCcwIcon className="size-4" />
              </button>
            ) : null}
          </div>
        </div>
      </div>

      <SessionStatePanel
        session={session}
        currentStep={currentStep}
        availableTools={availableTools}
        panelBodyClassName={sessionPanelBodyClassName}
      />
    </div>
  );
}
