import {
  AlertTriangleIcon,
  BookCheckIcon,
  ChevronDownIcon,
  MinusIcon,
  PlusIcon,
} from "lucide-react";
import { useMemo, useState } from "react";
import { Link } from "react-router";

export type SharedEvalAggregationStrategy = "passRate" | "meanScore";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

export interface SharedEvalProviderOption {
  readonly key: string;
  readonly displayName: string;
  readonly models: readonly { readonly id: string; readonly name: string }[];
}

export interface SharedEvalTestCaseLike<TStatus extends string = string> {
  readonly id: string;
  readonly input: string;
  readonly status: TStatus;
  readonly expectedTrajectory: readonly unknown[];
  readonly groundTruth: string | null;
  readonly structuredGroundTruth: unknown | null;
  readonly tags: readonly string[];
}

export interface SharedEvalTestCaseSetLike {
  readonly id: string;
  readonly name: string;
  readonly testCases: readonly unknown[];
}

export interface SharedEvalRetryPolicyLike {
  readonly maxRetries: number;
  readonly initialBackoffMs: number;
  readonly backoffMultiplier: number;
}

export type SharedEvalScoreWeightsLike<TKey extends string = string> = Partial<
  Record<TKey, number>
>;

export interface SharedEvalScoreWeightOption<TKey extends string = string> {
  readonly key: TKey;
  readonly label: string;
  readonly description: string;
  readonly eligible?: boolean;
  readonly defaultWeight?: number;
}

export interface SharedEvalConfigLike<
  TMode extends string = string,
  TScorerKey extends string = string,
> {
  readonly mode: TMode;
  readonly targetAgentId?: string | null;
  readonly testCaseSetId: string;
  readonly testCaseIds: readonly string[] | null;
  readonly samplesPerCase: number;
  readonly concurrency: number;
  readonly passThreshold: number;
  readonly timeoutPerSampleSecs?: number | null;
  readonly provider?: string | null;
  readonly model?: string | null;
  readonly kValues: readonly number[];
  readonly retryPolicy?: SharedEvalRetryPolicyLike;
  readonly aggregationStrategy?: SharedEvalAggregationStrategy;
  readonly scoreWeights?: SharedEvalScoreWeightsLike<TScorerKey> | null;
}

export interface SharedEvalModeOption<TMode extends string = string> {
  readonly value: TMode;
  readonly label: string;
  readonly description: string;
  readonly targetAgentId?: string | null;
}

export interface SharedEvalConfigFormProps<
  TMode extends string = string,
  TStatus extends string = string,
  TScorerKey extends string = string,
  TConfig extends SharedEvalConfigLike<TMode, TScorerKey> = SharedEvalConfigLike<TMode, TScorerKey>,
  TCase extends SharedEvalTestCaseLike<TStatus> = SharedEvalTestCaseLike<TStatus>,
  TSet extends SharedEvalTestCaseSetLike = SharedEvalTestCaseSetLike,
> {
  readonly config: TConfig;
  readonly testCaseSets: readonly TSet[];
  readonly testCases: readonly TCase[];
  readonly providers?: readonly SharedEvalProviderOption[];
  readonly onUpdate: (partial: Partial<TConfig>) => void;
  readonly onSubmit: () => void;
  readonly isRunning: boolean;
  readonly modeOptions: readonly SharedEvalModeOption<TMode>[];
  readonly formatText?: (value: string) => string;
  readonly statusOrder: Record<TStatus, number>;
  readonly activeStatus: TStatus;
  readonly archivedStatus: TStatus;
  readonly showAdvancedScoring: boolean;
  readonly scoreWeightOptions?: readonly SharedEvalScoreWeightOption<TScorerKey>[];
  readonly defaultScoreWeights?: SharedEvalScoreWeightsLike<TScorerKey>;
  readonly allowTestCaseSets?: boolean;
  readonly timeoutFallbackSeconds?: number;
  readonly emptyTestCasesHref?: string;
}

type TestCaseSource = "tests" | "set";

