import {
  ClipboardCheckIcon,
  MessageSquareIcon,
  PlusIcon,
  SearchIcon,
  TrashIcon,
} from "lucide-react";
import { memo, type ReactNode, useMemo, useState } from "react";
import { Link } from "react-router";
import {
  filterThreadSidebarItems,
  groupThreadSidebarItems,
  shouldShowThreadSidebarSearch,
  type ThreadSidebarItem,
} from "./thread-sidebar-model";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

function compactRelativeTime(dateString: string): string {
  const date = new Date(dateString);
  const deltaMs = Date.now() - date.getTime();

  if (!Number.isFinite(deltaMs) || deltaMs < 0) return "now";

  const minutes = Math.floor(deltaMs / 60_000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m`;

  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;

  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d`;

  const weeks = Math.floor(days / 7);
  if (weeks < 5) return `${weeks}w`;

  const months = Math.floor(days / 30);
  if (months < 12) return `${months}mo`;

  return `${Math.floor(days / 365)}y`;
}

function SidebarSearch({
  value,
  onChange,
  placeholder,
}: {
  readonly value: string;
  readonly onChange: (value: string) => void;
  readonly placeholder: string;
}) {
  return (
    <div className="border-b px-3 py-2">
      <div className="flex items-center gap-2 rounded-md bg-muted/50 px-2.5 py-1.5">
        <SearchIcon className="size-3.5 shrink-0 text-muted-foreground" />
        <input
          type="text"
          value={value}
          onChange={(event) => onChange(event.target.value)}
          placeholder={placeholder}
          aria-label="Search conversations"
          className="flex-1 bg-transparent text-xs placeholder:text-muted-foreground/60 focus:outline-none"
        />
      </div>
    </div>
  );
}

interface SharedThreadSidebarViewProps {
  readonly title: string;
  readonly items: readonly ThreadSidebarItem[];
  readonly isLoading: boolean;
  readonly transientError?: string | null;
  readonly onCreate: () => void;
  readonly onDelete?: (threadId: string) => void;
  readonly onSaveAsTest?: (threadId: string) => void;
  readonly navSlot?: ReactNode;
  readonly footerSlot?: ReactNode;
  readonly createAriaLabel?: string;
  readonly emptyStateLabel?: string;
  readonly emptySearchLabel?: string;
  readonly emptyActionLabel?: string;
  readonly searchPlaceholder?: string;
  readonly formatTitle?: (title: string) => string;
}

const ThreadItemRow = memo(function ThreadItemRow({
  item,
  onDelete,
  onSaveAsTest,
  formatTitle,
}: {
  readonly item: ThreadSidebarItem;
  readonly onDelete?: (threadId: string) => void;
  readonly onSaveAsTest?: (threadId: string) => void;
  readonly formatTitle: (title: string) => string;
}) {
  const title = formatTitle(item.title || "New Conversation");

  return (
    <Link
      to={item.href}
      prefetch="intent"
      className={cx(
        "group flex items-center gap-2 rounded-md px-3 py-1.5 transition-colors",
        item.isSelected ? "bg-primary/10 text-primary" : "text-foreground hover:bg-muted"
      )}
    >
      {item.isStreaming ? (
        <span className="relative flex size-2 shrink-0">
          <span className="absolute inline-flex size-full animate-ping rounded-full bg-[var(--dot-blue)] opacity-75" />
          <span className="relative inline-flex size-2 rounded-full bg-[var(--dot-blue)]" />
        </span>
      ) : null}
      <span className="flex-1 truncate text-sm">{title}</span>
      <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground/70 group-hover:hidden">
        {compactRelativeTime(item.updatedAt)}
      </span>
      <div className="hidden shrink-0 items-center gap-0.5 group-hover:flex">
        {onSaveAsTest ? (
          <button
            type="button"
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              onSaveAsTest(item.id);
            }}
            className="rounded p-0.5 transition-colors hover:bg-primary/10 hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label="Save as test case"
            title="Save as test case"
          >
            <ClipboardCheckIcon className="size-3" />
          </button>
        ) : null}
        {onDelete ? (
          <button
            type="button"
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              onDelete(item.id);
            }}
            className="rounded p-0.5 transition-colors hover:bg-destructive/10 hover:text-destructive focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label="Delete thread"
          >
            <TrashIcon className="size-3" />
          </button>
        ) : null}
      </div>
    </Link>
  );
});

