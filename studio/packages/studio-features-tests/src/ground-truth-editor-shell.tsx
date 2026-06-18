import { ChevronRightIcon, PlusIcon, TrashIcon, XIcon } from "lucide-react";
import {
  type KeyboardEvent,
  type MutableRefObject,
  type ReactNode,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

export type SharedGroundTruthMode = "text" | "flat" | "multiGroup";

export interface SharedGroundTruthNumericFilterLike<TNumericOp extends string = string> {
  readonly operator: TNumericOp;
  readonly value: number;
}

export interface SharedGroundTruthMeasureFilterLike<TNumericOp extends string = string> {
  readonly column: string;
  readonly operator: TNumericOp;
  readonly value: number;
}

export interface SharedExpectedFiltersLike<TNumericOp extends string = string> {
  readonly matchedFilters: Record<string, string[]>;
  readonly numericFilters: Record<string, SharedGroundTruthNumericFilterLike<TNumericOp>>;
  readonly booleanFilters: Record<string, boolean>;
  readonly measureFilters: Record<string, SharedGroundTruthMeasureFilterLike<TNumericOp>>;
}

export function SharedAutoGrowTextarea({
  value,
  onChange,
  placeholder,
  minRows = 3,
  maxRows = 20,
  className,
}: {
  readonly value: string;
  readonly onChange: (value: string) => void;
  readonly placeholder?: string;
  readonly minRows?: number;
  readonly maxRows?: number;
  readonly className?: string;
}) {
  const ref = useRef<HTMLTextAreaElement>(null);

  const resize = useCallback(() => {
    const element = ref.current;
    if (!element) return;
    element.style.height = "auto";
    const lineHeight = 20;
    const minHeight = minRows * lineHeight + 16;
    const maxHeight = maxRows * lineHeight + 16;
    element.style.height = `${Math.min(Math.max(element.scrollHeight, minHeight), maxHeight)}px`;
  }, [maxRows, minRows]);

  useEffect(resize, [resize, value]);

  return (
    <textarea
      ref={ref}
      value={value}
      onChange={(event) => onChange(event.target.value)}
      placeholder={placeholder}
      className={cx("form-textarea resize-none overflow-y-auto scroll-container", className)}
      style={{ minHeight: `${minRows * 20 + 16}px` }}
    />
  );
}

export function SharedInlinePills({
  items,
  onChange,
  placeholder,
  mono,
  truncate,
}: {
  readonly items: string[];
  readonly onChange: (items: string[]) => void;
  readonly placeholder: string;
  readonly mono?: boolean;
  readonly truncate?: (value: string) => string;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  const handleAdd = useCallback(() => {
    const trimmed = draft.trim();
    if (!trimmed) return;
    const nextItems = trimmed
      .split(",")
      .map((value) => value.trim())
      .filter(Boolean);
    onChange([...items, ...nextItems]);
    setDraft("");
  }, [draft, items, onChange]);

  const handleKeyDown = useCallback(
    (event: KeyboardEvent) => {
      if (event.key === "Enter" || event.key === ",") {
        event.preventDefault();
        handleAdd();
      }
      if (event.key === "Escape") {
        setEditing(false);
        setDraft("");
      }
      if (event.key === "Backspace" && !draft && items.length > 0) {
        onChange(items.slice(0, -1));
      }
    },
    [draft, handleAdd, items, onChange]
  );

  if (items.length === 0 && !editing) {
    return (
      <button
        type="button"
        onClick={() => {
          setEditing(true);
          requestAnimationFrame(() => inputRef.current?.focus());
        }}
        className="text-[10px] text-muted-foreground/50 hover:text-muted-foreground transition-colors rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      >
        + {placeholder}
      </button>
    );
  }

  return (
    <div className="flex flex-wrap items-center gap-1">
      {items.map((item, index) => (
        <span
          key={`${index}-${item}`}
          className={cx(
            "inline-flex items-center gap-0.5 rounded bg-muted/60 px-1.5 py-px text-[10px] text-muted-foreground group/pill",
            mono && "font-mono"
          )}
          title={item}
        >
          {truncate ? truncate(item) : item}
          <button
            type="button"
            onClick={() => onChange(items.filter((_, itemIndex) => itemIndex !== index))}
            className="rounded p-px opacity-0 transition-opacity group-hover/pill:opacity-100 hover:bg-black/5 dark:hover:bg-white/10"
            aria-label={`Remove ${item}`}
          >
            <XIcon className="size-2" />
          </button>
        </span>
      ))}
      {editing ? (
        <input
          ref={inputRef}
          type="text"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={handleKeyDown}
          onBlur={() => {
            handleAdd();
            if (!draft.trim()) setEditing(false);
          }}
          placeholder={placeholder}
          className={cx(
            "min-w-[60px] max-w-[120px] bg-transparent px-1 py-0.5 text-[10px] outline-none",
            mono && "font-mono"
          )}
          autoFocus
        />
      ) : (
        <button
          type="button"
          onClick={() => {
            setEditing(true);
            requestAnimationFrame(() => inputRef.current?.focus());
          }}
          className="px-1 text-[10px] text-muted-foreground/40 hover:text-muted-foreground transition-colors"
        >
          +
        </button>
      )}
    </div>
  );
}

export function useSharedGroundTruthMode<TStructured>({
  structuredGroundTruth,
  onStructuredGroundTruthChange,
  modeFromStructured,
  createFlat,
  createMultiGroup,
}: {
  readonly structuredGroundTruth: TStructured | null;
  readonly onStructuredGroundTruthChange: (value: TStructured | null) => void;
  readonly modeFromStructured: (value: TStructured | null) => SharedGroundTruthMode;
  readonly createFlat: () => TStructured;
  readonly createMultiGroup: () => TStructured;
}) {
  const [mode, setMode] = useState<SharedGroundTruthMode>(() =>
    modeFromStructured(structuredGroundTruth)
  );
  const cachedStructuredGroundTruth = useRef<TStructured | null>(structuredGroundTruth);

  useEffect(() => {
    if (structuredGroundTruth) {
      cachedStructuredGroundTruth.current = structuredGroundTruth;
    }
  }, [structuredGroundTruth]);

  useEffect(() => {
    setMode(modeFromStructured(structuredGroundTruth));
  }, [modeFromStructured, structuredGroundTruth]);

  const handleModeChange = useCallback(
    (nextMode: SharedGroundTruthMode) => {
      setMode(nextMode);

      if (nextMode === "text") {
        if (structuredGroundTruth) {
          cachedStructuredGroundTruth.current = structuredGroundTruth;
        }
        onStructuredGroundTruthChange(null);
        return;
      }

      const cached = cachedStructuredGroundTruth.current;
      if (cached && modeFromStructured(cached) === nextMode) {
        onStructuredGroundTruthChange(cached);
        return;
      }

      onStructuredGroundTruthChange(nextMode === "flat" ? createFlat() : createMultiGroup());
    },
    [
      createFlat,
      createMultiGroup,
      modeFromStructured,
      onStructuredGroundTruthChange,
      structuredGroundTruth,
    ]
  );

  return { mode, handleModeChange };
}

export interface SharedRenderScopeEditorArgs<TScope> {
  readonly scope: TScope | undefined;
  readonly onChange: (scope: TScope | undefined) => void;
}

export interface SharedRenderActionFieldsArgs<TAction, TScope> {
  readonly action: TAction;
  readonly onChange: (action: TAction) => void;
  readonly renderScopeEditor: (
    scope: TScope | undefined,
    onChange: (scope: TScope | undefined) => void
  ) => ReactNode;
}

export interface SharedRenderGroupExtraFieldsArgs<TGroup> {
  readonly group: TGroup;
  readonly onChange: (group: TGroup) => void;
}

export interface SharedGroundTruthFlatState<
  TAction,
  TScope,
  TFilters extends SharedExpectedFiltersLike<TNumericOp>,
  TNumericOp extends string = string,
> {
  readonly actions: readonly TAction[];
  readonly onActionsChange: (actions: TAction[]) => void;
  readonly filters: TFilters;
  readonly onFiltersChange: (filters: TFilters) => void;
  readonly scope: TScope | undefined;
  readonly onScopeChange: (scope: TScope | undefined) => void;
  readonly targetSection?: ReactNode;
}

export interface SharedGroundTruthGroupState<
  TGroup,
  TAction,
  TScope,
  TFilters extends SharedExpectedFiltersLike<TNumericOp>,
  TNumericOp extends string = string,
> {
  readonly groups: readonly TGroup[];
  readonly onGroupsChange: (groups: TGroup[]) => void;
  readonly createEmptyGroup: () => TGroup;
  readonly getFilters: (group: TGroup) => TFilters;
  readonly setFilters: (group: TGroup, filters: TFilters) => TGroup;
  readonly getScope: (group: TGroup) => TScope | undefined;
  readonly setScope: (group: TGroup, scope: TScope | undefined) => TGroup;
  readonly getActions: (group: TGroup) => readonly TAction[];
  readonly setActions: (group: TGroup, actions: TAction[]) => TGroup;
  readonly renderGroupExtraFields: (args: SharedRenderGroupExtraFieldsArgs<TGroup>) => ReactNode;
}

function createActionRowId(counterRef: MutableRefObject<number>): string {
  const next = counterRef.current;
  counterRef.current += 1;
  return `action-row-${next}`;
}

function findInsertionIndex<TAction>(
  prevActions: readonly TAction[],
  nextActions: readonly TAction[]
) {
  let prevIndex = 0;
  let nextIndex = 0;
  while (prevIndex < prevActions.length && nextIndex < nextActions.length) {
    if (prevActions[prevIndex] !== nextActions[nextIndex]) return nextIndex;
    prevIndex += 1;
    nextIndex += 1;
  }
  return nextActions.length - 1;
}

function findRemovalIndex<TAction>(
  prevActions: readonly TAction[],
  nextActions: readonly TAction[]
) {
  let prevIndex = 0;
  let nextIndex = 0;
  while (prevIndex < prevActions.length && nextIndex < nextActions.length) {
    if (prevActions[prevIndex] !== nextActions[nextIndex]) return prevIndex;
    prevIndex += 1;
    nextIndex += 1;
  }
  return prevActions.length - 1;
}

function reconcileActionRowIds<TAction>(
  prevRowIds: readonly string[],
  prevActions: readonly TAction[],
  nextActions: readonly TAction[],
  counterRef: MutableRefObject<number>
) {
  if (nextActions.length === prevActions.length) {
    return prevRowIds.slice(0, nextActions.length);
  }
  if (nextActions.length === prevActions.length + 1) {
    const insertionIndex = findInsertionIndex(prevActions, nextActions);
    const nextRowIds = [...prevRowIds];
    nextRowIds.splice(insertionIndex, 0, createActionRowId(counterRef));
    return nextRowIds;
  }
  if (nextActions.length + 1 === prevActions.length) {
    const removalIndex = findRemovalIndex(prevActions, nextActions);
    const nextRowIds = [...prevRowIds];
    nextRowIds.splice(removalIndex, 1);
    return nextRowIds;
  }
  return nextActions.map((_, index) => prevRowIds[index] ?? createActionRowId(counterRef));
}

function ActionEditorShell<TAction, TScope>({
  action,
  index,
  onChange,
  onRemove,
  renderActionSummary,
  renderActionFields,
  renderScopeEditor,
  getActionAccentColor,
}: {
  readonly action: TAction;
  readonly index: number;
  readonly onChange: (action: TAction) => void;
  readonly onRemove: () => void;
  readonly renderActionSummary: (action: TAction) => ReactNode;
  readonly renderActionFields: (args: SharedRenderActionFieldsArgs<TAction, TScope>) => ReactNode;
  readonly renderScopeEditor: (args: SharedRenderScopeEditorArgs<TScope>) => ReactNode;
  readonly getActionAccentColor: (action: TAction) => string;
}) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div
      className="overflow-hidden rounded-md bg-muted/5"
      style={{ boxShadow: `inset 3px 0 0 ${getActionAccentColor(action)}` }}
    >
      <div className="group/action flex items-center gap-2 px-3 py-1.5">
        <button
          type="button"
          onClick={() => setExpanded(!expanded)}
          className="shrink-0 rounded p-0.5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-label={expanded ? "Collapse" : "Expand"}
        >
          <ChevronRightIcon
            className={cx(
              "size-3 text-muted-foreground/50 transition-transform",
              expanded && "rotate-90"
            )}
          />
        </button>

        <span className="w-3 shrink-0 text-[9px] tabular-nums text-muted-foreground/40">
          {index + 1}
        </span>

        {renderActionSummary(action)}

        <button
          type="button"
          onClick={onRemove}
          className="ml-auto shrink-0 rounded p-0.5 text-destructive/30 opacity-0 transition-colors group-hover/action:opacity-100 hover:text-destructive focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-label="Remove action"
        >
          <TrashIcon className="size-3" />
        </button>
      </div>

      {expanded ? (
        <div className="space-y-2 border-t border-border/30 px-3 pb-3 pt-1">
          {renderActionFields({
            action,
            onChange,
            renderScopeEditor: (scope, onScopeChange) =>
              renderScopeEditor({ scope, onChange: onScopeChange }),
          })}
        </div>
      ) : null}
    </div>
  );
}

