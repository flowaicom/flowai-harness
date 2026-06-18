export type EvalCaseViewMode =
  | { readonly kind: "trajectory" }
  | { readonly kind: "sampleChat" }
  | { readonly kind: "fork"; readonly forkId: string };

export interface EvalCaseThreadForkLike {
  readonly id: string;
  readonly threadId: string;
  readonly forkAtMessageIndex?: number | null;
}

export interface EvalCaseSampleLike {
  readonly threadId?: string | null;
}

export type EvalCaseContentView<TSample extends EvalCaseSampleLike = EvalCaseSampleLike> =
  | { readonly view: "trajectory"; readonly sample: TSample }
  | { readonly view: "chat"; readonly threadId: string; readonly forkAtIndex?: number }
  | { readonly view: "empty" };

export function resolveEffectiveEvalCaseViewMode(
  viewMode: EvalCaseViewMode,
  forkIds: ReadonlySet<string>
): EvalCaseViewMode {
  if (viewMode.kind === "fork" && !forkIds.has(viewMode.forkId)) {
    return { kind: "trajectory" };
  }

  return viewMode;
}

export function getSelectedEvalCaseForkId(mode: EvalCaseViewMode): string | null {
  switch (mode.kind) {
    case "fork":
      return mode.forkId;
    case "trajectory":
    case "sampleChat":
      return null;
  }
}

export function deriveEvalCaseContentView<
  TSample extends EvalCaseSampleLike,
  TFork extends EvalCaseThreadForkLike,
>(
  viewMode: EvalCaseViewMode,
  sample: TSample | undefined,
  forks: readonly TFork[]
): EvalCaseContentView<TSample> {
  switch (viewMode.kind) {
    case "trajectory":
      return sample ? { view: "trajectory", sample } : { view: "empty" };
    case "sampleChat":
      return sample?.threadId
        ? { view: "chat", threadId: sample.threadId }
        : sample
          ? { view: "trajectory", sample }
          : { view: "empty" };
    case "fork": {
      const fork = forks.find((candidate) => candidate.id === viewMode.forkId);
      return fork
        ? {
            view: "chat",
            threadId: fork.threadId,
            ...(typeof fork.forkAtMessageIndex === "number"
              ? { forkAtIndex: fork.forkAtMessageIndex }
              : {}),
          }
        : { view: "empty" };
    }
  }
}
