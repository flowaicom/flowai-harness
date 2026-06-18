import type { AppScope } from "@studio/core/domain/scope";
import { ChevronDownIcon, ChevronRightIcon, Loader2Icon, PlayIcon, WrenchIcon } from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ConnectEmptyState,
  ConnectErrorBanner,
  ConnectSectionCard,
  ConnectSectionHeader,
} from "./connect-page-primitives";
import { type ConnectRuntimeResult, isConnectRuntimeOk } from "./connect-page-types";
import { getConnectToolDescriptionBlocks, getConnectToolsSection } from "./connect-tools-model";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

export interface ConnectToolInfoLike {
  readonly id: string;
  readonly name: string;
  readonly description: string;
  readonly parameters: Record<string, unknown>;
}

export interface ConnectToolResultLike {
  readonly success: boolean;
  readonly data: string;
  readonly count?: number | null;
  readonly error?: string | null;
}

export interface ConnectToolsRuntimeLike {
  listTools(scope: AppScope): Promise<ConnectRuntimeResult<readonly ConnectToolInfoLike[]>>;
  executeTool(
    scope: AppScope,
    toolId: string,
    input: Record<string, unknown>
  ): Promise<ConnectRuntimeResult<ConnectToolResultLike>>;
}

export interface ConnectToolsPageProps {
  readonly scope: AppScope;
  readonly scopeKey: string;
  readonly runtime: ConnectToolsRuntimeLike;
  readonly headerAccessory?: ReactNode;
  readonly subtitle?: ReactNode;
  readonly targetMeta?: ReactNode;
  readonly emptyDescription: string;
  readonly emptyAction?: { readonly label: string; readonly onClick: () => void };
}

