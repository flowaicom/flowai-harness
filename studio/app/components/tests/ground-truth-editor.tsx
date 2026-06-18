/**
 * Ground truth editor — supports text-only, flat (structured), and multi-group modes.
 *
 * Visual design: collapsible cards with accent bars,
 * inline pills, truncated UUID chips, and compact density.
 *
 * Component hierarchy:
 *   GroundTruthEditor
 *     ├── Mode tabs: Text | Structured (Flat) | Multi-Group
 *     ├── [text] AutoGrowTextarea
 *     ├── [flat] ActionList + ScopeEditor
 *     └── [multiGroup] GroupList → GroupEditor(s) → ActionList + FiltersEditor + ScopeEditor
 *
 * @module components/tests/ground-truth-editor
 */

import { ChevronRightIcon, PlusIcon, TrashIcon, XIcon } from "lucide-react";
import { type MutableRefObject, useCallback, useEffect, useRef, useState } from "react";
import type {
  ActionPayload,
  ExpectedAction,
  ExpectedFilters,
  ExpectedGroup,
  ExpectedScope,
  GroundTruth,
  GroundTruthComparisonOp,
  GroundTruthMeasureFilter,
  GroundTruthNumericFilter,
} from "~/lib/domain/test-case";
import {
  createEmptyAction,
  createEmptyGroup,
  EMPTY_EXPECTED_FILTERS,
} from "~/lib/domain/test-case";
import { cn } from "~/lib/utils";

// =============================================================================
// Constants
// =============================================================================

const NUMERIC_OPERATORS: { value: GroundTruthComparisonOp; label: string }[] = [
  { value: "greaterThan", label: ">" },
  { value: "greaterThanOrEqual", label: ">=" },
  { value: "lessThan", label: "<" },
  { value: "lessThanOrEqual", label: "<=" },
  { value: "equal", label: "=" },
  { value: "notEqual", label: "!=" },
];

/** Rotating accent colors for action type keylines. */
const ACTION_ACCENTS = [
  "var(--dot-emerald)",
  "var(--dot-amber)",
  "var(--dot-blue)",
  "var(--dot-purple)",
];

function actionAccent(actionType: string): string {
  let hash = 0;
  for (let i = 0; i < actionType.length; i++) {
    hash = (hash * 31 + actionType.charCodeAt(i)) | 0;
  }
  return ACTION_ACCENTS[Math.abs(hash) % ACTION_ACCENTS.length];
}

type GroundTruthMode = "text" | "flat" | "multiGroup";

function modeFromGroundTruth(gt: GroundTruth | null): GroundTruthMode {
  if (!gt) return "text";
  return gt.kind === "textOnly" ? "text" : gt.kind === "flat" ? "flat" : "multiGroup";
}

/** Truncate UUID to first 4 + last 3 chars. */
function truncateId(id: string): string {
  if (id.length <= 12) return id;
  return `${id.slice(0, 4)}…${id.slice(-3)}`;
}

// =============================================================================
// Auto-Growing Textarea
// =============================================================================

function AutoGrowTextarea({
  value,
  onChange,
  placeholder,
  minRows = 3,
  maxRows = 20,
  className,
}: {
  readonly value: string;
  readonly onChange: (v: string) => void;
  readonly placeholder?: string;
  readonly minRows?: number;
  readonly maxRows?: number;
  readonly className?: string;
}) {
  const ref = useRef<HTMLTextAreaElement>(null);
  const resize = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    const lineH = 20;
    const minH = minRows * lineH + 16;
    const maxH = maxRows * lineH + 16;
    el.style.height = `${Math.min(Math.max(el.scrollHeight, minH), maxH)}px`;
  }, [minRows, maxRows]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: value drives re-measurement
  useEffect(resize, [resize, value]);

  return (
    <textarea
      ref={ref}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      className={cn("form-textarea resize-none overflow-y-auto scroll-container", className)}
      style={{ minHeight: `${minRows * 20 + 16}px` }}
    />
  );
}

// =============================================================================
// InlinePills — editable inline pill display (for scope values, IDs, etc.)
// =============================================================================

