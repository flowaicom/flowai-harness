import { ChevronDownIcon, ChevronRightIcon, GripVerticalIcon, XIcon } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { SharedTestCaseStatus, SharedTrajectoryMode } from "./domain";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

const CATEGORY_COLORS: Record<
  string,
  {
    readonly bg: string;
    readonly text: string;
    readonly border: string;
    readonly hoverBorder: string;
  }
> = {
  discovery: {
    bg: "bg-[var(--accent-blue)]",
    text: "text-[var(--dot-blue)]",
    border: "border-[var(--dot-blue)]/25",
    hoverBorder: "hover:border-[var(--dot-blue)]/50",
  },
  planning: {
    bg: "bg-[var(--accent-purple)]",
    text: "text-[var(--dot-purple)]",
    border: "border-[var(--dot-purple)]/25",
    hoverBorder: "hover:border-[var(--dot-purple)]/50",
  },
  execution: {
    bg: "bg-[var(--accent-amber)]",
    text: "text-[var(--dot-amber)]",
    border: "border-[var(--dot-amber)]/25",
    hoverBorder: "hover:border-[var(--dot-amber)]/50",
  },
  knowledge: {
    bg: "bg-[var(--accent-emerald)]",
    text: "text-[var(--dot-emerald)]",
    border: "border-[var(--dot-emerald)]/25",
    hoverBorder: "hover:border-[var(--dot-emerald)]/50",
  },
};

const FALLBACK_CATEGORY_COLOR = {
  bg: "bg-primary/8",
  text: "text-primary",
  border: "border-primary/20",
  hoverBorder: "hover:border-primary/40",
};

function categoryColor(category: string | undefined) {
  return (category && CATEGORY_COLORS[category]) || FALLBACK_CATEGORY_COLOR;
}

function TestSectionCard({
  children,
  className,
}: {
  readonly children: ReactNode;
  readonly className?: string;
}) {
  return (
    <div className={cx("space-y-3 rounded-lg border border-border/50 p-4", className)}>
      {children}
    </div>
  );
}

function TestSectionHeader({
  children,
  className,
}: {
  readonly children: ReactNode;
  readonly className?: string;
}) {
  return <h3 className={cx("section-label", className)}>{children}</h3>;
}

const CATALOG_DRAG_MIME = "application/x-tool-name";

const TEST_STATUS_DOT_CLASS: Record<SharedTestCaseStatus, string> = {
  draft: "bg-muted-foreground/50",
  active: "bg-[var(--dot-emerald)]",
  archived: "bg-[var(--dot-amber)]",
};

const TRAJECTORY_MODES: readonly {
  readonly value: SharedTrajectoryMode;
  readonly label: string;
  readonly hint: string;
}[] = [
  { value: "strict", label: "Strict sequence", hint: "same tools, same order, no extras" },
  { value: "unordered", label: "Unordered exact", hint: "same tools, order ignored" },
  { value: "subset", label: "Actual subset", hint: "actual tools contained in expected" },
  { value: "superset", label: "Required present", hint: "expected tools present; extras allowed" },
  {
    value: "subsequence",
    label: "Required in order",
    hint: "expected tools in order; extras allowed",
  },
];

const DEFAULT_CATEGORY_ORDER = ["discovery", "planning", "execution", "knowledge", "delegation"];

function AutoGrowTextarea({
  value,
  onChange,
  placeholder,
  minRows,
  maxRows,
  className,
  id,
}: {
  readonly value: string;
  readonly onChange: (value: string) => void;
  readonly placeholder?: string;
  readonly minRows?: number;
  readonly maxRows?: number;
  readonly className?: string;
  readonly id?: string;
}) {
  const ref = useRef<HTMLTextAreaElement>(null);
  const min = minRows ?? 3;
  const max = maxRows ?? 20;

  const resize = useCallback(() => {
    const element = ref.current;
    if (!element) return;
    element.style.height = "auto";
    const lineHeight = 20;
    const minHeight = min * lineHeight + 16;
    const maxHeight = max * lineHeight + 16;
    element.style.height = `${Math.min(Math.max(element.scrollHeight, minHeight), maxHeight)}px`;
  }, [max, min]);

  useEffect(resize, [resize, value]);

  return (
    <textarea
      ref={ref}
      id={id}
      value={value}
      onChange={(event) => onChange(event.target.value)}
      placeholder={placeholder}
      className={cx("form-textarea scroll-container resize-none overflow-y-auto", className)}
      style={{ minHeight: `${min * 20 + 16}px` }}
    />
  );
}