export const SharedThreadSidebarView = memo(function SharedThreadSidebarView({
  title,
  items,
  isLoading,
  transientError,
  onCreate,
  onDelete,
  onSaveAsTest,
  navSlot,
  footerSlot,
  createAriaLabel = "New conversation",
  emptyStateLabel = "No conversations yet",
  emptySearchLabel = "No matches",
  emptyActionLabel = "Start a new chat",
  searchPlaceholder = "Search conversations...",
  formatTitle = (value) => value,
}: SharedThreadSidebarViewProps) {
  const [searchQuery, setSearchQuery] = useState("");

  const displayedItems = useMemo(
    () => filterThreadSidebarItems(items, searchQuery),
    [items, searchQuery]
  );
  const groups = useMemo(() => groupThreadSidebarItems(displayedItems), [displayedItems]);

  return (
    <div className="flex h-full flex-col">
      {navSlot}

      <div className="flex items-center justify-between border-b px-4 py-3">
        <h2 className="text-sm font-semibold text-foreground">
          {title}
          {items.length > 0 ? (
            <span className="ml-1.5 font-normal tabular-nums text-muted-foreground">
              {items.length}
            </span>
          ) : null}
        </h2>
        <button
          type="button"
          onClick={onCreate}
          disabled={isLoading}
          className="rounded-md p-1.5 transition-colors hover:bg-muted disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-label={createAriaLabel}
        >
          <PlusIcon className="size-4" />
        </button>
      </div>

      {shouldShowThreadSidebarSearch(items) ? (
        <SidebarSearch
          value={searchQuery}
          onChange={setSearchQuery}
          placeholder={searchPlaceholder}
        />
      ) : null}

      {transientError ? (
        <div className="mx-2 mt-1 rounded-md bg-destructive/10 px-2.5 py-1.5 text-xs text-destructive">
          {transientError}
        </div>
      ) : null}

      <div className="scroll-container flex-1 space-y-0.5 overflow-y-auto p-2">
        {isLoading && items.length === 0 ? (
          <div className="space-y-0.5 px-1 pt-1">
            {Array.from({ length: 6 }, (_, index) => (
              <div
                key={`thread-skeleton-${index}`}
                className="flex items-center gap-2 px-3 py-1.5"
                style={{ animationDelay: `${index * 50}ms` }}
              >
                <div
                  className="h-3.5 flex-1 animate-pulse rounded bg-muted"
                  style={{ maxWidth: `${80 - index * 8}%` }}
                />
                <div className="h-3 w-6 animate-pulse rounded bg-muted/60" />
              </div>
            ))}
          </div>
        ) : displayedItems.length === 0 ? (
          <div className="px-4 py-8 text-center">
            <MessageSquareIcon className="mx-auto mb-2 size-8 text-muted-foreground/30" />
            <p className="text-sm text-muted-foreground">
              {searchQuery ? emptySearchLabel : emptyStateLabel}
            </p>
            {!searchQuery ? (
              <button
                type="button"
                onClick={onCreate}
                className="mt-2 rounded text-xs text-primary hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                {emptyActionLabel}
              </button>
            ) : null}
          </div>
        ) : (
          groups.map((group) => (
            <div key={group.period}>
              <div className="px-3 pb-1 pt-2 first:pt-0">
                <span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground/60">
                  {group.label}
                </span>
              </div>
              {group.items.map((item) => (
                <ThreadItemRow
                  key={item.id}
                  item={item}
                  onDelete={onDelete}
                  onSaveAsTest={onSaveAsTest}
                  formatTitle={formatTitle}
                />
              ))}
            </div>
          ))
        )}
      </div>

      {footerSlot ? <div className="flex justify-end border-t p-3">{footerSlot}</div> : null}
    </div>
  );
});