function InlinePills({
  items,
  onChange,
  placeholder,
  mono,
  truncate: truncateFn,
}: {
  readonly items: string[];
  readonly onChange: (items: string[]) => void;
  readonly placeholder: string;
  readonly mono?: boolean;
  readonly truncate?: (s: string) => string;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  const handleAdd = useCallback(() => {
    const trimmed = draft.trim();
    if (!trimmed) return;
    // Support comma-separated paste
    const newItems = trimmed
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    onChange([...items, ...newItems]);
    setDraft("");
  }, [draft, items, onChange]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" || e.key === ",") {
        e.preventDefault();
        handleAdd();
      }
      if (e.key === "Escape") {
        setEditing(false);
        setDraft("");
      }
      if (e.key === "Backspace" && !draft && items.length > 0) {
        onChange(items.slice(0, -1));
      }
    },
    [handleAdd, draft, items, onChange]
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
      {items.map((item, i) => (
        <span
          key={`${i}-${item}`}
          className={cn(
            "inline-flex items-center gap-0.5 px-1.5 py-px rounded text-[10px] bg-muted/60 text-muted-foreground group/pill",
            mono && "font-mono"
          )}
          title={item}
        >
          {truncateFn ? truncateFn(item) : item}
          <button
            type="button"
            onClick={() => onChange(items.filter((_, j) => j !== i))}
            className="p-px rounded hover:bg-black/5 dark:hover:bg-white/10 opacity-0 group-hover/pill:opacity-100 transition-opacity"
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
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={handleKeyDown}
          onBlur={() => {
            handleAdd();
            if (!draft.trim()) setEditing(false);
          }}
          placeholder={placeholder}
          className={cn(
            "text-[10px] bg-transparent outline-none min-w-[60px] max-w-[120px] px-1 py-0.5",
            mono && "font-mono"
          )}
          // biome-ignore lint/a11y/noAutofocus: intentional — focus newly-revealed inline input
          autoFocus
        />
      ) : (
        <button
          type="button"
          onClick={() => {
            setEditing(true);
            requestAnimationFrame(() => inputRef.current?.focus());
          }}
          className="text-[10px] text-muted-foreground/40 hover:text-muted-foreground transition-colors px-1"
        >
          +
        </button>
      )}
    </div>
  );
}

// =============================================================================
// ScopeEditor — generic key-value scope dimensions
// =============================================================================

function ScopeEditor({
  scope,
  onChange,
}: {
  readonly scope: ExpectedScope | undefined;
  readonly onChange: (s: ExpectedScope | undefined) => void;
}) {
  const entries = scope ? Object.entries(scope) : [];

  return (
    <div className="space-y-1">
      <span className="text-[10px] text-muted-foreground/60 uppercase tracking-wider">Scope</span>
      {entries.map(([dim, values], i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: dynamic form list
        <div key={`scope-${i}`} className="flex items-center gap-1.5">
          <input
            type="text"
            value={dim}
            onChange={(e) => {
              const next = { ...scope };
              delete next[dim];
              if (e.target.value.trim()) next[e.target.value.trim()] = values;
              onChange(Object.keys(next).length > 0 ? next : undefined);
            }}
            placeholder="dimension"
            className="w-24 form-input form-input-sm"
          />
          <InlinePills
            items={values}
            onChange={(list) => {
              if (list.length === 0) {
                const next = { ...scope };
                delete next[dim];
                onChange(Object.keys(next).length > 0 ? next : undefined);
                return;
              }
              onChange({ ...scope, [dim]: list });
            }}
            placeholder="add value"
          />
          <button
            type="button"
            onClick={() => {
              const next = { ...scope };
              delete next[dim];
              onChange(Object.keys(next).length > 0 ? next : undefined);
            }}
            className="p-0.5 text-destructive/30 hover:text-destructive transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none rounded"
          >
            <TrashIcon className="size-3" />
          </button>
        </div>
      ))}
      <button
        type="button"
        onClick={() => onChange({ ...scope, "": [] })}
        className="text-[10px] text-muted-foreground/50 hover:text-muted-foreground flex items-center gap-0.5 transition-colors rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      >
        <PlusIcon className="size-2.5" /> add dimension
      </button>
    </div>
  );
}

// =============================================================================
// ActionEditor — collapsible with summary line
// =============================================================================

function ActionSummary({ action }: { readonly action: ExpectedAction }) {
  const entityCount = (action.entityIds?.length ?? 0) + (action.entityFingerprints?.length ?? 0);
  const scopeDims = action.scope ? Object.keys(action.scope).length : 0;

  // Show the first few payload keys as a compact summary
  const payloadEntries = Object.entries(action.payload).slice(0, 3);

  return (
    <div className="flex items-center gap-1.5 text-[11px] min-w-0 overflow-hidden">
      <span className="font-medium text-foreground shrink-0">
        {action.actionType || "(no type)"}
      </span>
      {payloadEntries.map(([key, val]) => (
        <span key={key} className="text-muted-foreground shrink-0">
          <span className="text-muted-foreground/40">{key}:</span>
          <span className="font-mono tabular-nums ml-0.5">{String(val)}</span>
        </span>
      ))}
      {entityCount > 0 && (
        <span className="tag-badge bg-muted/40 text-muted-foreground text-[9px] shrink-0">
          {entityCount} entity{entityCount !== 1 ? "s" : ""}
        </span>
      )}
      {scopeDims > 0 && (
        <span className="tag-badge bg-muted/40 text-muted-foreground text-[9px] shrink-0">
          {scopeDims} scope dim{scopeDims !== 1 ? "s" : ""}
        </span>
      )}
    </div>
  );
}