export function updateSharedScoreWeight<TKey extends string>(
  weights: SharedEvalScoreWeightsLike<TKey>,
  key: TKey,
  weight: number | null
): SharedEvalScoreWeightsLike<TKey> {
  const next = { ...weights };
  if (weight === null) {
    delete next[key];
  } else {
    next[key] = weight;
  }
  return next;
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: shared form renderer spans multiple configuration sections
export function SharedEvalConfigForm<
  TMode extends string = string,
  TStatus extends string = string,
  TScorerKey extends string = string,
  TConfig extends SharedEvalConfigLike<TMode, TScorerKey> = SharedEvalConfigLike<TMode, TScorerKey>,
  TCase extends SharedEvalTestCaseLike<TStatus> = SharedEvalTestCaseLike<TStatus>,
  TSet extends SharedEvalTestCaseSetLike = SharedEvalTestCaseSetLike,
>({
  config,
  testCaseSets,
  testCases,
  providers,
  onUpdate,
  onSubmit,
  isRunning,
  modeOptions,
  formatText = (value) => value,
  statusOrder,
  activeStatus,
  archivedStatus,
  showAdvancedScoring,
  scoreWeightOptions = [],
  defaultScoreWeights,
  allowTestCaseSets = true,
  timeoutFallbackSeconds = 120,
  emptyTestCasesHref = "/tests/new",
}: SharedEvalConfigFormProps<TMode, TStatus, TScorerKey, TConfig, TCase, TSet>) {
  const [testCaseSource, setTestCaseSource] = useState<TestCaseSource>(() =>
    !allowTestCaseSets || (config.testCaseIds && config.testCaseIds.length > 0)
      ? "tests"
      : config.testCaseSetId
        ? "set"
        : "tests"
  );
  const [activeTagFilter, setActiveTagFilter] = useState<string | null>(null);
  const [filtersOpen, setFiltersOpen] = useState(false);
  const effectiveTestCaseSource = allowTestCaseSets ? testCaseSource : "tests";

  const selectedIds = useMemo(() => new Set(config.testCaseIds ?? []), [config.testCaseIds]);
  const sortedTestCases = useMemo(
    () =>
      [...testCases].sort((left, right) => statusOrder[left.status] - statusOrder[right.status]),
    [statusOrder, testCases]
  );
  const hasTestCases = (allowTestCaseSets && !!config.testCaseSetId) || selectedIds.size > 0;

  const allTags = useMemo(() => {
    const tagSet = new Set<string>();
    for (const testCase of testCases) {
      for (const tag of testCase.tags) tagSet.add(tag);
    }
    return [...tagSet].sort();
  }, [testCases]);

  const availableModels = providers
    ? config.provider
      ? (providers.find((provider) => provider.key === config.provider)?.models ?? [])
      : providers.flatMap((provider) => provider.models ?? [])
    : [];

  const scoreWeightEligibility = useMemo(
    () =>
      new Map<TScorerKey, boolean>(
        scoreWeightOptions.map((option) => [option.key, option.eligible ?? true])
      ),
    [scoreWeightOptions]
  );
  const scoreWeightEntries = Object.entries(config.scoreWeights ?? {}) as [TScorerKey, number][];
  const hasInvalidCustomScorers =
    config.scoreWeights != null &&
    scoreWeightOptions.length > 0 &&
    (!scoreWeightEntries.some(
      ([key, weight]) => (scoreWeightEligibility.get(key) ?? false) && weight > 0
    ) ||
      scoreWeightEntries.some(
        ([key, weight]) => !(scoreWeightEligibility.get(key) ?? false) && weight > 0
      ));

  const toConfigPatch = (
    patch: Partial<SharedEvalConfigLike<TMode, TScorerKey>>
  ): Partial<TConfig> => patch as unknown as Partial<TConfig>;

  return (
    <div className="space-y-5">
      <div>
        <span className="section-label mb-2 block">Eval Mode</span>
        <div data-testid="eval-mode-scroll" className="overflow-x-auto overflow-y-hidden pb-2">
          <div className="flex min-w-full w-max gap-2">
            {modeOptions.map(({ value, label, description, targetAgentId }) => (
              <button
                key={`${value}:${targetAgentId ?? ""}`}
                type="button"
                onClick={() => onUpdate(toConfigPatch({ mode: value, targetAgentId }))}
                className={cx(
                  "min-w-24 flex-1 shrink-0 basis-28 rounded-lg border p-3 text-left transition-all md:basis-36",
                  config.mode === value &&
                    (config.targetAgentId ?? null) === (targetAgentId ?? null)
                    ? "border-[var(--dot-blue)]/30 bg-[var(--accent-blue)]"
                    : "hover:border-border hover:bg-muted/40"
                )}
              >
                <div className="text-sm font-medium">{label}</div>
                <div className="text-xs text-muted-foreground">{description}</div>
              </button>
            ))}
          </div>
        </div>
      </div>

      <div>
        <div className="mb-2 flex items-baseline justify-between">
          <span className="section-label">Test Cases</span>
          {effectiveTestCaseSource === "tests" && selectedIds.size > 0 ? (
            <span className="text-xs text-muted-foreground">{selectedIds.size} selected</span>
          ) : null}
        </div>
        {allowTestCaseSets ? (
          <div className="mb-3 inline-flex items-center rounded-lg border bg-muted/30 p-0.5">
            <button
              type="button"
              onClick={() => {
                setTestCaseSource("tests");
                onUpdate(
                  toConfigPatch({ testCaseSetId: "", testCaseIds: config.testCaseIds ?? [] })
                );
              }}
              className={cx(
                "rounded-md px-3 py-1.5 text-xs font-medium transition-colors",
                effectiveTestCaseSource === "tests"
                  ? "bg-background text-foreground shadow-sm"
                  : "text-muted-foreground hover:text-foreground"
              )}
            >
              From Tests Tab
            </button>
            <button
              type="button"
              onClick={() => {
                setTestCaseSource("set");
                onUpdate(
                  toConfigPatch({ testCaseIds: null, testCaseSetId: config.testCaseSetId ?? "" })
                );
              }}
              className={cx(
                "rounded-md px-3 py-1.5 text-xs font-medium transition-colors",
                effectiveTestCaseSource === "set"
                  ? "bg-background text-foreground shadow-sm"
                  : "text-muted-foreground hover:text-foreground"
              )}
            >
              Uploaded Set
            </button>
          </div>
        ) : null}

        {effectiveTestCaseSource === "tests" ? (
          <div>
            {sortedTestCases.length === 0 ? (
              <div className="keyline-card rounded-lg p-4 text-center text-sm text-muted-foreground">
                No test cases found.{" "}
                <Link to={emptyTestCasesHref} className="text-primary hover:underline">
                  Create one in the Tests tab
                </Link>
              </div>
            ) : (
              <>
                {allTags.length > 0 ? (
                  <div className="mb-2">
                    <button
                      type="button"
                      onClick={() => setFiltersOpen(!filtersOpen)}
                      className="mb-1.5 flex items-center gap-1 text-[10px] uppercase tracking-wider text-muted-foreground transition-colors hover:text-foreground"
                    >
                      <ChevronDownIcon
                        className={cx("size-3 transition-transform", filtersOpen && "rotate-180")}
                      />
                      Filter
                      {activeTagFilter ? (
                        <span className="font-medium normal-case tracking-normal text-foreground">
                          : {activeTagFilter}
                        </span>
                      ) : null}
                    </button>
                    {filtersOpen ? (
                      <div className="flex flex-wrap items-center gap-1.5">
                        {allTags.map((tag) => {
                          const tagCount = sortedTestCases.filter((testCase) =>
                            testCase.tags.includes(tag)
                          ).length;
                          return (
                            <button
                              key={tag}
                              type="button"
                              onClick={() => {
                                if (activeTagFilter === tag) {
                                  setActiveTagFilter(null);
                                } else {
                                  setActiveTagFilter(tag);
                                  const matching = sortedTestCases
                                    .filter((testCase) => testCase.tags.includes(tag))
                                    .map((testCase) => testCase.id);
                                  onUpdate(toConfigPatch({ testCaseIds: matching }));
                                }
                              }}
                              className={cx(
                                "rounded border px-2 py-0.5 text-[10px] font-medium transition-colors",
                                activeTagFilter === tag
                                  ? "border-[var(--dot-blue)]/30 bg-[var(--accent-blue)] text-foreground"
                                  : "border-border/50 bg-transparent text-muted-foreground hover:border-border hover:text-foreground"
                              )}
                            >
                              {formatText(tag)}
                              <span className="ml-0.5 opacity-60">{tagCount}</span>
                            </button>
                          );
                        })}
                        {activeTagFilter ? (
                          <button
                            type="button"
                            onClick={() => setActiveTagFilter(null)}
                            className="text-[10px] text-muted-foreground hover:text-foreground"
                          >
                            Clear
                          </button>
                        ) : null}
                      </div>
                    ) : null}
                  </div>
                ) : null}

                <div className="mb-1.5 flex gap-3">
                  <button
                    type="button"
                    onClick={() => {
                      onUpdate(
                        toConfigPatch({
                          testCaseIds: sortedTestCases.map((testCase) => testCase.id),
                        })
                      );
                      setActiveTagFilter(null);
                    }}
                    className="text-xs text-primary hover:underline"
                  >
                    Select all
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      onUpdate(
                        toConfigPatch({
                          testCaseIds: sortedTestCases
                            .filter((testCase) => testCase.status === activeStatus)
                            .map((testCase) => testCase.id),
                        })
                      );
                      setActiveTagFilter(null);
                    }}
                    className="text-xs text-primary hover:underline"
                  >
                    Active only
                  </button>
                  {selectedIds.size > 0 ? (
                    <button
                      type="button"
                      onClick={() => {
                        onUpdate(toConfigPatch({ testCaseIds: [] }));
                        setActiveTagFilter(null);
                      }}
                      className="text-xs text-muted-foreground hover:underline"
                    >
                      Deselect all
                    </button>
                  ) : null}
                </div>

                <div className="keyline-card max-h-56 overflow-y-auto rounded-lg">
                  {(activeTagFilter
                    ? sortedTestCases.filter((testCase) => testCase.tags.includes(activeTagFilter))
                    : sortedTestCases
                  ).map(
                    // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: row rendering includes tags and readiness indicators.
                    (testCase) => {
                      const selected = selectedIds.has(testCase.id);

                      return (
                        <label
                          key={testCase.id}
                          className="group flex cursor-pointer items-center gap-2 border-b px-3 py-1.5 hover:bg-muted/50 last:border-b-0"
                        >
                          <input
                            type="checkbox"
                            checked={selected}
                            onChange={() => {
                              const current = config.testCaseIds ?? [];
                              const next = selected
                                ? current.filter((id) => id !== testCase.id)
                                : [...current, testCase.id];
                              onUpdate(toConfigPatch({ testCaseIds: next }));
                            }}
                            className="shrink-0 rounded"
                          />
                          <div className="min-w-0 flex-1">
                            <span className="block truncate text-sm">
                              {formatText(testCase.input)}
                            </span>
                            {testCase.tags.length > 0 ? (
                              <div className="mt-0.5 flex gap-1 opacity-0 transition-opacity group-hover:opacity-100">
                                {testCase.tags.map((tag) => (
                                  <span
                                    key={tag}
                                    className="rounded bg-muted px-1 py-px text-[9px] text-muted-foreground"
                                  >
                                    {formatText(tag)}
                                  </span>
                                ))}
                              </div>
                            ) : null}
                          </div>
                          {testCase.expectedTrajectory.length > 0 ? (
                            <span
                              className="shrink-0 text-[10px] font-mono tabular-nums text-muted-foreground/50"
                              title={`${testCase.expectedTrajectory.length} trajectory steps`}
                            >
                              {testCase.expectedTrajectory.length}
                            </span>
                          ) : null}
                          {testCase.structuredGroundTruth ? (
                            <BookCheckIcon
                              className="size-3 shrink-0 text-[var(--dot-emerald)]/60"
                              aria-label="Has structured ground truth"
                            />
                          ) : !testCase.groundTruth ? (
                            <AlertTriangleIcon
                              className="size-3 shrink-0 text-[var(--dot-amber)]/60"
                              aria-label="No ground truth"
                            />
                          ) : null}
                          <span
                            className={cx(
                              "tag-badge shrink-0",
                              testCase.status === activeStatus
                                ? "bg-[var(--accent-emerald)] text-[var(--dot-emerald)]"
                                : testCase.status === archivedStatus
                                  ? "bg-[var(--accent-amber)] text-[var(--dot-amber)]"
                                  : "bg-muted/60 text-muted-foreground"
                            )}
                          >
                            {String(testCase.status)}
                          </span>
                        </label>
                      );
                    }
                  )}
                </div>
              </>
            )}
          </div>
        ) : (
          <select
            value={config.testCaseSetId}
            onChange={(event) => onUpdate(toConfigPatch({ testCaseSetId: event.target.value }))}
            className="form-select"
          >
            <option value="">Select a test case set...</option>
            {testCaseSets.map((testCaseSet) => (
              <option key={testCaseSet.id} value={testCaseSet.id}>
                {formatText(testCaseSet.name)} ({testCaseSet.testCases.length} cases)
              </option>
            ))}
          </select>
        )}
      </div>

      <div className="grid grid-cols-4 gap-3">
        <NumberStepper
          id="eval-samples"
          label="Samples/Case"
          value={config.samplesPerCase}
          min={1}
          max={20}
          onChange={(value) =>
            onUpdate(
              toConfigPatch({
                samplesPerCase: value,
                kValues: config.kValues.filter((k) => k <= value),
              })
            )
          }
        />
        <NumberStepper
          id="eval-concurrency"
          label="Concurrency"
          value={config.concurrency}
          min={1}
          max={10}
          onChange={(value) => onUpdate(toConfigPatch({ concurrency: value }))}
        />
        <NumberStepper
          id="eval-threshold"
          label="Pass Threshold"
          value={config.passThreshold}
          min={0}
          max={1}
          step={0.05}
          onChange={(value) => onUpdate(toConfigPatch({ passThreshold: value }))}
        />
        <NumberStepper
          id="eval-timeout"
          label="Timeout (s)"
          value={config.timeoutPerSampleSecs ?? timeoutFallbackSeconds}
          min={10}
          max={600}
          step={10}
          onChange={(value) => onUpdate(toConfigPatch({ timeoutPerSampleSecs: value }))}
        />
      </div>

      <div>
        <span className="form-label">Pass@k Values</span>
        <div className="flex items-center gap-1.5">
          {Array.from({ length: Math.max(config.samplesPerCase, 5) }, (_, index) => index + 1).map(
            (k) => {
              const selected = config.kValues.includes(k);
              const disabled = k > config.samplesPerCase;
              return (
                <button
                  key={k}
                  type="button"
                  disabled={disabled}
                  onClick={() => {
                    const next = selected
                      ? config.kValues.filter((value) => value !== k)
                      : [...config.kValues, k].sort((left, right) => left - right);
                    if (next.length > 0) {
                      onUpdate(toConfigPatch({ kValues: next }));
                    }
                  }}
                  className={cx(
                    "size-8 rounded-md border text-sm font-medium transition-colors",
                    disabled
                      ? "cursor-not-allowed border-border/30 text-muted-foreground/30"
                      : selected
                        ? "border-[var(--dot-blue)]/30 bg-[var(--accent-blue)] text-foreground"
                        : "border-border/50 bg-transparent text-muted-foreground hover:border-border hover:text-foreground"
                  )}
                >
                  {k}
                </button>
              );
            }
          )}
          <span className="ml-1.5 text-[10px] text-muted-foreground">
            k &le; {config.samplesPerCase}
          </span>
        </div>
      </div>

      {providers && providers.length > 0 ? (
        <div className="grid grid-cols-2 gap-4">
          <div>
            <label htmlFor="eval-provider" className="form-label">
              Provider Override
            </label>
            <select
              id="eval-provider"
              value={config.provider ?? ""}
              onChange={(event) => {
                const value = event.target.value || null;
                onUpdate(toConfigPatch({ provider: value, model: null }));
              }}
              className="form-select"
            >
              <option value="">System Default</option>
              {providers.map((provider) => (
                <option key={provider.key} value={provider.key}>
                  {provider.displayName}
                </option>
              ))}
            </select>
          </div>
          <div>
            <label htmlFor="eval-model" className="form-label">
              Model Override
            </label>
            <select
              id="eval-model"
              value={config.model ?? ""}
              onChange={(event) => onUpdate(toConfigPatch({ model: event.target.value || null }))}
              className="form-select"
            >
              <option value="">System Default</option>
              {availableModels.map((model) => (
                <option key={model.id} value={model.id}>
                  {model.name}
                </option>
              ))}
            </select>
          </div>
        </div>
      ) : null}

      <RetryPolicySection config={config} onUpdate={onUpdate} />

      {showAdvancedScoring ? (
        <AdvancedScoringSection
          config={config}
          onUpdate={onUpdate}
          scoreWeightOptions={scoreWeightOptions}
          defaultScoreWeights={defaultScoreWeights}
        />
      ) : null}

      <button
        type="button"
        onClick={onSubmit}
        disabled={isRunning || !hasTestCases || hasInvalidCustomScorers}
        title={
          !hasTestCases
            ? "Select test cases first"
            : hasInvalidCustomScorers
              ? "Select at least one scorer"
              : undefined
        }
        className={cx(
          "w-full rounded-md py-2.5 text-sm font-medium transition-colors",
          isRunning || !hasTestCases || hasInvalidCustomScorers
            ? "cursor-not-allowed bg-muted text-muted-foreground"
            : "bg-primary text-primary-foreground hover:bg-primary/90"
        )}
      >
        {isRunning
          ? "Running..."
          : !hasTestCases
            ? "Select test cases to start"
            : hasInvalidCustomScorers
              ? "Select at least one scorer"
              : "Start Eval"}
      </button>
    </div>
  );
}