interface SharedTestCaseFormToolLike {
  readonly name: string;
  readonly description: string;
  readonly category: string;
}

interface TrajectoryTagProps {
  readonly name: string;
  readonly index: number;
  readonly category?: string;
  readonly isInvalid: boolean;
  readonly showOrder: boolean;
  readonly isDragging: boolean;
  readonly isDropTarget: boolean;
  readonly onRemove: () => void;
  readonly onDragStart: (index: number) => void;
  readonly onDragOver: (index: number) => void;
  readonly onDragEnd: () => void;
}

function TrajectoryTag({
  name,
  index,
  category,
  isInvalid,
  showOrder,
  isDragging,
  isDropTarget,
  onRemove,
  onDragStart,
  onDragOver,
  onDragEnd,
}: TrajectoryTagProps) {
  const color = categoryColor(category);

  return (
    <div
      role="option"
      aria-selected={isDragging}
      tabIndex={0}
      draggable
      onDragStart={(event) => {
        event.dataTransfer.effectAllowed = "move";
        onDragStart(index);
      }}
      onDragOver={(event) => {
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
        onDragOver(index);
      }}
      onDragEnd={onDragEnd}
      className={cx(
        "group inline-flex cursor-grab select-none items-center gap-1 rounded-md border pl-1 pr-0.5 py-0.5 text-xs font-mono transition-all active:cursor-grabbing",
        isInvalid
          ? "border-destructive/30 bg-destructive/10 text-destructive"
          : `${color.bg} ${color.text} ${color.border} ${color.hoverBorder}`,
        isDragging && "scale-95 opacity-40",
        isDropTarget && "ring-2 ring-primary/40 ring-offset-1"
      )}
    >
      <GripVerticalIcon className="size-3 shrink-0 opacity-40" />
      {showOrder ? (
        <span className="w-3 shrink-0 text-center text-[9px] tabular-nums opacity-50">
          {index + 1}
        </span>
      ) : null}
      <span className="px-0.5">{name}</span>
      <button
        type="button"
        onClick={(event) => {
          event.stopPropagation();
          onRemove();
        }}
        className="rounded p-0.5 opacity-0 transition-colors group-hover:opacity-100 hover:bg-black/5 dark:hover:bg-white/10"
        aria-label={`Remove ${name}`}
      >
        <XIcon className="size-2.5" />
      </button>
    </div>
  );
}