function ActionList<TAction, TScope>({
  actions,
  onChange,
  createEmptyAction,
  renderActionSummary,
  renderActionFields,
  renderScopeEditor,
  getActionAccentColor,
}: {
  readonly actions: readonly TAction[];
  readonly onChange: (actions: TAction[]) => void;
  readonly createEmptyAction: () => TAction;
  readonly renderActionSummary: (action: TAction) => ReactNode;
  readonly renderActionFields: (args: SharedRenderActionFieldsArgs<TAction, TScope>) => ReactNode;
  readonly renderScopeEditor: (args: SharedRenderScopeEditorArgs<TScope>) => ReactNode;
  readonly getActionAccentColor: (action: TAction) => string;
}) {
  const actionRowCounterRef = useRef(0);
  const actionRowsRef = useRef<{ actions: readonly TAction[]; rowIds: string[] }>({
    actions,
    rowIds: actions.map(() => createActionRowId(actionRowCounterRef)),
  });

  if (actionRowsRef.current.actions !== actions) {
    actionRowsRef.current = {
      actions,
      rowIds: reconcileActionRowIds(
        actionRowsRef.current.rowIds,
        actionRowsRef.current.actions,
        actions,
        actionRowCounterRef
      ),
    };
  }

  const keyedActions = actions.map((action, index) => ({
    action,
    index,
    key: actionRowsRef.current.rowIds[index] ?? `action-fallback-${index}`,
  }));

  return (
    <div className="space-y-1">
      {keyedActions.map(({ action, index, key }) => (
        <ActionEditorShell
          key={key}
          action={action}
          index={index}
          onChange={(updated) => {
            const next = [...actions];
            next[index] = updated;
            onChange(next);
          }}
          onRemove={() => onChange(actions.filter((_, actionIndex) => actionIndex !== index))}
          renderActionSummary={renderActionSummary}
          renderActionFields={renderActionFields}
          renderScopeEditor={renderScopeEditor}
          getActionAccentColor={getActionAccentColor}
        />
      ))}
      <button
        type="button"
        onClick={() => onChange([...actions, createEmptyAction()])}
        className="flex items-center gap-1 rounded pt-0.5 text-[10px] text-muted-foreground/50 transition-colors hover:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <PlusIcon className="size-2.5" />
        Add action
      </button>
    </div>
  );
}