function NumberStepper({
  id,
  label,
  value,
  onChange,
  min = 0,
  max = 999,
  step = 1,
  suffix,
}: {
  readonly id: string;
  readonly label: string;
  readonly value: number;
  readonly onChange: (value: number) => void;
  readonly min?: number;
  readonly max?: number;
  readonly step?: number;
  readonly suffix?: string;
}) {
  const clamp = (value: number) => Math.min(max, Math.max(min, value));
  const isFloat = step < 1;
  const parse = isFloat ? Number.parseFloat : (raw: string) => Number.parseInt(raw, 10);

  return (
    <div>
      <label htmlFor={id} className="form-label">
        {label}
      </label>
      <div className="flex items-center overflow-hidden rounded-md border">
        <button
          type="button"
          onClick={() => onChange(clamp(value - step))}
          disabled={value <= min}
          className="px-2 py-2 text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-30"
          aria-label={`Decrease ${label}`}
        >
          <MinusIcon className="size-3" />
        </button>
        <input
          id={id}
          type="text"
          inputMode={isFloat ? "decimal" : "numeric"}
          value={isFloat ? value.toFixed(2) : value}
          onChange={(event) => {
            const nextValue = parse(event.target.value);
            if (!Number.isNaN(nextValue)) onChange(clamp(nextValue));
          }}
          className="min-w-0 flex-1 bg-background py-2 text-center text-sm tabular-nums focus:outline-none"
        />
        <button
          type="button"
          onClick={() => onChange(clamp(value + step))}
          disabled={value >= max}
          className="px-2 py-2 text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-30"
          aria-label={`Increase ${label}`}
        >
          <PlusIcon className="size-3" />
        </button>
      </div>
      {suffix ? <span className="text-[10px] text-muted-foreground">{suffix}</span> : null}
    </div>
  );
}