function TrajectoryEditor({
  steps,
  invalidSet,
  showOrder,
  toolCategoryMap,
  onChange,
}: {
  readonly steps: string[];
  readonly invalidSet: Set<string>;
  readonly showOrder: boolean;
  readonly toolCategoryMap: Map<string, string>;
  readonly onChange: (steps: string[]) => void;
}) {
  const [dragIndex, setDragIndex] = useState<number | null>(null);
  const [dropIndex, setDropIndex] = useState<number | null>(null);
  const [dropZoneActive, setDropZoneActive] = useState(false);

  const handleDragStart = useCallback((index: number) => {
    setDragIndex(index);
  }, []);

  const handleDragOver = useCallback(
    (index: number) => {
      if (dragIndex === null || dragIndex === index) {
        setDropIndex(null);
        return;
      }
      setDropIndex(index);
    },
    [dragIndex]
  );

  const handleDragEnd = useCallback(() => {
    if (dragIndex !== null && dropIndex !== null && dragIndex !== dropIndex) {
      const next = [...steps];
      const [moved] = next.splice(dragIndex, 1);
      next.splice(dropIndex, 0, moved);
      onChange(next);
    }
    setDragIndex(null);
    setDropIndex(null);
    setDropZoneActive(false);
  }, [dragIndex, dropIndex, onChange, steps]);

  const handleRemove = useCallback(
    (index: number) => {
      onChange(steps.filter((_, current) => current !== index));
    },
    [onChange, steps]
  );

  const handleZoneDrop = useCallback(
    (event: React.DragEvent) => {
      event.preventDefault();
      setDropZoneActive(false);
      const toolName = event.dataTransfer.getData(CATALOG_DRAG_MIME);
      if (toolName) {
        onChange([...steps, toolName]);
      }
    },
    [onChange, steps]
  );

  const handleZoneDragOver = useCallback((event: React.DragEvent) => {
    event.preventDefault();
    if (event.dataTransfer.types.includes(CATALOG_DRAG_MIME)) {
      event.dataTransfer.dropEffect = "copy";
      setDropZoneActive(true);
    }
  }, []);

  const handleZoneDragLeave = useCallback(() => {
    setDropZoneActive(false);
  }, []);

  if (steps.length === 0) {
    return (
      <div
        role="listbox"
        aria-label="Trajectory steps — empty"
        className={cx(
          "rounded-md border border-dashed px-3 py-4 text-center text-xs text-muted-foreground/60 transition-colors",
          dropZoneActive && "border-primary bg-primary/5 text-primary/60"
        )}
        onDrop={handleZoneDrop}
        onDragOver={handleZoneDragOver}
        onDragLeave={handleZoneDragLeave}
      >
        {dropZoneActive ? "Drop to add" : "Click or drag tools from the catalog below"}
      </div>
    );
  }

  return (
    <div
      role="listbox"
      aria-label="Trajectory steps"
      className={cx(
        "flex min-h-[44px] flex-wrap gap-1.5 rounded-md border bg-muted/30 px-2.5 py-2 transition-colors",
        dropZoneActive && "border-primary/50 bg-primary/5"
      )}
      onDragOver={handleZoneDragOver}
      onDragLeave={handleZoneDragLeave}
      onDrop={handleZoneDrop}
    >
      {steps.map((name, index) => (
        <TrajectoryTag
          key={`${index}-${name}`}
          name={name}
          index={index}
          category={toolCategoryMap.get(name)}
          isInvalid={invalidSet.has(name)}
          showOrder={showOrder}
          isDragging={dragIndex === index}
          isDropTarget={dropIndex === index}
          onRemove={() => handleRemove(index)}
          onDragStart={handleDragStart}
          onDragOver={handleDragOver}
          onDragEnd={handleDragEnd}
        />
      ))}
      {dropZoneActive ? (
        <span className="inline-flex items-center rounded-md border border-dashed border-primary/40 px-2 py-0.5 text-[10px] text-primary/50">
          + drop here
        </span>
      ) : null}
    </div>
  );
}

function CatalogChip({
  tool,
  onAppend,
}: {
  readonly tool: SharedTestCaseFormToolLike;
  readonly onAppend: (name: string) => void;
}) {
  const color = categoryColor(tool.category);

  return (
    <button
      type="button"
      draggable
      onClick={() => onAppend(tool.name)}
      onDragStart={(event) => {
        event.dataTransfer.effectAllowed = "copy";
        event.dataTransfer.setData(CATALOG_DRAG_MIME, tool.name);
      }}
      className={cx(
        "cursor-grab rounded border px-1.5 py-0.5 text-[11px] font-mono leading-tight transition-colors active:cursor-grabbing",
        color.bg,
        color.text,
        color.border,
        color.hoverBorder
      )}
      title={`${tool.description} — click or drag to add`}
    >
      {tool.name}
    </button>
  );
}

function ToolPalette({
  tools,
  onAppend,
  initiallyExpanded = false,
}: {
  readonly tools: readonly SharedTestCaseFormToolLike[];
  readonly onAppend: (name: string) => void;
  readonly initiallyExpanded?: boolean;
}) {
  const [expanded, setExpanded] = useState(initiallyExpanded);

  const grouped = useMemo(() => {
    const map: Record<string, SharedTestCaseFormToolLike[]> = {};
    for (const tool of tools) {
      if (!map[tool.category]) {
        map[tool.category] = [];
      }
      map[tool.category].push(tool);
    }
    return map;
  }, [tools]);

  const categories = useMemo(() => {
    const known = DEFAULT_CATEGORY_ORDER.filter((category) => grouped[category]?.length > 0);
    const rest = Object.keys(grouped)
      .filter((category) => !DEFAULT_CATEGORY_ORDER.includes(category))
      .sort();
    return [...known, ...rest];
  }, [grouped]);

  if (categories.length === 0) {
    return null;
  }

  return (
    <div className="space-y-1.5">
      <button
        type="button"
        onClick={() => setExpanded((previous) => !previous)}
        className="flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
      >
        {expanded ? (
          <ChevronDownIcon className="size-3" />
        ) : (
          <ChevronRightIcon className="size-3" />
        )}
        Tool Catalog ({tools.length})
      </button>
      {expanded ? (
        <div className="space-y-2.5 pl-1">
          {categories.map((category) => {
            const color = categoryColor(category);
            return (
              <div key={category}>
                <span
                  className={cx(
                    "text-[10px] font-medium uppercase tracking-wider opacity-70",
                    color.text
                  )}
                >
                  {category}
                </span>
                <div className="mt-0.5 flex flex-wrap gap-1">
                  {grouped[category].map((tool) => (
                    <CatalogChip key={tool.name} tool={tool} onAppend={onAppend} />
                  ))}
                </div>
              </div>
            );
          })}
          <p className="pl-0.5 text-[9px] text-muted-foreground/40">
            Click to append or drag into the trajectory
          </p>
        </div>
      ) : null}
    </div>
  );
}