function KeyValueListEditor({
  label,
  entries,
  onChange,
}: {
  readonly label: string;
  readonly entries: { key: string; value: string }[];
  readonly onChange: (entries: { key: string; value: string }[]) => void;
}) {
  return (
    <div className="space-y-1">
      <span className="text-[9px] uppercase tracking-wider text-muted-foreground/60">{label}</span>
      {entries.map((entry, index) => (
        <div
          // biome-ignore lint/suspicious/noArrayIndexKey: dynamic form list
          key={`kv-${index}`}
          className="flex items-center gap-1.5"
        >
          <input
            type="text"
            value={entry.key}
            onChange={(event) => {
              const next = [...entries];
              next[index] = { ...entry, key: event.target.value };
              onChange(next);
            }}
            placeholder="column"
            className="form-input form-input-sm flex-1"
          />
          <input
            type="text"
            value={entry.value}
            onChange={(event) => {
              const next = [...entries];
              next[index] = { ...entry, value: event.target.value };
              onChange(next);
            }}
            placeholder="values"
            className="form-input form-input-sm flex-[2]"
          />
          <button
            type="button"
            onClick={() => onChange(entries.filter((_, entryIndex) => entryIndex !== index))}
            className="rounded p-0.5 text-destructive/30 transition-colors hover:text-destructive focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <TrashIcon className="size-3" />
          </button>
        </div>
      ))}
      <button
        type="button"
        onClick={() => onChange([...entries, { key: "", value: "" }])}
        className="flex items-center gap-0.5 rounded text-[10px] text-muted-foreground/50 transition-colors hover:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <PlusIcon className="size-2.5" /> add
      </button>
    </div>
  );
}