function RetryPolicySection<TMode extends string, TConfig extends SharedEvalConfigLike<TMode>>({
  config,
  onUpdate,
}: {
  readonly config: TConfig;
  readonly onUpdate: (partial: Partial<TConfig>) => void;
}) {
  const [open, setOpen] = useState(false);
  const policy = config.retryPolicy;
  const enabled = !!policy;
  const toConfigPatch = (patch: Partial<SharedEvalConfigLike<TMode>>): Partial<TConfig> =>
    patch as unknown as Partial<TConfig>;

  return (
    <div className="keyline-card rounded-lg">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="flex w-full items-center justify-between px-4 py-2.5 text-sm font-medium transition-colors hover:bg-muted/50"
      >
        <span>
          Retry Policy
          {enabled ? (
            <span className="ml-2 text-xs font-normal text-muted-foreground">
              {policy.maxRetries} retries
            </span>
          ) : null}
        </span>
        <ChevronDownIcon className={cx("size-4 transition-transform", open && "rotate-180")} />
      </button>
      {open ? (
        <div className="space-y-3 border-t px-4 pb-4 pt-3">
          <label className="flex cursor-pointer items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={enabled}
              onChange={(event) => {
                if (event.target.checked) {
                  onUpdate(
                    toConfigPatch({
                      retryPolicy: {
                        maxRetries: 2,
                        initialBackoffMs: 1000,
                        backoffMultiplier: 2.0,
                      },
                    })
                  );
                } else {
                  onUpdate(toConfigPatch({ retryPolicy: undefined }));
                }
              }}
              className="rounded"
            />
            Enable retries on failure
          </label>
          {enabled ? (
            <div className="grid grid-cols-3 gap-3">
              <NumberStepper
                id="rp-max"
                label="Max Retries"
                value={policy.maxRetries}
                min={1}
                max={5}
                onChange={(value) =>
                  onUpdate(toConfigPatch({ retryPolicy: { ...policy, maxRetries: value } }))
                }
              />
              <NumberStepper
                id="rp-backoff"
                label="Initial Backoff"
                value={policy.initialBackoffMs}
                min={100}
                max={30000}
                step={100}
                suffix="ms"
                onChange={(value) =>
                  onUpdate(
                    toConfigPatch({
                      retryPolicy: { ...policy, initialBackoffMs: value },
                    })
                  )
                }
              />
              <NumberStepper
                id="rp-mult"
                label="Multiplier"
                value={policy.backoffMultiplier}
                min={1}
                max={10}
                step={0.5}
                onChange={(value) =>
                  onUpdate(
                    toConfigPatch({
                      retryPolicy: { ...policy, backoffMultiplier: value },
                    })
                  )
                }
              />
            </div>
          ) : null}
          <p className="text-xs text-muted-foreground">
            Retries failed or timed-out samples with exponential backoff.
          </p>
        </div>
      ) : null}
    </div>
  );
}

