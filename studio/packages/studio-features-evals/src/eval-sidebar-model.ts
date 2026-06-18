export type EvalSidebarStatusFilter = "all" | "running" | "passed" | "failed";

export interface EvalSidebarCompletedStatusLike {
  readonly status: "completed";
  readonly summary: {
    readonly aggregateScore: number;
  };
}

export type EvalSidebarRunStatusLike =
  | { readonly status: "queued" }
  | { readonly status: "running" }
  | { readonly status: "paused" }
  | EvalSidebarCompletedStatusLike
  | { readonly status: "failed" }
  | { readonly status: "cancelled" }
  | { readonly status: "skipped" };

export interface EvalSidebarRunLike<TMode extends string = string> {
  readonly id: string;
  readonly createdAt: string;
  readonly parentRunId?: string | null;
  readonly resultCount: number;
  readonly config: {
    readonly mode: TMode;
    readonly model: string | null;
    readonly passThreshold: number;
  };
  readonly status: EvalSidebarRunStatusLike;
}

const KNOWN_EVAL_MODE_LABELS: Record<string, string> = {
  planner: "Planner",
  executor: "Executor",
  sequential: "Sequential",
  testCaseBuilder: "Test Case Builder",
};

export function matchesEvalSidebarStatusFilter(
  run: EvalSidebarRunLike,
  filter: EvalSidebarStatusFilter
): boolean {
  if (filter === "all") return true;
  if (filter === "running") {
    return run.status.status === "running" || run.status.status === "paused";
  }
  if (filter === "passed") {
    return (
      run.status.status === "completed" &&
      run.status.summary.aggregateScore >= run.config.passThreshold
    );
  }

  return (
    run.status.status === "failed" ||
    run.status.status === "cancelled" ||
    (run.status.status === "completed" &&
      run.status.summary.aggregateScore < run.config.passThreshold)
  );
}

export function getEvalModeLabel(mode: string | null | undefined): string {
  if (!mode) return "Unknown";
  return KNOWN_EVAL_MODE_LABELS[mode] ?? `${mode.charAt(0).toUpperCase()}${mode.slice(1)}`;
}

export function getShortEvalModelLabel(model: string | null): string | null {
  if (!model) return null;

  const normalized = model.toLowerCase();
  if (normalized.includes("opus")) return "opus";
  if (normalized.includes("sonnet")) return "sonnet";
  if (normalized.includes("haiku")) return "haiku";
  if (normalized.includes("gpt-4o")) return "4o";
  if (normalized.includes("gpt-4")) return "gpt4";
  if (normalized.includes("glm")) return "glm";

  const parts = model.split("-");
  return parts.length > 1 ? parts.slice(0, 2).join("-") : model.slice(0, 12);
}

export function filterEvalSidebarRuns<TRun extends EvalSidebarRunLike>(
  runs: readonly TRun[],
  search: string,
  filter: EvalSidebarStatusFilter
): TRun[] {
  const query = search.trim().toLowerCase();

  return runs.filter((run) => {
    if (!matchesEvalSidebarStatusFilter(run, filter)) return false;
    if (!query) return true;

    return (
      run.id.toLowerCase().includes(query) ||
      run.config.mode.toLowerCase().includes(query) ||
      (run.config.model ?? "").toLowerCase().includes(query)
    );
  });
}