function InlineTagChips({
  tags,
  onChange,
}: {
  readonly tags: string;
  readonly onChange: (value: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  const tagList = useMemo(
    () =>
      tags
        .split(",")
        .map((tag) => tag.trim())
        .filter(Boolean),
    [tags]
  );

  const handleAdd = useCallback(() => {
    const trimmed = draft.trim();
    if (!trimmed) return;
    const newTags = trimmed
      .split(",")
      .map((value) => value.trim())
      .filter(Boolean)
      .slice(0, 20)
      .map((value) => value.slice(0, 50));
    onChange([...tagList, ...newTags].join(", "));
    setDraft("");
  }, [draft, onChange, tagList]);

  const handleRemove = useCallback(
    (index: number) => {
      onChange(tagList.filter((_, current) => current !== index).join(", "));
    },
    [onChange, tagList]
  );

  const handleKeyDown = useCallback(
    (event: React.KeyboardEvent) => {
      if (event.key === "Enter" || event.key === ",") {
        event.preventDefault();
        handleAdd();
      }
      if (event.key === "Escape") {
        setEditing(false);
        setDraft("");
      }
      if (event.key === "Backspace" && !draft && tagList.length > 0) {
        handleRemove(tagList.length - 1);
      }
    },
    [draft, handleAdd, handleRemove, tagList.length]
  );

  if (tagList.length === 0 && !editing) {
    return (
      <button
        type="button"
        onClick={() => {
          setEditing(true);
          requestAnimationFrame(() => inputRef.current?.focus());
        }}
        className="text-[10px] text-muted-foreground/40 transition-colors hover:text-muted-foreground"
      >
        + add tags
      </button>
    );
  }

  return (
    <div className="flex min-w-0 flex-wrap items-center gap-1">
      {tagList.map((tag, index) => (
        <span
          key={`${index}-${tag}`}
          className="group/tag inline-flex items-center gap-0.5 rounded bg-muted/60 px-1.5 py-px text-[10px] text-muted-foreground"
        >
          {tag}
          <button
            type="button"
            onClick={() => handleRemove(index)}
            className="rounded p-px opacity-0 transition-opacity group-hover/tag:opacity-100 hover:bg-black/5 dark:hover:bg-white/10"
            aria-label={`Remove tag ${tag}`}
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
            if (!draft.trim()) {
              setEditing(false);
            }
          }}
          placeholder="tag name"
          className="min-w-[50px] max-w-[100px] bg-transparent px-1 py-0.5 text-[10px] outline-none"
          autoFocus
        />
      ) : (
        <button
          type="button"
          onClick={() => {
            setEditing(true);
            requestAnimationFrame(() => inputRef.current?.focus());
          }}
          className="px-0.5 text-[10px] text-muted-foreground/30 transition-colors hover:text-muted-foreground"
        >
          +
        </button>
      )}
    </div>
  );
}

export interface SharedTestCaseFormIdentity {
  readonly name: string;
  readonly onNameChange: (value: string) => void;
  readonly nameReadOnly?: boolean;
  readonly description: string;
  readonly onDescriptionChange: (value: string) => void;
}

export interface SharedTestCaseFormProps {
  readonly identity?: SharedTestCaseFormIdentity;
  readonly input: string;
  readonly onInputChange: (value: string) => void;
  readonly inputPlaceholder?: string;
  readonly trajectory: string;
  readonly onTrajectoryChange: (value: string) => void;
  readonly mode: SharedTrajectoryMode;
  readonly onModeChange: (value: SharedTrajectoryMode) => void;
  readonly tags: string;
  readonly onTagsChange: (value: string) => void;
  readonly status: SharedTestCaseStatus;
  readonly onStatusChange: (value: SharedTestCaseStatus) => void;
  readonly showStatus?: boolean;
  readonly availableTools: readonly SharedTestCaseFormToolLike[];
  readonly groundTruthEditor: ReactNode;
  readonly toolCatalogUnavailableMessage?: string;
  readonly toolCatalogInitiallyExpanded?: boolean;
}

export function SharedTestCaseForm({
  identity,
  input,
  onInputChange,
  inputPlaceholder = "e.g., What is the average response time?",
  trajectory,
  onTrajectoryChange,
  mode,
  onModeChange,
  tags,
  onTagsChange,
  status,
  onStatusChange,
  showStatus = true,
  availableTools,
  groundTruthEditor,
  toolCatalogUnavailableMessage = "Tool catalog unavailable — trajectory validation disabled",
  toolCatalogInitiallyExpanded = false,
}: SharedTestCaseFormProps) {
  const toolNameSet = useMemo(
    () => new Set(availableTools.map((tool) => tool.name)),
    [availableTools]
  );
  const toolCategoryMap = useMemo(() => {
    const map = new Map<string, string>();
    for (const tool of availableTools) {
      map.set(tool.name, tool.category);
    }
    return map;
  }, [availableTools]);

  const trajectorySteps = useMemo(
    () =>
      trajectory
        .split(/\r?\n/)
        .map((step) => step.trim())
        .filter(Boolean),
    [trajectory]
  );

  const invalidTools = useMemo(() => {
    if (toolNameSet.size === 0) {
      return new Set<string>();
    }
    return new Set(trajectorySteps.filter((step) => !toolNameSet.has(step)));
  }, [toolNameSet, trajectorySteps]);

  const handleStepsChange = useCallback(
    (steps: string[]) => {
      onTrajectoryChange(steps.join("\n"));
    },
    [onTrajectoryChange]
  );

  const handleAppendTool = useCallback(
    (name: string) => {
      onTrajectoryChange([...trajectorySteps, name].join("\n"));
    },
    [onTrajectoryChange, trajectorySteps]
  );

  const [manualTool, setManualTool] = useState("");
  const handleManualAdd = useCallback(() => {
    const name = manualTool.trim();
    if (!name) {
      return;
    }
    handleAppendTool(name);
    setManualTool("");
  }, [handleAppendTool, manualTool]);

  const showOrder = mode === "strict" || mode === "subsequence";

  return (
    <div className="space-y-4">
      {identity ? (
        <TestSectionCard>
          <TestSectionHeader>Identity</TestSectionHeader>
          <div className="space-y-2">
            <input
              id="name"
              type="text"
              value={identity.name}
              onChange={(event) => identity.onNameChange(event.target.value.slice(0, 120))}
              placeholder={identity.nameReadOnly ? "" : "Test case name"}
              readOnly={identity.nameReadOnly}
              disabled={identity.nameReadOnly}
              title={
                identity.nameReadOnly
                  ? "The name is the test-case id and cannot be changed"
                  : undefined
              }
              className={cx(
                "form-input w-full text-sm",
                identity.nameReadOnly && "cursor-not-allowed opacity-70"
              )}
            />
            <input
              id="description"
              type="text"
              value={identity.description}
              onChange={(event) => identity.onDescriptionChange(event.target.value.slice(0, 500))}
              placeholder="Optional description"
              className="form-input w-full text-sm text-muted-foreground"
            />
          </div>
        </TestSectionCard>
      ) : null}

      <TestSectionCard>
        <TestSectionHeader>User Input</TestSectionHeader>
        <AutoGrowTextarea
          id="input"
          value={input}
          onChange={(value) => onInputChange(value.slice(0, 10_000))}
          placeholder={inputPlaceholder}
          minRows={3}
          maxRows={10}
        />
        <div
          className={cx(
            "text-right text-[10px] tabular-nums",
            input.length >= 10_000 ? "text-destructive" : "text-muted-foreground/60"
          )}
        >
          {input.length.toLocaleString()} / 10,000
        </div>
      </TestSectionCard>

      <TestSectionCard>
        <div className="flex items-center justify-between">
          <TestSectionHeader>Expected Trajectory</TestSectionHeader>
          <div className="flex items-center gap-2">
            {trajectorySteps.length > 0 ? (
              <span className="text-[10px] tabular-nums text-muted-foreground">
                {trajectorySteps.length} step{trajectorySteps.length !== 1 ? "s" : ""}
              </span>
            ) : null}
            {showOrder && trajectorySteps.length > 1 ? (
              <span className="text-[9px] uppercase tracking-wider text-muted-foreground/50">
                drag to reorder
              </span>
            ) : null}
          </div>
        </div>

        <TrajectoryEditor
          steps={trajectorySteps}
          invalidSet={invalidTools}
          showOrder={showOrder}
          toolCategoryMap={toolCategoryMap}
          onChange={handleStepsChange}
        />

        {invalidTools.size > 0 ? (
          <div className="flex items-center gap-1.5 text-xs text-destructive">
            <span className="size-1.5 shrink-0 rounded-full bg-destructive" />
            Unknown: {[...invalidTools].join(", ")}
          </div>
        ) : null}

        {availableTools.length === 0 ? (
          <div className="space-y-1.5">
            <div className="flex items-center gap-2">
              <input
                value={manualTool}
                onChange={(event) => setManualTool(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    handleManualAdd();
                  }
                }}
                placeholder="Add a tool by name..."
                className="flex-1 rounded-md border bg-background px-2 py-1 text-xs"
              />
              <button
                type="button"
                onClick={handleManualAdd}
                disabled={!manualTool.trim()}
                className="rounded-md border px-2.5 py-1 text-xs hover:bg-muted disabled:opacity-50"
              >
                Add
              </button>
            </div>
            <div className="px-0.5 text-[10px] text-muted-foreground">
              {toolCatalogUnavailableMessage}
            </div>
          </div>
        ) : (
          <ToolPalette
            tools={availableTools}
            onAppend={handleAppendTool}
            initiallyExpanded={toolCatalogInitiallyExpanded}
          />
        )}
      </TestSectionCard>

      <TestSectionCard>
        <div className="space-y-3">
          <div className="flex items-center gap-3">
            <span className="shrink-0 text-[10px] uppercase tracking-wider text-muted-foreground/60">
              Mode
            </span>
            <div className="inline-flex items-center rounded-lg border bg-muted/30 p-0.5">
              {TRAJECTORY_MODES.map((trajectoryMode) => (
                <button
                  key={trajectoryMode.value}
                  type="button"
                  onClick={() => onModeChange(trajectoryMode.value)}
                  title={trajectoryMode.hint}
                  className={cx(
                    "rounded-md px-2.5 py-1 text-[11px] font-medium transition-colors",
                    mode === trajectoryMode.value
                      ? "bg-background text-foreground shadow-sm"
                      : "text-muted-foreground hover:text-foreground"
                  )}
                >
                  {trajectoryMode.label}
                </button>
              ))}
            </div>
            <span className="text-[10px] italic text-muted-foreground/40">
              {TRAJECTORY_MODES.find((trajectoryMode) => trajectoryMode.value === mode)?.hint}
            </span>
          </div>

          <div className="flex items-center gap-4">
            <div className="flex min-w-0 flex-1 items-center gap-2">
              <span className="shrink-0 text-[10px] uppercase tracking-wider text-muted-foreground/60">
                Tags
              </span>
              <InlineTagChips tags={tags} onChange={onTagsChange} />
            </div>

            {showStatus ? (
              <div className="flex shrink-0 items-center gap-1.5">
                <span className={cx("status-dot", TEST_STATUS_DOT_CLASS[status])} />
                <select
                  id="status"
                  value={status}
                  onChange={(event) => onStatusChange(event.target.value as SharedTestCaseStatus)}
                  className="cursor-pointer appearance-none border-none bg-transparent pr-4 text-[11px] font-medium text-foreground outline-none"
                  style={{
                    backgroundImage:
                      "url(\"data:image/svg+xml;charset=utf-8,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' fill='none' stroke='%236b7280' stroke-width='1.5'%3E%3Cpath d='M3 4.5l3 3 3-3'/%3E%3C/svg%3E\")",
                    backgroundRepeat: "no-repeat",
                    backgroundPosition: "right 0 center",
                    backgroundSize: "12px",
                  }}
                >
                  <option value="draft">Draft</option>
                  <option value="active">Active</option>
                  <option value="archived">Archived</option>
                </select>
              </div>
            ) : null}
          </div>
        </div>
      </TestSectionCard>

      <TestSectionCard className="border-dashed">
        <TestSectionHeader>Ground Truth (optional)</TestSectionHeader>
        {groundTruthEditor}
      </TestSectionCard>
    </div>
  );
}
