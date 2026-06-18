import type { AppScope } from "@studio/core/domain/scope";
import { useVirtualizer } from "@tanstack/react-virtual";
import { MessageSquareIcon, SearchIcon } from "lucide-react";
import type { ChangeEvent, KeyboardEvent, ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ConnectCategoryBadge,
  ConnectEmptyState,
  ConnectErrorBanner,
  ConnectPillTabs,
  ConnectSectionCard,
  ConnectSectionHeader,
} from "./connect-page-primitives";
import { type ConnectRuntimeResult, isConnectRuntimeOk } from "./connect-page-types";
import {
  buildConnectSearchAskPrompt,
  CONNECT_SEARCH_TABS,
  type ConnectSearchMode,
  type ConnectToolSearchRow,
  getConnectSearchItemCategory,
  parseConnectToolSearchRows,
  resolveConnectSearchRequest,
} from "./connect-search-model";

const ROW_HEIGHT = 44;

export interface ConnectSearchResultItemLike {
  readonly id: string;
  readonly name: string;
  readonly itemType: string;
  readonly description: string | null;
  readonly tags: readonly string[];
  readonly score: number;
}

export interface ConnectSearchResultsLike<
  TItem extends ConnectSearchResultItemLike = ConnectSearchResultItemLike,
> {
  readonly items: readonly TItem[];
  readonly totalCount: number;
  readonly queryTimeMs: number;
}

export interface ConnectToolResultLike {
  readonly success: boolean;
  readonly data: string;
  readonly count?: number | null;
  readonly error?: string | null;
}

export interface ConnectSearchRuntimeLike<
  TSearchResults extends ConnectSearchResultsLike = ConnectSearchResultsLike,
> {
  searchCatalog(
    scope: AppScope,
    input: { readonly query: string; readonly mode: "unified" | "semantic" }
  ): Promise<ConnectRuntimeResult<TSearchResults>>;
  runSearchTool(
    scope: AppScope,
    toolId: string,
    input: Record<string, unknown>
  ): Promise<ConnectRuntimeResult<ConnectToolResultLike>>;
}

export interface ConnectSearchPageProps<
  TSearchResults extends ConnectSearchResultsLike = ConnectSearchResultsLike,
> {
  readonly scope: AppScope;
  readonly scopeKey: string;
  readonly runtime: ConnectSearchRuntimeLike<TSearchResults>;
  readonly query: string;
  readonly setQuery: (query: string) => void;
  readonly results: TSearchResults | null;
  readonly setResults: (results: TSearchResults | null) => void;
  readonly onAskAboutItem: (args: {
    readonly item: TSearchResults["items"][number];
    readonly prompt: string;
    readonly title: string;
  }) => Promise<void> | void;
  readonly headerAccessory?: ReactNode;
  readonly subtitle?: ReactNode;
  readonly targetMeta?: ReactNode;
  readonly formatText?: (value: string) => string;
}

export function ConnectSearchPage<
  TSearchResults extends ConnectSearchResultsLike = ConnectSearchResultsLike,