const AGGREGATION_OPTIONS: readonly {
  readonly value: SharedEvalAggregationStrategy;
  readonly label: string;
  readonly description: string;
}[] = [
  { value: "passRate", label: "Pass Rate", description: "Binary pass/fail counting" },
  { value: "meanScore", label: "Mean Score", description: "Continuous mean of sample scores" },
];

function defaultScoreWeightsForOptions<TKey extends string>(
  options: readonly SharedEvalScoreWeightOption<TKey>[],
  explicitDefault?: SharedEvalScoreWeightsLike<TKey>
): SharedEvalScoreWeightsLike<TKey> {
  if (explicitDefault) {
    return explicitDefault;
  }
  return Object.fromEntries(
    options
      .filter((option) => option.eligible ?? true)
      .map((option) => [option.key, option.defaultWeight ?? 1])
  ) as SharedEvalScoreWeightsLike<TKey>;
}

function AdvancedScoringSection<
  TMode extends string,
  TScorerKey extends string,
  TConfig extends SharedEvalConfigLike<TMode, TScorerKey>,
>({
  config,
  onUpdate,
  scoreWeightOptions,
  defaultScoreWeights,
}: {
  readonly config: TConfig;
  readonly onUpdate: (partial: Partial<TConfig>) => void;
  readonly scoreWeightOptions: readonly SharedEvalScoreWeightOption<TScorerKey>[];
  readonly defaultScoreWeights?: SharedEvalScoreWeightsLike<TScorerKey>;
}) {
  const [open, setOpen] = useState(false);
  const customScorers = config.scoreWeights != null;
  const weights = (config.scoreWeights ?? {}) as SharedEvalScoreWeightsLike<TScorerKey>;
  const selectedCount = Object.values(weights).filter((weight) => Number(weight) > 0).length;
  const anyEligible = scoreWeightOptions.some((option) => option.eligible ?? true);
  const initialScoreWeights = defaultScoreWeightsForOptions(
    scoreWeightOptions,
    defaultScoreWeights
  );
  const toConfigPatch = (
    patch: Partial<SharedEvalConfigLike<TMode, TScorerKey>>
  ): Partial<TConfig> => patch as unknown as Partial<TConfig>;

  return (
    <div className="keyline-card rounded-lg">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="flex w-full items-center justify-between px-4 py-2.5 text-sm font-medium transition-colors hover:bg-muted/50"
      >
        <span>Advanced Scoring</span>
        <ChevronDownIcon className={cx("size-4 transition-transform", open && "rotate-180")} />
      </button>
      {open ? (
        <div className="space-y-4 border-t px-4 pb-4 pt-3">
          <div>
            <span className="section-label mb-1.5 block">Aggregation Strategy</span>
            <div className="flex gap-2">
              {AGGREGATION_OPTIONS.map(({ value, label, description }) => (
                <button
                  key={value}
                  type="button"
                  onClick={() => onUpdate(toConfigPatch({ aggregationStrategy: value }))}
                  className={cx(
                    "flex-1 rounded-md border p-2.5 text-left transition-all",
                    (config.aggregationStrategy ?? "passRate") === value
                      ? "border-[var(--dot-blue)]/30 bg-[var(--accent-blue)]"
                      : "hover:bg-muted/40"
                  )}
                >
                  <div className="text-sm font-medium">{label}</div>
                  <div className="text-xs text-muted-foreground">{description}</div>
                </button>
              ))}
            </div>
          </div>

          {scoreWeightOptions.length > 0 ? (
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <span className="section-label">Scorers</span>
                {customScorers ? (
                  <span className="text-[10px] text-muted-foreground">
                    {selectedCount} selected
                  </span>
                ) : null}
              </div>
              <label className="flex cursor-pointer items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={!customScorers}
                  onChange={(event) => {
                    if (event.target.checked) {
                      onUpdate(toConfigPatch({ scoreWeights: null }));
                    } else if (anyEligible) {
                      onUpdate(toConfigPatch({ scoreWeights: initialScoreWeights }));
                    }
                  }}
                  className="rounded"
                />
                Use harness defaults
              </label>
              {customScorers ? (
                <div className="space-y-1.5">
                  {scoreWeightOptions.map((option) => {
                    const eligible = option.eligible ?? true;
                    const selected = typeof weights[option.key] === "number";
                    const weight = weights[option.key] ?? option.defaultWeight ?? 1;
                    return (
                      <div
                        key={option.key}
                        className={cx(
                          "flex items-center gap-3 rounded-md border px-3 py-2",
                          eligible ? "bg-background" : "opacity-45"
                        )}
                      >
                        <label className="flex min-w-0 flex-1 items-center gap-2">
                          <input
                            type="checkbox"
                            checked={selected}
                            disabled={!eligible}
                            onChange={(event) => {
                              onUpdate(
                                toConfigPatch({
                                  scoreWeights: updateSharedScoreWeight(
                                    weights,
                                    option.key,
                                    event.target.checked ? (option.defaultWeight ?? 1) : null
                                  ),
                                })
                              );
                            }}
                            className="rounded"
                          />
                          <span className="min-w-0">
                            <span className="block text-sm font-medium">{option.label}</span>
                            <span className="block text-xs text-muted-foreground">
                              {eligible ? option.description : "No selected test case data"}
                            </span>
                          </span>
                        </label>
                        <input
                          type="number"
                          value={weight}
                          min={0}
                          step={0.1}
                          disabled={!eligible || !selected}
                          onChange={(event) => {
                            const parsed = Number.parseFloat(event.target.value);
                            onUpdate(
                              toConfigPatch({
                                scoreWeights: updateSharedScoreWeight(
                                  weights,
                                  option.key,
                                  Number.isFinite(parsed) && parsed >= 0 ? parsed : 0
                                ),
                              })
                            );
                          }}
                          className="w-20 rounded-md border bg-background px-2 py-1 text-sm tabular-nums disabled:opacity-50"
                          aria-label={`${option.label} weight`}
                        />
                      </div>
                    );
                  })}
                  {selectedCount === 0 ? (
                    <p className="text-xs text-[var(--dot-amber)]">
                      Select at least one eligible scorer or use harness defaults.
                    </p>
                  ) : null}
                </div>
              ) : null}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