export function ConnectToolsPage({
  scope,
  scopeKey,
  runtime,
  headerAccessory,
  subtitle = "Browse and test data catalog tools against the active target",
  targetMeta,
  emptyDescription,
  emptyAction,
}: ConnectToolsPageProps) {
  const [tools, setTools] = useState<readonly ConnectToolInfoLike[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [expandedTool, setExpandedTool] = useState<string | null>(null);
  const loadRequestIdRef = useRef(0);

  const loadTools = useCallback(async () => {
    const requestId = loadRequestIdRef.current + 1;
    loadRequestIdRef.current = requestId;

    setIsLoading(true);
    setLoadError(null);

    const result = await runtime.listTools(scope);
    if (loadRequestIdRef.current !== requestId) {
      return;
    }

    setIsLoading(false);
    if (isConnectRuntimeOk(result)) {
      setTools(result.value);
    } else {
      setTools([]);
      setLoadError(result.error.message);
    }
  }, [runtime, scope]);

  useEffect(() => {
    void loadTools();

    return () => {
      loadRequestIdRef.current += 1;
    };
  }, [loadTools]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: scopeKey resets expansion when the active workspace changes.
  useEffect(() => {
    setExpandedTool(null);
  }, [scopeKey]);

  const section = useMemo(() => getConnectToolsSection(tools), [tools]);

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      <div className="px-6 py-4 border-b">
        <div className={headerAccessory ? "flex items-center justify-between mb-1" : undefined}>
          <h1 className="text-lg font-semibold">Tools</h1>
          {headerAccessory}
        </div>
        <p className="text-sm text-muted-foreground">{subtitle}</p>
        {targetMeta ? <div className="text-xs text-muted-foreground mt-1">{targetMeta}</div> : null}
      </div>

      {loadError ? (
        <div className="px-6 pt-3">
          <ConnectErrorBanner
            message={loadError}
            onDismiss={() => setLoadError(null)}
            onRetry={() => {
              void loadTools();
            }}
          />
        </div>
      ) : null}

      <div className="flex-1 overflow-y-auto scroll-container">
        <div className="max-w-3xl mx-auto p-6 space-y-6">
          {isLoading ? (
            <ConnectSectionCard>
              <ConnectSectionHeader>Loading tools...</ConnectSectionHeader>
              <div className="space-y-2">
                {/* biome-ignore-start lint/suspicious/noArrayIndexKey: static skeleton rows never reorder */}
                {Array.from({ length: 6 }, (_, index) => (
                  <div
                    key={`tool-skeleton-${index}`}
                    className="flex items-center gap-3 py-2"
                    style={{ animationDelay: `${index * 60}ms` }}
                  >
                    <div className="size-4 bg-muted rounded animate-pulse" />
                    <div
                      className="h-4 bg-muted rounded animate-pulse flex-1"
                      style={{ maxWidth: `${70 - index * 6}%` }}
                    />
                    <div className="h-5 w-16 bg-muted/60 rounded-full animate-pulse" />
                  </div>
                ))}
                {/* biome-ignore-end lint/suspicious/noArrayIndexKey: static skeleton rows never reorder */}
              </div>
            </ConnectSectionCard>
          ) : tools.length === 0 ? (
            <ConnectEmptyState
              icon={WrenchIcon}
              title="No tools available"
              description={emptyDescription}
              action={emptyAction}
            />
          ) : (
            <ConnectSectionCard>
              <div className="flex items-center gap-2">
                <ConnectSectionHeader>{section.title}</ConnectSectionHeader>
                <span className="text-xs text-muted-foreground font-mono tabular-nums">
                  {section.tools.length} tools
                </span>
              </div>
              <div className="divide-y">
                {section.tools.map((tool) => (
                  <ToolCard
                    key={`${tool.id}:${scopeKey}`}
                    tool={tool}
                    scope={scope}
                    runtime={runtime}
                    isExpanded={expandedTool === tool.id}
                    onToggle={() => setExpandedTool(expandedTool === tool.id ? null : tool.id)}
                  />
                ))}
              </div>
            </ConnectSectionCard>
          )}
        </div>
      </div>
    </div>
  );
}

function ToolCard({
  tool,
  scope,
  runtime,
  isExpanded,
  onToggle,
}: {
  readonly tool: ConnectToolInfoLike;
  readonly scope: AppScope;
  readonly runtime: ConnectToolsRuntimeLike;
  readonly isExpanded: boolean;
  readonly onToggle: () => void;
}) {
  const [input, setInput] = useState("{}");
  const [result, setResult] = useState<ConnectToolResultLike | null>(null);
  const [isRunning, setIsRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleExecute = useCallback(async () => {
    setIsRunning(true);
    setError(null);
    setResult(null);

    try {
      const parsed = JSON.parse(input) as Record<string, unknown>;
      const response = await runtime.executeTool(scope, tool.id, parsed);
      if (isConnectRuntimeOk(response)) {
        setResult(response.value);
      } else {
        setError(response.error.message);
      }
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Invalid JSON input");
    } finally {
      setIsRunning(false);
    }
  }, [input, runtime, scope, tool.id]);

  return (
    <div className="py-2.5 first:pt-0 last:pb-0">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={isExpanded}
        aria-label={`${isExpanded ? "Collapse" : "Expand"} ${tool.name}`}
        className="w-full flex items-center gap-2 py-1.5 text-left hover:bg-muted/50 rounded-md px-2 -mx-2 transition-colors focus-visible:ring-2 focus-visible:ring-primary/50 focus-visible:outline-none"
      >
        {isExpanded ? (
          <ChevronDownIcon className="size-3.5 text-muted-foreground shrink-0" />
        ) : (
          <ChevronRightIcon className="size-3.5 text-muted-foreground shrink-0" />
        )}
        <span className="font-medium text-sm truncate flex-1">{tool.name}</span>
        <span className="font-mono text-[10px] text-muted-foreground/60 shrink-0">{tool.id}</span>
      </button>

      {isExpanded ? (
        <div className="ml-5.5 mt-2 space-y-3">
          <ToolDescription description={tool.description} />

          {Object.keys(tool.parameters).length > 0 ? (
            <div>
              <span className="text-xs font-medium text-muted-foreground">Parameters</span>
              <pre className="mt-1 p-2.5 bg-muted/50 rounded-md text-xs font-mono overflow-x-auto border">
                {JSON.stringify(tool.parameters, null, 2)}
              </pre>
            </div>
          ) : null}

          <label className="space-y-1">
            <span className="text-xs font-medium text-muted-foreground">Input JSON</span>
            <textarea
              value={input}
              onChange={(event) => setInput(event.target.value)}
              rows={3}
              className="mt-1 w-full p-2.5 border rounded-md bg-background text-xs font-mono resize-y focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              placeholder='{"query": "bookings"}'
            />
          </label>

          <button
            type="button"
            onClick={handleExecute}
            disabled={isRunning}
            className="flex items-center gap-2 px-3 py-1.5 bg-primary text-primary-foreground rounded-md hover:bg-primary/90 transition-colors disabled:opacity-50 text-xs font-medium focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          >
            {isRunning ? (
              <Loader2Icon className="size-3.5 animate-spin" />
            ) : (
              <PlayIcon className="size-3.5" />
            )}
            {isRunning ? "Running..." : "Execute"}
          </button>

          {error ? <ConnectErrorBanner message={error} onDismiss={() => setError(null)} /> : null}

          {isRunning && !result ? (
            <div className="space-y-2">
              <div className="flex items-center gap-2">
                <div className="h-5 w-16 bg-muted rounded animate-pulse" />
                <div className="h-4 w-12 bg-muted/60 rounded animate-pulse" />
              </div>
              <div className="p-2.5 bg-muted/50 rounded-md border space-y-1.5">
                {/* biome-ignore-start lint/suspicious/noArrayIndexKey: static skeleton rows never reorder */}
                {Array.from({ length: 4 }, (_, index) => (
                  <div
                    key={`result-skel-${index}`}
                    className="h-3 bg-muted rounded animate-pulse"
                    style={{ width: `${85 - index * 12}%`, animationDelay: `${index * 75}ms` }}
                  />
                ))}
                {/* biome-ignore-end lint/suspicious/noArrayIndexKey: static skeleton rows never reorder */}
              </div>
            </div>
          ) : null}

          {result ? (
            <div className="space-y-2">
              <div className="flex items-center gap-2">
                <span
                  className={cx(
                    "px-1.5 py-0.5 rounded text-xs font-medium",
                    result.success
                      ? "bg-[var(--accent-emerald)] text-[var(--dot-emerald)]"
                      : "bg-destructive/10 text-destructive"
                  )}
                >
                  {result.success ? "Success" : "Failed"}
                </span>
                {result.count != null ? (
                  <span className="text-xs text-muted-foreground font-mono tabular-nums">
                    {result.count} items
                  </span>
                ) : null}
              </div>
              <pre className="p-2.5 bg-muted/50 rounded-md text-xs font-mono overflow-x-auto border max-h-64 overflow-y-auto scroll-container whitespace-pre-wrap">
                {result.data}
              </pre>
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function ToolDescription({ description }: { readonly description: string }) {
  const blocks = useMemo(() => getConnectToolDescriptionBlocks(description), [description]);
  if (blocks.length === 0) return null;

  return (
    <div className="space-y-2 text-xs text-muted-foreground">
      {blocks.map((block, index) => {
        switch (block.kind) {
          case "heading":
            return (
              <p
                key={`heading-${index}`}
                className="pt-1 font-medium leading-5 text-foreground first:pt-0"
              >
                {block.text}
              </p>
            );
          case "list":
            return (
              <ul key={`list-${index}`} className="list-disc space-y-1 pl-4 leading-5">
                {block.items.map((item, itemIndex) => (
                  <li key={`${itemIndex}-${item}`}>{renderInlineCode(item)}</li>
                ))}
              </ul>
            );
          case "paragraph":
            return (
              <p key={`paragraph-${index}`} className="leading-5">
                {renderInlineCode(block.text)}
              </p>
            );
        }
      })}
    </div>
  );
}

function renderInlineCode(text: string): ReactNode[] {
  return text.split(/(`[^`]+`)/g).map((part, index) => {
    if (/^`[^`]+`$/.test(part)) {
      return (
        <code
          key={`${index}-${part}`}
          className="rounded bg-muted px-1 py-0.5 font-mono text-[0.94em] text-foreground"
        >
          {part.slice(1, -1)}
        </code>
      );
    }

    return part;
  });
}