>({
  scope,
  scopeKey,
  runtime,
  query,
  setQuery,
  results,
  setResults,
  onAskAboutItem,
  headerAccessory,
  subtitle = "Search across tables, columns, enums, metrics, and knowledge",
  targetMeta,
  formatText = (value) => value,
}: ConnectSearchPageProps<TSearchResults>) {
  const [isSearching, setIsSearching] = useState(false);
  const [mode, setMode] = useState<ConnectSearchMode>("unified");
  const [toolResult, setToolResult] = useState<ConnectToolResultLike | null>(null);
  const [error, setError] = useState<string | null>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const searchRequestIdRef = useRef(0);

  const executeSearch = useCallback(
    async (searchQuery: string) => {
      const requestId = searchRequestIdRef.current + 1;
      searchRequestIdRef.current = requestId;
      setError(null);

      if (!searchQuery.trim()) {
        setResults(null);
        setToolResult(null);
        setIsSearching(false);
        return;
      }

      setIsSearching(true);
      setToolResult(null);

      const request = resolveConnectSearchRequest(mode, searchQuery);
      if (request.kind === "catalog") {
        const result = await runtime.searchCatalog(scope, request);
        if (searchRequestIdRef.current !== requestId) {
          return;
        }
        if (isConnectRuntimeOk(result)) {
          setResults(result.value);
        } else {
          setError(result.error.message);
        }
      } else {
        const result = await runtime.runSearchTool(scope, request.toolId, request.input);
        if (searchRequestIdRef.current !== requestId) {
          return;
        }
        if (isConnectRuntimeOk(result)) {
          setToolResult(result.value);
          setResults(null);
        } else {
          setError(result.error.message);
        }
      }

      if (searchRequestIdRef.current !== requestId) {
        return;
      }
      setIsSearching(false);
    },
    [mode, runtime, scope, setResults]
  );

  const handleInputChange = useCallback(
    (event: ChangeEvent<HTMLInputElement>) => {
      const nextQuery = event.target.value;
      setQuery(nextQuery);
      clearTimeout(debounceRef.current);
      if (!nextQuery.trim()) {
        setResults(null);
        setToolResult(null);
        setError(null);
        setIsSearching(false);
        return;
      }
      debounceRef.current = setTimeout(() => {
        void executeSearch(nextQuery);
      }, 250);
    },
    [executeSearch, setQuery, setResults]
  );

  const handleKeyDown = useCallback(
    (event: KeyboardEvent<HTMLInputElement>) => {
      if (event.key === "Enter") {
        event.preventDefault();
        clearTimeout(debounceRef.current);
        void executeSearch(query);
        return;
      }

      if (event.key === "Escape") {
        clearTimeout(debounceRef.current);
        setQuery("");
        setResults(null);
        setToolResult(null);
        setError(null);
        setIsSearching(false);
        (event.target as HTMLInputElement).blur();
      }
    },
    [executeSearch, query, setQuery, setResults]
  );

  useEffect(
    () => () => {
      searchRequestIdRef.current += 1;
      clearTimeout(debounceRef.current);
    },
    []
  );

  useEffect(() => {
    searchRequestIdRef.current += 1;
    setToolResult(null);
    setError(null);
    setIsSearching(false);
  }, [scopeKey]);

  useEffect(() => {
    if (!query.trim()) {
      setResults(null);
      setToolResult(null);
      setError(null);
      return;
    }

    void executeSearch(query);
  }, [executeSearch, query, setResults]);

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      <div className="px-6 py-4 border-b">
        <div className={headerAccessory ? "flex items-center justify-between mb-1" : undefined}>
          <h1 className="text-lg font-semibold">Search</h1>
          {headerAccessory}
        </div>
        <p className="text-sm text-muted-foreground">{subtitle}</p>
        {targetMeta ? <div className="text-xs text-muted-foreground mt-1">{targetMeta}</div> : null}
      </div>

      <div className="px-6 py-4 border-b space-y-3">
        <div className="relative max-w-xl">
          <SearchIcon className="absolute left-3 top-1/2 -translate-y-1/2 size-4 text-muted-foreground" />
          <input
            type="text"
            value={query}
            onChange={handleInputChange}
            onKeyDown={handleKeyDown}
            autoFocus
            placeholder={
              mode === "resolveTerm"
                ? "Resolve a business term..."
                : "Search tables, columns, metrics..."
            }
            className="w-full pl-9 pr-3 py-2 border rounded-md bg-background text-sm focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          />
        </div>
        <div className="flex items-center justify-between">
          <ConnectPillTabs tabs={CONNECT_SEARCH_TABS} active={mode} onChange={setMode} />
          <span className="text-[10px] text-muted-foreground/40 hidden sm:inline">
            Enter to search · Esc to clear
          </span>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto scroll-container">
        <div className="max-w-3xl mx-auto p-6 space-y-6">
          {error ? <ConnectErrorBanner message={error} onDismiss={() => setError(null)} /> : null}

          {isSearching ? (
            <ConnectSectionCard>
              <ConnectSectionHeader>Searching...</ConnectSectionHeader>
              <div className="space-y-2">
                {Array.from({ length: 5 }, (_, index) => (
                  <div
                    key={`search-skeleton-${index}`}
                    className="flex items-center gap-3 py-2"
                    style={{ animationDelay: `${index * 60}ms` }}
                  >
                    <div
                      className="h-4 bg-muted rounded animate-pulse flex-1"
                      style={{ maxWidth: `${80 - index * 10}%` }}
                    />
                    <div className="h-5 w-14 bg-muted/60 rounded-full animate-pulse" />
                    <div className="h-3 w-10 bg-muted/40 rounded animate-pulse" />
                  </div>
                ))}
              </div>
            </ConnectSectionCard>
          ) : toolResult ? (
            <ToolSearchResults result={toolResult} formatText={formatText} />
          ) : results ? (
            results.items.length === 0 ? (
              <ConnectEmptyState
                icon={SearchIcon}
                title="No results"
                description={`No matches found for "${query}"`}
              />
            ) : (
              <ConnectSectionCard>
                <ConnectSectionHeader>
                  {results.totalCount} results
                  <span className="ml-2 font-mono tabular-nums opacity-60">
                    {results.queryTimeMs}ms
                  </span>
                </ConnectSectionHeader>
                <div className="divide-y">
                  {results.items.map((item) => (
                    <SearchResultItem
                      key={item.id}
                      item={item}
                      formatText={formatText}
                      onAskAboutItem={onAskAboutItem}
                    />
                  ))}
                </div>
              </ConnectSectionCard>
            )
          ) : (
            <ConnectEmptyState
              icon={SearchIcon}
              title="Search your data"
              description="Enter a query to search tables, columns, metrics, and knowledge items"
            />
          )}
        </div>
      </div>
    </div>
  );
}