function ActionEditor({
  action,
  index,
  onChange,
  onRemove,
}: {
  readonly action: ExpectedAction;
  readonly index: number;
  readonly onChange: (a: ExpectedAction) => void;
  readonly onRemove: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const accentColor = actionAccent(action.actionType);

  return (
    <div
      className="rounded-md bg-muted/5 overflow-hidden"
      style={{ boxShadow: `inset 3px 0 0 ${accentColor}` }}
    >
      {/* Header — always visible */}
      <div className="flex items-center gap-2 px-3 py-1.5 group/action">
        <button
          type="button"
          onClick={() => setExpanded(!expanded)}
          className="p-0.5 shrink-0 rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          aria-label={expanded ? "Collapse" : "Expand"}
        >
          <ChevronRightIcon
            className={cn(
              "size-3 text-muted-foreground/50 transition-transform",
              expanded && "rotate-90"
            )}
          />
        </button>

        <span className="text-[9px] text-muted-foreground/40 tabular-nums w-3 shrink-0">
          {index + 1}
        </span>

        <ActionSummary action={action} />

        <button
          type="button"
          onClick={onRemove}
          className="ml-auto p-0.5 rounded text-destructive/30 hover:text-destructive transition-colors opacity-0 group-hover/action:opacity-100 shrink-0 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none focus-visible:opacity-100"
          aria-label="Remove action"
        >
          <TrashIcon className="size-3" />
        </button>
      </div>

      {/* Expanded form */}
      {expanded && (
        <div className="px-3 pb-3 pt-1 space-y-2 border-t border-border/30">
          {/* Type */}
          <div className="space-y-0.5">
            <span className="text-[9px] text-muted-foreground/60 uppercase tracking-wider">
              Action Type
            </span>
            <input
              type="text"
              value={action.actionType}
              onChange={(e) => onChange({ ...action, actionType: e.target.value })}
              placeholder="e.g. UPDATE_RECORD, SEND_EMAIL"
              className="w-full form-input form-input-sm"
            />
          </div>

          {/* Payload — generic key-value editor */}
          <PayloadEditor
            payload={action.payload}
            onChange={(p) => onChange({ ...action, payload: p })}
          />

          {/* Entity IDs — inline pills with truncated UUIDs */}
          <div className="space-y-0.5">
            <span className="text-[9px] text-muted-foreground/60 uppercase tracking-wider">
              Entity IDs
            </span>
            <InlinePills
              items={action.entityIds ?? []}
              onChange={(ids) =>
                onChange({ ...action, entityIds: ids.length > 0 ? ids : undefined })
              }
              placeholder="add entity ID"
              mono
              truncate={truncateId}
            />
          </div>

          {/* Entity Fingerprints — inline pills */}
          <div className="space-y-0.5">
            <span className="text-[9px] text-muted-foreground/60 uppercase tracking-wider">
              Fingerprints
            </span>
            <InlinePills
              items={action.entityFingerprints ?? []}
              onChange={(fps) =>
                onChange({ ...action, entityFingerprints: fps.length > 0 ? fps : undefined })
              }
              placeholder="add fingerprint"
              mono
            />
          </div>

          {/* Scope */}
          <ScopeEditor scope={action.scope} onChange={(s) => onChange({ ...action, scope: s })} />
        </div>
      )}
    </div>
  );
}

// =============================================================================
// PayloadEditor — generic key-value editor for action payload
// =============================================================================

function PayloadEditor({
  payload,
  onChange,
}: {
  readonly payload: ActionPayload;
  readonly onChange: (p: ActionPayload) => void;
}) {
  const entries = Object.entries(payload);

  return (
    <div className="space-y-1">
      <span className="text-[9px] text-muted-foreground/60 uppercase tracking-wider">Payload</span>
      {entries.map(([key, val], i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: dynamic form list
        <div key={`payload-${i}`} className="flex gap-1.5 items-center">
          <input
            type="text"
            value={key}
            onChange={(e) => {
              const next = { ...payload };
              delete next[key];
              if (e.target.value.trim()) next[e.target.value.trim()] = val;
              onChange(next);
            }}
            placeholder="key"
            className="flex-1 form-input form-input-sm"
          />
          <input
            type="text"
            value={String(val ?? "")}
            onChange={(e) => {
              // Auto-detect numbers
              const num = Number(e.target.value);
              onChange({
                ...payload,
                [key]: !Number.isNaN(num) && e.target.value.trim() !== "" ? num : e.target.value,
              });
            }}
            placeholder="value"
            className="flex-[2] form-input form-input-sm font-mono"
          />
          <button
            type="button"
            onClick={() => {
              const next = { ...payload };
              delete next[key];
              onChange(next);
            }}
            className="p-0.5 text-destructive/30 hover:text-destructive transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none rounded"
          >
            <TrashIcon className="size-3" />
          </button>
        </div>
      ))}
      <button
        type="button"
        onClick={() => onChange({ ...payload, "": "" })}
        className="text-[10px] text-muted-foreground/50 hover:text-muted-foreground flex items-center gap-0.5 transition-colors rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      >
        <PlusIcon className="size-2.5" /> add field
      </button>
    </div>
  );
}

// =============================================================================
// ActionList
// =============================================================================

function createActionRowId(counterRef: MutableRefObject<number>): string {
  const next = counterRef.current;
  counterRef.current += 1;
  return `action-row-${next}`;
}

function findInsertionIndex(
  prevActions: readonly ExpectedAction[],
  nextActions: readonly ExpectedAction[]
) {
  let prevIdx = 0;
  let nextIdx = 0;
  while (prevIdx < prevActions.length && nextIdx < nextActions.length) {
    if (prevActions[prevIdx] !== nextActions[nextIdx]) return nextIdx;
    prevIdx++;
    nextIdx++;
  }
  return nextActions.length - 1;
}

function findRemovalIndex(
  prevActions: readonly ExpectedAction[],
  nextActions: readonly ExpectedAction[]
) {
  let prevIdx = 0;
  let nextIdx = 0;
  while (prevIdx < prevActions.length && nextIdx < nextActions.length) {
    if (prevActions[prevIdx] !== nextActions[nextIdx]) return prevIdx;
    prevIdx++;
    nextIdx++;
  }
  return prevActions.length - 1;
}

function reconcileActionRowIds(
  prevRowIds: readonly string[],
  prevActions: readonly ExpectedAction[],
  nextActions: readonly ExpectedAction[],
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
  return nextActions.map((_, i) => prevRowIds[i] ?? createActionRowId(counterRef));
}

function ActionList({
  actions,
  onChange,
}: {
  readonly actions: ExpectedAction[];
  readonly onChange: (a: ExpectedAction[]) => void;
}) {
  const actionRowCounterRef = useRef(0);
  const actionRowsRef = useRef<{ actions: readonly ExpectedAction[]; rowIds: string[] }>({
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

  const actionRowIds = actionRowsRef.current.rowIds;

  const keyedActions = actions.map((action, index) => ({
    action,
    index,
    key: actionRowIds[index] ?? `action-fallback-${index}-${action.actionType}`,
  }));

  return (
    <div className="space-y-1">
      {keyedActions.map(({ action, index, key }) => (
        <ActionEditor
          key={key}
          action={action}
          index={index}
          onChange={(updated) => {
            const next = [...actions];
            next[index] = updated;
            onChange(next);
          }}
          onRemove={() => onChange(actions.filter((_, j) => j !== index))}
        />
      ))}
      <button
        type="button"
        onClick={() => onChange([...actions, createEmptyAction()])}
        className="flex items-center gap-1 text-[10px] text-muted-foreground/50 hover:text-muted-foreground transition-colors pt-0.5 rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      >
        <PlusIcon className="size-2.5" />
        Add action
      </button>
    </div>
  );
}

// =============================================================================
// FiltersEditor
// =============================================================================

function FiltersEditor({
  filters,
  onChange,
}: {
  readonly filters: ExpectedFilters;
  readonly onChange: (f: ExpectedFilters) => void;
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
        className="flex items-center gap-1.5 text-[10px] text-muted-foreground/60 hover:text-muted-foreground transition-colors uppercase tracking-wider group rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      >
        <ChevronRightIcon className={cn("size-3 transition-transform", expanded && "rotate-90")} />
        Filters
        {filterCount > 0 && (
          <>
            <span className="status-dot bg-[var(--dot-emerald)]" />
            <span className="normal-case tracking-normal text-muted-foreground/40">
              {filterCount}
            </span>
          </>
        )}
      </button>

      {expanded && (
        <div className="pl-3 space-y-3 border-l-2 border-border/30 ml-1.5">
          <KeyValueListEditor
            label="Matched Filters"
            entries={Object.entries(filters.matchedFilters).map(([k, v]) => ({
              key: k,
              value: v.join(", "),
            }))}
            onChange={(entries) => {
              const mf: Record<string, string[]> = {};
              for (const e of entries) {
                if (e.key.trim())
                  mf[e.key.trim()] = e.value
                    .split(",")
                    .map((s) => s.trim())
                    .filter(Boolean);
              }
              onChange({ ...filters, matchedFilters: mf });
            }}
          />
          <NumericFiltersEditor
            filters={filters.numericFilters}
            onChange={(nf) => onChange({ ...filters, numericFilters: nf })}
          />
          <BooleanFiltersEditor
            filters={filters.booleanFilters}
            onChange={(bf) => onChange({ ...filters, booleanFilters: bf })}
          />
          <MeasureFiltersEditor
            filters={filters.measureFilters}
            onChange={(mf) => onChange({ ...filters, measureFilters: mf })}
          />
        </div>
      )}
    </div>
  );
}

// -- Filter sub-editors --

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
      <span className="text-[9px] text-muted-foreground/60 uppercase tracking-wider">{label}</span>
      {entries.map((entry, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: dynamic form list
        <div key={`kv-${i}`} className="flex gap-1.5 items-center">
          <input
            type="text"
            value={entry.key}
            onChange={(e) => {
              const next = [...entries];
              next[i] = { ...entry, key: e.target.value };
              onChange(next);
            }}
            placeholder="column"
            className="flex-1 form-input form-input-sm"
          />
          <input
            type="text"
            value={entry.value}
            onChange={(e) => {
              const next = [...entries];
              next[i] = { ...entry, value: e.target.value };
              onChange(next);
            }}
            placeholder="values"
            className="flex-[2] form-input form-input-sm"
          />
          <button
            type="button"
            onClick={() => onChange(entries.filter((_, j) => j !== i))}
            className="p-0.5 text-destructive/30 hover:text-destructive transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none rounded"
          >
            <TrashIcon className="size-3" />
          </button>
        </div>
      ))}
      <button
        type="button"
        onClick={() => onChange([...entries, { key: "", value: "" }])}
        className="text-[10px] text-muted-foreground/50 hover:text-muted-foreground flex items-center gap-0.5 transition-colors rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      >
        <PlusIcon className="size-2.5" /> add
      </button>
    </div>
  );
}

function NumericFiltersEditor({
  filters,
  onChange,
}: {
  readonly filters: Record<string, GroundTruthNumericFilter>;
  readonly onChange: (f: Record<string, GroundTruthNumericFilter>) => void;
}) {
  const entries = Object.entries(filters);
  return (
    <div className="space-y-1">
      <span className="text-[9px] text-muted-foreground/60 uppercase tracking-wider">
        Numeric Filters
      </span>
      {entries.map(([col, nf], i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: dynamic form list
        <div key={`nf-${i}`} className="flex gap-1.5 items-center">
          <input
            type="text"
            value={col}
            onChange={(e) => {
              const next = { ...filters };
              delete next[col];
              if (e.target.value.trim()) next[e.target.value.trim()] = nf;
              onChange(next);
            }}
            placeholder="column"
            className="flex-1 form-input form-input-sm"
          />
          <select
            value={nf.operator}
            onChange={(e) =>
              onChange({
                ...filters,
                [col]: { ...nf, operator: e.target.value as GroundTruthComparisonOp },
              })
            }
            className="form-select form-input-sm"
          >
            {NUMERIC_OPERATORS.map((op) => (
              <option key={op.value} value={op.value}>
                {op.label}
              </option>
            ))}
          </select>
          <input
            type="number"
            value={nf.value}
            onChange={(e) =>
              onChange({ ...filters, [col]: { ...nf, value: Number(e.target.value) || 0 } })
            }
            className="w-20 form-input form-input-sm font-mono tabular-nums"
          />
          <button
            type="button"
            onClick={() => {
              const next = { ...filters };
              delete next[col];
              onChange(next);
            }}
            className="p-0.5 text-destructive/30 hover:text-destructive transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none rounded"
          >
            <TrashIcon className="size-3" />
          </button>
        </div>
      ))}
      <button
        type="button"
        onClick={() => onChange({ ...filters, "": { operator: "greaterThan", value: 0 } })}
        className="text-[10px] text-muted-foreground/50 hover:text-muted-foreground flex items-center gap-0.5 transition-colors rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
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
  readonly onChange: (f: Record<string, boolean>) => void;
}) {
  const entries = Object.entries(filters);
  return (
    <div className="space-y-1">
      <span className="text-[9px] text-muted-foreground/60 uppercase tracking-wider">
        Boolean Filters
      </span>
      {entries.map(([col, val], i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: dynamic form list
        <div key={`bf-${i}`} className="flex gap-1.5 items-center">
          <input
            type="text"
            value={col}
            onChange={(e) => {
              const next = { ...filters };
              delete next[col];
              if (e.target.value.trim()) next[e.target.value.trim()] = val;
              onChange(next);
            }}
            placeholder="column"
            className="flex-1 form-input form-input-sm"
          />
          <select
            value={val ? "true" : "false"}
            onChange={(e) => onChange({ ...filters, [col]: e.target.value === "true" })}
            className="form-select form-input-sm"
          >
            <option value="true">true</option>
            <option value="false">false</option>
          </select>
          <button
            type="button"
            onClick={() => {
              const next = { ...filters };
              delete next[col];
              onChange(next);
            }}
            className="p-0.5 text-destructive/30 hover:text-destructive transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none rounded"
          >
            <TrashIcon className="size-3" />
          </button>
        </div>
      ))}
      <button
        type="button"
        onClick={() => onChange({ ...filters, "": true })}
        className="text-[10px] text-muted-foreground/50 hover:text-muted-foreground flex items-center gap-0.5 transition-colors rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      >
        <PlusIcon className="size-2.5" /> add
      </button>
    </div>
  );
}

function MeasureFiltersEditor({
  filters,
  onChange,
}: {
  readonly filters: Record<string, GroundTruthMeasureFilter>;
  readonly onChange: (f: Record<string, GroundTruthMeasureFilter>) => void;
}) {
  const entries = Object.entries(filters);
  return (
    <div className="space-y-1">
      <span className="text-[9px] text-muted-foreground/60 uppercase tracking-wider">
        Measure Filters
      </span>
      {entries.map(([key, mf], i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: dynamic form list
        <div key={`mf-${i}`} className="flex gap-1.5 items-center">
          <input
            type="text"
            value={mf.column}
            onChange={(e) => {
              const next = { ...filters };
              next[key] = { ...mf, column: e.target.value };
              onChange(next);
            }}
            placeholder="column"
            className="flex-1 form-input form-input-sm"
          />
          <select
            value={mf.operator}
            onChange={(e) =>
              onChange({
                ...filters,
                [key]: { ...mf, operator: e.target.value as GroundTruthComparisonOp },
              })
            }
            className="form-select form-input-sm"
          >
            {NUMERIC_OPERATORS.map((op) => (
              <option key={op.value} value={op.value}>
                {op.label}
              </option>
            ))}
          </select>
          <input
            type="number"
            value={mf.value}
            onChange={(e) =>
              onChange({ ...filters, [key]: { ...mf, value: Number(e.target.value) || 0 } })
            }
            className="w-20 form-input form-input-sm font-mono tabular-nums"
          />
          <button
            type="button"
            onClick={() => {
              const next = { ...filters };
              delete next[key];
              onChange(next);
            }}
            className="p-0.5 text-destructive/30 hover:text-destructive transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none rounded"
          >
            <TrashIcon className="size-3" />
          </button>
        </div>
      ))}
      <button
        type="button"
        onClick={() => {
          const maxIdx = entries.reduce((max, [k]) => {
            const m = k.match(/^measure_(\d+)$/);
            return m ? Math.max(max, Number(m[1])) : max;
          }, -1);
          const key = `measure_${maxIdx + 1}`;
          onChange({ ...filters, [key]: { column: "", operator: "greaterThan", value: 0 } });
        }}
        className="text-[10px] text-muted-foreground/50 hover:text-muted-foreground flex items-center gap-0.5 transition-colors rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      >
        <PlusIcon className="size-2.5" /> add
      </button>
    </div>
  );
}

// =============================================================================
// GroupEditor
// =============================================================================

function GroupEditor({
  group,
  index,
  onChange,
  onRemove,
}: {
  readonly group: ExpectedGroup;
  readonly index: number;
  readonly onChange: (g: ExpectedGroup) => void;
  readonly onRemove: () => void;
}) {
  const [expanded, setExpanded] = useState(true);
  const actionCount = group.actions.length;

  return (
    <div
      className="rounded-md bg-muted/5 overflow-hidden"
      style={{ boxShadow: "inset 3px 0 0 var(--dot-purple)" }}
    >
      {/* Header */}
      <div className="flex items-center gap-2 px-3 py-1.5 group/group">
        <button
          type="button"
          onClick={() => setExpanded(!expanded)}
          className="p-0.5 shrink-0 rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
        >
          <ChevronRightIcon
            className={cn(
              "size-3 text-muted-foreground/50 transition-transform",
              expanded && "rotate-90"
            )}
          />
        </button>
        <span className="text-[10px] font-medium text-muted-foreground uppercase tracking-wider">
          Group {index + 1}
        </span>
        <span className="text-[10px] text-muted-foreground/40">
          {actionCount} action{actionCount !== 1 ? "s" : ""}
        </span>
        <button
          type="button"
          onClick={onRemove}
          className="ml-auto p-0.5 rounded text-destructive/30 hover:text-destructive transition-colors opacity-0 group-hover/group:opacity-100 shrink-0 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none focus-visible:opacity-100"
          aria-label="Remove group"
        >
          <TrashIcon className="size-3" />
        </button>
      </div>

      {expanded && (
        <div className="px-3 pb-3 pt-1 space-y-2.5 border-t border-border/30">
          <FiltersEditor
            filters={group.filters}
            onChange={(f) => onChange({ ...group, filters: f })}
          />
          <ScopeEditor scope={group.scope} onChange={(s) => onChange({ ...group, scope: s })} />

          {/* Entity Description — compact */}
          <div className="flex items-center gap-2">
            <span className="text-[10px] text-muted-foreground/60 shrink-0">Description</span>
            <input
              type="text"
              value={group.entityDescription ?? ""}
              onChange={(e) =>
                onChange({ ...group, entityDescription: e.target.value || undefined })
              }
              placeholder="e.g. High-priority entities"
              className="flex-1 form-input form-input-sm"
            />
          </div>

          {/* Entity SQL — compact */}
          <div className="flex items-start gap-2">
            <span className="text-[10px] text-muted-foreground/60 shrink-0 pt-1">SQL</span>
            <input
              type="text"
              value={group.entitySql ?? ""}
              onChange={(e) => onChange({ ...group, entitySql: e.target.value || undefined })}
              placeholder="SELECT id FROM ..."
              className="flex-1 form-input form-input-sm font-mono"
            />
          </div>

          <div className="space-y-1">
            <span className="text-[9px] text-muted-foreground/60 uppercase tracking-wider">
              Actions
            </span>
            <ActionList
              actions={group.actions}
              onChange={(a) => onChange({ ...group, actions: a })}
            />
          </div>
        </div>
      )}
    </div>
  );
}

// =============================================================================
// EntityGroundTruth — collapsible with indicator dot
// =============================================================================

function EntityGroundTruth({
  groundTruthEntityIds,
  groundTruthSql,
  groundTruthEntityFingerprints,
  onChange,
}: {
  readonly groundTruthEntityIds: string[] | undefined;
  readonly groundTruthSql: string | undefined;
  readonly groundTruthEntityFingerprints: string[] | undefined;
  readonly onChange: (patch: {
    groundTruthEntityIds?: string[];
    groundTruthSql?: string;
    groundTruthEntityFingerprints?: string[];
  }) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const hasContent =
    (groundTruthEntityIds && groundTruthEntityIds.length > 0) ||
    (groundTruthEntityFingerprints && groundTruthEntityFingerprints.length > 0) ||
    !!groundTruthSql;

  const entityCount =
    (groundTruthEntityIds?.length ?? 0) + (groundTruthEntityFingerprints?.length ?? 0);

  return (
    <div className="space-y-1.5">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="flex items-center gap-1.5 text-[10px] text-muted-foreground/60 hover:text-muted-foreground transition-colors uppercase tracking-wider rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      >
        <ChevronRightIcon className={cn("size-3 transition-transform", expanded && "rotate-90")} />
        Entity Ground Truth
        {hasContent && (
          <>
            <span className="status-dot bg-[var(--dot-emerald)]" />
            {entityCount > 0 && (
              <span className="normal-case tracking-normal text-muted-foreground/40">
                {entityCount} entit{entityCount !== 1 ? "ies" : "y"}
              </span>
            )}
          </>
        )}
      </button>

      {expanded && (
        <div className="pl-3 space-y-2 border-l-2 border-border/30 ml-1.5">
          {/* Expected Entity IDs */}
          <div className="space-y-0.5">
            <span className="text-[9px] text-muted-foreground/60 uppercase tracking-wider">
              Expected Entity IDs
            </span>
            <InlinePills
              items={groundTruthEntityIds ?? []}
              onChange={(ids) =>
                onChange({ groundTruthEntityIds: ids.length > 0 ? ids : undefined })
              }
              placeholder="add entity ID"
              mono
              truncate={truncateId}
            />
          </div>

          {/* Expected Fingerprints */}
          <div className="space-y-0.5">
            <span className="text-[9px] text-muted-foreground/60 uppercase tracking-wider">
              Expected Fingerprints
            </span>
            <InlinePills
              items={groundTruthEntityFingerprints ?? []}
              onChange={(fps) =>
                onChange({ groundTruthEntityFingerprints: fps.length > 0 ? fps : undefined })
              }
              placeholder="add fingerprint"
              mono
            />
          </div>

          {/* Expected SQL */}
          <div className="space-y-0.5">
            <span className="text-[9px] text-muted-foreground/60 uppercase tracking-wider">
              Expected SQL Query
            </span>
            <textarea
              value={groundTruthSql ?? ""}
              onChange={(e) => onChange({ groundTruthSql: e.target.value || undefined })}
              placeholder="SELECT id FROM your_table WHERE ..."
              rows={2}
              className="form-textarea form-input-sm font-mono"
            />
          </div>
        </div>
      )}
    </div>
  );
}

// =============================================================================
// GroundTruthEditor (main export)
// =============================================================================

export interface GroundTruthEditorProps {
  readonly groundTruthText: string;
  readonly onGroundTruthTextChange: (v: string) => void;
  readonly structuredGroundTruth: GroundTruth | null;
  readonly onStructuredGroundTruthChange: (v: GroundTruth | null) => void;
}

const MODE_TABS: { value: GroundTruthMode; label: string }[] = [
  { value: "text", label: "Text" },
  { value: "flat", label: "Structured" },
  { value: "multiGroup", label: "Multi-Group" },
];

export function GroundTruthEditor({
  groundTruthText,
  onGroundTruthTextChange,
  structuredGroundTruth,
  onStructuredGroundTruthChange,
}: GroundTruthEditorProps) {
  const [mode, setMode] = useState<GroundTruthMode>(() =>
    modeFromGroundTruth(structuredGroundTruth)
  );

  // Cache structured GT so switching to text mode and back doesn't lose work
  const cachedStructuredGt = useRef<GroundTruth | null>(structuredGroundTruth);
  // Keep cache in sync with external changes (e.g., prop updates from parent)
  useEffect(() => {
    if (structuredGroundTruth) cachedStructuredGt.current = structuredGroundTruth;
  }, [structuredGroundTruth]);

  useEffect(() => {
    setMode(modeFromGroundTruth(structuredGroundTruth));
  }, [structuredGroundTruth]);

  const handleModeChange = useCallback(
    (newMode: GroundTruthMode) => {
      setMode(newMode);

      if (newMode === "text") {
        // Cache before nullifying — restore on switch-back
        if (structuredGroundTruth) cachedStructuredGt.current = structuredGroundTruth;
        onStructuredGroundTruthChange(null);
      } else if (newMode === "flat") {
        if (structuredGroundTruth?.kind === "flat") return;
        // Restore from cache if it was flat, otherwise create fresh
        const cached = cachedStructuredGt.current;
        onStructuredGroundTruthChange(
          cached?.kind === "flat"
            ? cached
            : {
                kind: "flat",
                expectedActions: [createEmptyAction()],
                expectedFilters: { ...EMPTY_EXPECTED_FILTERS },
                expectedScope: {},
              }
        );
      } else {
        if (structuredGroundTruth?.kind === "multiGroup") return;
        // Restore from cache if it was multiGroup, otherwise create fresh
        const cached = cachedStructuredGt.current;
        onStructuredGroundTruthChange(
          cached?.kind === "multiGroup"
            ? cached
            : {
                kind: "multiGroup",
                groups: [createEmptyGroup()],
              }
        );
      }
    },
    [structuredGroundTruth, onStructuredGroundTruthChange]
  );

  return (
    <div className="space-y-3">
      {/* Mode tabs — pill toggle */}
      <div className="inline-flex items-center rounded-lg border bg-muted/30 p-0.5">
        {MODE_TABS.map((tab) => (
          <button
            key={tab.value}
            type="button"
            onClick={() => handleModeChange(tab.value)}
            className={cn(
              "px-3 py-1 rounded-md text-[11px] font-medium transition-colors",
              mode === tab.value
                ? "bg-background text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground"
            )}
          >
            {tab.label}
          </button>
        ))}
      </div>

      {/* Text mode */}
      {mode === "text" && (
        <AutoGrowTextarea
          value={groundTruthText}
          onChange={onGroundTruthTextChange}
          placeholder="Expected final output text for output-matching scoring"
          minRows={3}
          maxRows={30}
        />
      )}

      {/* Flat (structured) mode */}
      {mode === "flat" && structuredGroundTruth?.kind === "flat" && (
        <div className="space-y-3">
          <ActionList
            actions={structuredGroundTruth.expectedActions}
            onChange={(actions) =>
              onStructuredGroundTruthChange({ ...structuredGroundTruth, expectedActions: actions })
            }
          />
          <ScopeEditor
            scope={
              Object.keys(structuredGroundTruth.expectedScope).length > 0
                ? structuredGroundTruth.expectedScope
                : undefined
            }
            onChange={(s) =>
              onStructuredGroundTruthChange({
                ...structuredGroundTruth,
                expectedScope: s ?? {},
              })
            }
          />
          <FiltersEditor
            filters={structuredGroundTruth.expectedFilters}
            onChange={(f) =>
              onStructuredGroundTruthChange({ ...structuredGroundTruth, expectedFilters: f })
            }
          />
          <EntityGroundTruth
            groundTruthEntityIds={structuredGroundTruth.groundTruthEntityIds}
            groundTruthSql={structuredGroundTruth.groundTruthSql}
            groundTruthEntityFingerprints={structuredGroundTruth.groundTruthEntityFingerprints}
            onChange={(patch) =>
              onStructuredGroundTruthChange({ ...structuredGroundTruth, ...patch })
            }
          />
        </div>
      )}

      {/* Multi-Group mode */}
      {mode === "multiGroup" && structuredGroundTruth?.kind === "multiGroup" && (
        <div className="space-y-1.5">
          {structuredGroundTruth.groups.map((group, i) => (
            <GroupEditor
              // biome-ignore lint/suspicious/noArrayIndexKey: dynamic form list
              key={`group-${i}`}
              group={group}
              index={i}
              onChange={(updated) => {
                const next = [...structuredGroundTruth.groups];
                next[i] = updated;
                onStructuredGroundTruthChange({ ...structuredGroundTruth, groups: next });
              }}
              onRemove={() => {
                const next = structuredGroundTruth.groups.filter((_, j) => j !== i);
                onStructuredGroundTruthChange({
                  ...structuredGroundTruth,
                  groups: next.length > 0 ? next : [createEmptyGroup()],
                });
              }}
            />
          ))}
          <button
            type="button"
            onClick={() =>
              onStructuredGroundTruthChange({
                ...structuredGroundTruth,
                groups: [...structuredGroundTruth.groups, createEmptyGroup()],
              })
            }
            className="flex items-center gap-1 text-[10px] text-muted-foreground/50 hover:text-muted-foreground transition-colors rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          >
            <PlusIcon className="size-2.5" />
            Add group
          </button>
        </div>
      )}
    </div>
  );
}