function NumericFiltersEditor<TNumericOp extends string>({
  filters,
  onChange,
  numericOperatorOptions,
  parseNumericOperator,
}: {
  readonly filters: Record<string, SharedGroundTruthNumericFilterLike<TNumericOp>>;
  readonly onChange: (
    filters: Record<string, SharedGroundTruthNumericFilterLike<TNumericOp>>
  ) => void;
  readonly numericOperatorOptions: readonly { value: TNumericOp; label: string }[];
  readonly parseNumericOperator: (value: string) => TNumericOp | undefined;
}) {
  const entries = Object.entries(filters);
  const defaultOperator = numericOperatorOptions[0]?.value;

  return (
    <div className="space-y-1">
      <span className="text-[9px] uppercase tracking-wider text-muted-foreground/60">
        Numeric Filters
      </span>
      {entries.map(([column, numericFilter], index) => (
        <div
          // biome-ignore lint/suspicious/noArrayIndexKey: dynamic form list
          key={`nf-${index}`}
          className="flex items-center gap-1.5"
        >
          <input
            type="text"
            value={column}
            onChange={(event) => {
              const next = { ...filters };
              delete next[column];
              if (event.target.value.trim()) next[event.target.value.trim()] = numericFilter;
              onChange(next);
            }}
            placeholder="column"
            className="form-input form-input-sm flex-1"
          />
          <select
            value={numericFilter.operator}
            onChange={(event) =>
              onChange({
                ...filters,
                [column]: {
                  ...numericFilter,
                  operator: parseNumericOperator(event.target.value) ?? numericFilter.operator,
                },
              })
            }
            className="form-select form-input-sm"
          >
            {numericOperatorOptions.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
          <input
            type="number"
            value={numericFilter.value}
            onChange={(event) =>
              onChange({
                ...filters,
                [column]: { ...numericFilter, value: Number(event.target.value) || 0 },
              })
            }
            className="form-input form-input-sm w-20 font-mono tabular-nums"
          />
          <button
            type="button"
            onClick={() => {
              const next = { ...filters };
              delete next[column];
              onChange(next);
            }}
            className="rounded p-0.5 text-destructive/30 transition-colors hover:text-destructive focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <TrashIcon className="size-3" />
          </button>
        </div>
      ))}
      <button
        type="button"
        onClick={() => {
          if (!defaultOperator) return;
          onChange({ ...filters, "": { operator: defaultOperator, value: 0 } });
        }}
        className="flex items-center gap-0.5 rounded text-[10px] text-muted-foreground/50 transition-colors hover:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <PlusIcon className="size-2.5" /> add
      </button>
    </div>
  );
}

function BooleanFiltersEditor({
  filters,
  onChange,
}: {
  readonly filters: Record<string, boolean>;
  readonly onChange: (filters: Record<string, boolean>) => void;
}) {
  const entries = Object.entries(filters);

  return (
    <div className="space-y-1">
      <span className="text-[9px] uppercase tracking-wider text-muted-foreground/60">
        Boolean Filters
      </span>
      {entries.map(([column, value], index) => (
        <div
          // biome-ignore lint/suspicious/noArrayIndexKey: dynamic form list
          key={`bf-${index}`}
          className="flex items-center gap-1.5"
        >
          <input
            type="text"
            value={column}
            onChange={(event) => {
              const next = { ...filters };
              delete next[column];
              if (event.target.value.trim()) next[event.target.value.trim()] = value;
              onChange(next);
            }}
            placeholder="column"
            className="form-input form-input-sm flex-1"
          />
          <select
            value={value ? "true" : "false"}
            onChange={(event) => onChange({ ...filters, [column]: event.target.value === "true" })}
            className="form-select form-input-sm"
          >
            <option value="true">true</option>
            <option value="false">false</option>
          </select>
          <button
            type="button"
            onClick={() => {
              const next = { ...filters };
              delete next[column];
              onChange(next);
            }}
            className="rounded p-0.5 text-destructive/30 transition-colors hover:text-destructive focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <TrashIcon className="size-3" />
          </button>
        </div>
      ))}
      <button
        type="button"
        onClick={() => onChange({ ...filters, "": true })}
        className="flex items-center gap-0.5 rounded text-[10px] text-muted-foreground/50 transition-colors hover:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <PlusIcon className="size-2.5" /> add
      </button>
    </div>
  );
}

function MeasureFiltersEditor<TNumericOp extends string>({
  filters,
  onChange,
  numericOperatorOptions,
  parseNumericOperator,
}: {
  readonly filters: Record<string, SharedGroundTruthMeasureFilterLike<TNumericOp>>;
  readonly onChange: (
    filters: Record<string, SharedGroundTruthMeasureFilterLike<TNumericOp>>
  ) => void;
  readonly numericOperatorOptions: readonly { value: TNumericOp; label: string }[];
  readonly parseNumericOperator: (value: string) => TNumericOp | undefined;
}) {
  const entries = Object.entries(filters);
  const defaultOperator = numericOperatorOptions[0]?.value;

  return (
    <div className="space-y-1">
      <span className="text-[9px] uppercase tracking-wider text-muted-foreground/60">
        Measure Filters
      </span>
      {entries.map(([key, measureFilter], index) => (
        <div
          // biome-ignore lint/suspicious/noArrayIndexKey: dynamic form list
          key={`mf-${index}`}
          className="flex items-center gap-1.5"
        >
          <input
            type="text"
            value={measureFilter.column}
            onChange={(event) =>
              onChange({ ...filters, [key]: { ...measureFilter, column: event.target.value } })
            }
            placeholder="column"
            className="form-input form-input-sm flex-1"
          />
          <select
            value={measureFilter.operator}
            onChange={(event) =>
              onChange({
                ...filters,
                [key]: {
                  ...measureFilter,
                  operator: parseNumericOperator(event.target.value) ?? measureFilter.operator,
                },
              })
            }
            className="form-select form-input-sm"
          >
            {numericOperatorOptions.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
          <input
            type="number"
            value={measureFilter.value}
            onChange={(event) =>
              onChange({
                ...filters,
                [key]: { ...measureFilter, value: Number(event.target.value) || 0 },
              })
            }
            className="form-input form-input-sm w-20 font-mono tabular-nums"
          />
          <button
            type="button"
            onClick={() => {
              const next = { ...filters };
              delete next[key];
              onChange(next);
            }}
            className="rounded p-0.5 text-destructive/30 transition-colors hover:text-destructive focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <TrashIcon className="size-3" />
          </button>
        </div>
      ))}
      <button
        type="button"
        onClick={() => {
          if (!defaultOperator) return;
          const maxIndex = entries.reduce((max, [key]) => {
            const match = key.match(/^measure_(\d+)$/);
            return match ? Math.max(max, Number(match[1])) : max;
          }, -1);
          const key = `measure_${maxIndex + 1}`;
          onChange({ ...filters, [key]: { column: "", operator: defaultOperator, value: 0 } });
        }}
        className="flex items-center gap-0.5 rounded text-[10px] text-muted-foreground/50 transition-colors hover:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <PlusIcon className="size-2.5" /> add
      </button>
    </div>
  );
}

function FiltersEditor<
  TNumericOp extends string,
  TFilters extends SharedExpectedFiltersLike<TNumericOp>,
>({
  filters,
  onChange,
  numericOperatorOptions,
  parseNumericOperator,
}: {
  readonly filters: TFilters;
  readonly onChange: (filters: TFilters) => void;
  readonly numericOperatorOptions: readonly { value: TNumericOp; label: string }[];
  readonly parseNumericOperator: (value: string) => TNumericOp | undefined;
}) {
  const [expanded, setExpanded] = useState(false);

  const filterCount =
    Object.keys(filters.matchedFilters).length +
    Object.keys(filters.numericFilters).length +
    Object.keys(filters.booleanFilters).length +
    Object.keys(filters.measureFilters).length;

  return (
    <div className="space-y-1">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="group flex items-center gap-1.5 rounded text-[10px] uppercase tracking-wider text-muted-foreground/60 transition-colors hover:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <ChevronRightIcon className={cx("size-3 transition-transform", expanded && "rotate-90")} />
        Filters
        {filterCount > 0 ? (
          <>
            <span className="status-dot bg-[var(--dot-emerald)]" />
            <span className="normal-case tracking-normal text-muted-foreground/40">
              {filterCount}
            </span>
          </>
        ) : null}
      </button>

      {expanded ? (
        <div className="ml-1.5 space-y-3 border-l-2 border-border/30 pl-3">
          <KeyValueListEditor
            label="Matched Filters"
            entries={Object.entries(filters.matchedFilters).map(([key, value]) => ({
              key,
              value: value.join(", "),
            }))}
            onChange={(entries) => {
              const matchedFilters: Record<string, string[]> = {};
              for (const entry of entries) {
                if (!entry.key.trim()) continue;
                matchedFilters[entry.key.trim()] = entry.value
                  .split(",")
                  .map((value) => value.trim())
                  .filter(Boolean);
              }
              onChange({ ...filters, matchedFilters } as TFilters);
            }}
          />
          <NumericFiltersEditor
            filters={filters.numericFilters}
            onChange={(numericFilters) => onChange({ ...filters, numericFilters } as TFilters)}
            numericOperatorOptions={numericOperatorOptions}
            parseNumericOperator={parseNumericOperator}
          />
          <BooleanFiltersEditor
            filters={filters.booleanFilters}
            onChange={(booleanFilters) => onChange({ ...filters, booleanFilters } as TFilters)}
          />
          <MeasureFiltersEditor
            filters={filters.measureFilters}
            onChange={(measureFilters) => onChange({ ...filters, measureFilters } as TFilters)}
            numericOperatorOptions={numericOperatorOptions}
            parseNumericOperator={parseNumericOperator}
          />
        </div>
      ) : null}
    </div>
  );
}

function GroupEditorShell<
  TGroup,
  TAction,
  TScope,
  TNumericOp extends string,
  TFilters extends SharedExpectedFiltersLike<TNumericOp>,
>({
  group,
  index,
  onChange,
  onRemove,
  createEmptyAction,
  getFilters,
  setFilters,
  getScope,
  setScope,
  getActions,
  setActions,
  renderScopeEditor,
  renderActionSummary,
  renderActionFields,
  getActionAccentColor,
  renderGroupExtraFields,
  numericOperatorOptions,
  parseNumericOperator,
}: {
  readonly group: TGroup;
  readonly index: number;
  readonly onChange: (group: TGroup) => void;
  readonly onRemove: () => void;
  readonly createEmptyAction: () => TAction;
  readonly getFilters: (group: TGroup) => TFilters;
  readonly setFilters: (group: TGroup, filters: TFilters) => TGroup;
  readonly getScope: (group: TGroup) => TScope | undefined;
  readonly setScope: (group: TGroup, scope: TScope | undefined) => TGroup;
  readonly getActions: (group: TGroup) => readonly TAction[];
  readonly setActions: (group: TGroup, actions: TAction[]) => TGroup;
  readonly renderScopeEditor: (args: SharedRenderScopeEditorArgs<TScope>) => ReactNode;
  readonly renderActionSummary: (action: TAction) => ReactNode;
  readonly renderActionFields: (args: SharedRenderActionFieldsArgs<TAction, TScope>) => ReactNode;
  readonly getActionAccentColor: (action: TAction) => string;
  readonly renderGroupExtraFields: (args: SharedRenderGroupExtraFieldsArgs<TGroup>) => ReactNode;
  readonly numericOperatorOptions: readonly { value: TNumericOp; label: string }[];
  readonly parseNumericOperator: (value: string) => TNumericOp | undefined;
}) {
  const [expanded, setExpanded] = useState(true);
  const actions = getActions(group);

  return (
    <div
      className="overflow-hidden rounded-md bg-muted/5"
      style={{ boxShadow: "inset 3px 0 0 var(--dot-purple)" }}
    >
      <div className="group/group flex items-center gap-2 px-3 py-1.5">
        <button
          type="button"
          onClick={() => setExpanded(!expanded)}
          className="shrink-0 rounded p-0.5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <ChevronRightIcon
            className={cx(
              "size-3 text-muted-foreground/50 transition-transform",
              expanded && "rotate-90"
            )}
          />
        </button>
        <span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
          Group {index + 1}
        </span>
        <span className="text-[10px] text-muted-foreground/40">
          {actions.length} action{actions.length !== 1 ? "s" : ""}
        </span>
        <button
          type="button"
          onClick={onRemove}
          className="ml-auto shrink-0 rounded p-0.5 text-destructive/30 opacity-0 transition-colors group-hover/group:opacity-100 hover:text-destructive focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-label="Remove group"
        >
          <TrashIcon className="size-3" />
        </button>
      </div>

      {expanded ? (
        <div className="space-y-2.5 border-t border-border/30 px-3 pb-3 pt-1">
          <FiltersEditor
            filters={getFilters(group)}
            onChange={(filters) => onChange(setFilters(group, filters))}
            numericOperatorOptions={numericOperatorOptions}
            parseNumericOperator={parseNumericOperator}
          />
          {renderScopeEditor({
            scope: getScope(group),
            onChange: (scope) => onChange(setScope(group, scope)),
          })}
          {renderGroupExtraFields({ group, onChange })}
          <div className="space-y-1">
            <span className="text-[9px] uppercase tracking-wider text-muted-foreground/60">
              Actions
            </span>
            <ActionList
              actions={actions}
              onChange={(nextActions) => onChange(setActions(group, nextActions))}
              createEmptyAction={createEmptyAction}
              renderActionSummary={renderActionSummary}
              renderActionFields={renderActionFields}
              renderScopeEditor={renderScopeEditor}
              getActionAccentColor={getActionAccentColor}
            />
          </div>
        </div>
      ) : null}
    </div>
  );
}

const MODE_TABS: readonly { value: SharedGroundTruthMode; label: string }[] = [
  { value: "text", label: "Text" },
  { value: "flat", label: "Structured" },
  { value: "multiGroup", label: "Multi-Group" },
];

export function SharedGroundTruthEditorShell<
  TAction,
  TGroup,
  TScope,
  TNumericOp extends string,
  TFilters extends SharedExpectedFiltersLike<TNumericOp>,
>({
  mode,
  onModeChange,
  groundTruthText,
  onGroundTruthTextChange,
  textPlaceholder = "Expected final output text for output-matching scoring",
  flat,
  groups,
  createEmptyAction,
  renderScopeEditor,
  renderActionSummary,
  renderActionFields,
  getActionAccentColor,
  numericOperatorOptions,
  parseNumericOperator,
}: {
  readonly mode: SharedGroundTruthMode;
  readonly onModeChange: (mode: SharedGroundTruthMode) => void;
  readonly groundTruthText: string;
  readonly onGroundTruthTextChange: (value: string) => void;
  readonly textPlaceholder?: string;
  readonly flat?: SharedGroundTruthFlatState<TAction, TScope, TFilters, TNumericOp>;
  readonly groups?: SharedGroundTruthGroupState<TGroup, TAction, TScope, TFilters, TNumericOp>;
  readonly createEmptyAction: () => TAction;
  readonly renderScopeEditor: (args: SharedRenderScopeEditorArgs<TScope>) => ReactNode;
  readonly renderActionSummary: (action: TAction) => ReactNode;
  readonly renderActionFields: (args: SharedRenderActionFieldsArgs<TAction, TScope>) => ReactNode;
  readonly getActionAccentColor: (action: TAction) => string;
  readonly numericOperatorOptions: readonly { value: TNumericOp; label: string }[];
  readonly parseNumericOperator: (value: string) => TNumericOp | undefined;
}) {
  return (
    <div className="space-y-3">
      <div className="inline-flex items-center rounded-lg border bg-muted/30 p-0.5">
        {MODE_TABS.map((tab) => (
          <button
            key={tab.value}
            type="button"
            onClick={() => onModeChange(tab.value)}
            className={cx(
              "rounded-md px-3 py-1 text-[11px] font-medium transition-colors",
              mode === tab.value
                ? "bg-background text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground"
            )}
          >
            {tab.label}
          </button>
        ))}
      </div>

      {mode === "text" ? (
        <SharedAutoGrowTextarea
          value={groundTruthText}
          onChange={onGroundTruthTextChange}
          placeholder={textPlaceholder}
          minRows={3}
          maxRows={30}
        />
      ) : null}

      {mode === "flat" && flat ? (
        <div className="space-y-3">
          <ActionList
            actions={flat.actions}
            onChange={flat.onActionsChange}
            createEmptyAction={createEmptyAction}
            renderActionSummary={renderActionSummary}
            renderActionFields={renderActionFields}
            renderScopeEditor={renderScopeEditor}
            getActionAccentColor={getActionAccentColor}
          />
          {renderScopeEditor({ scope: flat.scope, onChange: flat.onScopeChange })}
          <FiltersEditor
            filters={flat.filters}
            onChange={flat.onFiltersChange}
            numericOperatorOptions={numericOperatorOptions}
            parseNumericOperator={parseNumericOperator}
          />
          {flat.targetSection}
        </div>
      ) : null}

      {mode === "multiGroup" && groups ? (
        <div className="space-y-1.5">
          {groups.groups.map((group, index) => (
            <GroupEditorShell
              key={`group-${index}`}
              group={group}
              index={index}
              onChange={(updatedGroup) => {
                const nextGroups = [...groups.groups];
                nextGroups[index] = updatedGroup;
                groups.onGroupsChange(nextGroups);
              }}
              onRemove={() => {
                const nextGroups = groups.groups.filter((_, groupIndex) => groupIndex !== index);
                groups.onGroupsChange(
                  nextGroups.length > 0 ? nextGroups : [groups.createEmptyGroup()]
                );
              }}
              createEmptyAction={createEmptyAction}
              getFilters={groups.getFilters}
              setFilters={groups.setFilters}
              getScope={groups.getScope}
              setScope={groups.setScope}
              getActions={groups.getActions}
              setActions={groups.setActions}
              renderScopeEditor={renderScopeEditor}
              renderActionSummary={renderActionSummary}
              renderActionFields={renderActionFields}
              getActionAccentColor={getActionAccentColor}
              renderGroupExtraFields={groups.renderGroupExtraFields}
              numericOperatorOptions={numericOperatorOptions}
              parseNumericOperator={parseNumericOperator}
            />
          ))}
          <button
            type="button"
            onClick={() => groups.onGroupsChange([...groups.groups, groups.createEmptyGroup()])}
            className="flex items-center gap-1 rounded text-[10px] text-muted-foreground/50 transition-colors hover:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <PlusIcon className="size-2.5" />
            Add group
          </button>
        </div>
      ) : null}
    </div>
  );
}