function ScoreBar({ score }: { readonly score: number }) {
  const percentage = Math.round(score * 100);
  return (
    <div className="flex items-center gap-2 shrink-0">
      <div className="w-12 h-1.5 bg-muted rounded-full overflow-hidden">
        <div className="h-full rounded-full bg-primary/60" style={{ width: `${percentage}%` }} />
      </div>
      <span className="text-[10px] font-mono tabular-nums text-muted-foreground w-7 text-right">
        {percentage}%
      </span>
    </div>
  );
}

function SearchResultItem<TItem extends ConnectSearchResultItemLike>({
  item,
  formatText,
  onAskAboutItem,
}: {
  readonly item: TItem;
  readonly formatText: (value: string) => string;
  readonly onAskAboutItem: (args: {
    readonly item: TItem;
    readonly prompt: string;
    readonly title: string;
  }) => Promise<void> | void;
}) {
  const [busy, setBusy] = useState(false);

  const handleAsk = useCallback(async () => {
    if (busy) {
      return;
    }

    setBusy(true);
    try {
      const prompt = buildConnectSearchAskPrompt({
        itemType: item.itemType,
        name: item.name,
        description: item.description,
      });

      await onAskAboutItem({
        item,
        prompt,
        title: `Ask: ${item.name}`,
      });
    } finally {
      setBusy(false);
    }
  }, [busy, item, onAskAboutItem]);

  return (
    <div className="py-2.5 first:pt-0 last:pb-0 space-y-1 group">
      <div className="flex items-center gap-2">
        <span className="font-medium text-sm truncate flex-1">{formatText(item.name)}</span>
        <button
          type="button"
          onClick={handleAsk}
          disabled={busy}
          className="opacity-0 group-hover:opacity-100 transition-opacity flex items-center gap-1 px-2 py-0.5 rounded text-[10px] text-muted-foreground hover:text-foreground hover:bg-muted border"
          title="Ask about this in chat"
        >
          <MessageSquareIcon className="size-3" />
          Ask
        </button>
        <ConnectCategoryBadge
          label={item.itemType}
          category={getConnectSearchItemCategory(item.itemType)}
        />
        <ScoreBar score={item.score} />
      </div>
      {item.description ? (
        <p className="text-xs text-muted-foreground line-clamp-2">{formatText(item.description)}</p>
      ) : null}
      {item.tags.length > 0 ? (
        <div className="flex gap-1 flex-wrap">
          {item.tags.map((tag) => (
            <span
              key={tag}
              className="px-1.5 py-0.5 bg-muted/50 rounded text-[10px] text-muted-foreground"
            >
              {formatText(tag)}
            </span>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function ToolSearchResults({
  result,
  formatText,
}: {
  readonly result: ConnectToolResultLike;
  readonly formatText: (value: string) => string;
}) {
  const parentRef = useRef<HTMLDivElement>(null);
  const rows = useMemo<ConnectToolSearchRow[]>(
    () => (result.success ? parseConnectToolSearchRows(result.data) : []),
    [result]
  );

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 20,
  });

  if (!result.success) {
    return <ConnectErrorBanner message={result.error ?? "Search failed"} onDismiss={() => {}} />;
  }

  if (rows.length === 0) {
    if (result.data && result.data.length > 2) {
      return (
        <ConnectSectionCard>
          <ConnectSectionHeader>Raw Result</ConnectSectionHeader>
          <pre className="p-2.5 bg-muted/50 rounded-lg text-xs font-mono overflow-x-auto keyline-card max-h-96 overflow-y-auto scroll-container whitespace-pre-wrap">
            {result.data}
          </pre>
        </ConnectSectionCard>
      );
    }

    return (
      <ConnectEmptyState
        icon={SearchIcon}
        title="No results"
        description="Try a different search query"
      />
    );
  }

  return (
    <ConnectSectionCard>
      <ConnectSectionHeader>{rows.length} results</ConnectSectionHeader>
      <div
        ref={parentRef}
        className="overflow-y-auto scroll-container rounded-lg keyline-card"
        style={{ maxHeight: "calc(100vh - 340px)" }}
      >
        <div
          style={{
            height: virtualizer.getTotalSize(),
            position: "relative",
          }}
        >
          {virtualizer.getVirtualItems().map((virtualItem) => {
            const row = rows[virtualItem.index];
            return (
              <div
                key={virtualItem.index}
                className="flex items-center gap-3 px-3 border-b last:border-b-0 absolute w-full"
                style={{
                  height: ROW_HEIGHT,
                  transform: `translateY(${virtualItem.start}px)`,
                }}
              >
                <span className="font-mono text-xs truncate flex-1" title={formatText(row.name)}>
                  {formatText(row.name)}
                </span>
                <ConnectCategoryBadge
                  label={row.itemType}
                  category={getConnectSearchItemCategory(row.itemType)}
                />
                <span
                  className="text-xs text-muted-foreground truncate max-w-40"
                  title={formatText(row.description)}
                >
                  {formatText(row.description)}
                </span>
                <ScoreBar score={row.score} />
              </div>
            );
          })}
        </div>
      </div>
    </ConnectSectionCard>
  );
}
