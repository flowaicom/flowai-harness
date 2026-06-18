export type ThreadSidebarPeriod = "today" | "yesterday" | "thisWeek" | "thisMonth" | "older";

export interface ThreadSidebarItem {
  readonly id: string;
  readonly title: string | null;
  readonly updatedAt: string;
  readonly href: string;
  readonly isSelected: boolean;
  readonly isStreaming?: boolean;
}

export interface ThreadSidebarGroup<TItem extends ThreadSidebarItem = ThreadSidebarItem> {
  readonly period: ThreadSidebarPeriod;
  readonly label: string;
  readonly items: TItem[];
}

const THREAD_SIDEBAR_PERIOD_ORDER: readonly ThreadSidebarPeriod[] = [
  "today",
  "yesterday",
  "thisWeek",
  "thisMonth",
  "older",
];

const THREAD_SIDEBAR_PERIOD_LABELS: Record<ThreadSidebarPeriod, string> = {
  today: "Today",
  yesterday: "Yesterday",
  thisWeek: "This week",
  thisMonth: "This month",
  older: "Older",
};

function getThreadSidebarPeriod(updatedAt: string | Date | undefined | null): ThreadSidebarPeriod {
  if (!updatedAt) return "older";

  const date = typeof updatedAt === "string" ? new Date(updatedAt) : updatedAt;
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const yesterday = new Date(today.getTime() - 86_400_000);
  const weekAgo = new Date(today.getTime() - 7 * 86_400_000);
  const monthAgo = new Date(today.getTime() - 30 * 86_400_000);

  if (date >= today) return "today";
  if (date >= yesterday) return "yesterday";
  if (date >= weekAgo) return "thisWeek";
  if (date >= monthAgo) return "thisMonth";
  return "older";
}

export function shouldShowThreadSidebarSearch(
  items: readonly ThreadSidebarItem[],
  minimumVisibleItems = 4
): boolean {
  return items.length >= minimumVisibleItems;
}

export function filterThreadSidebarItems<TItem extends ThreadSidebarItem>(
  items: readonly TItem[],
  searchQuery: string
): TItem[] {
  const query = searchQuery.trim().toLowerCase();
  if (!query) {
    return [...items];
  }

  return items.filter((item) => (item.title || "").toLowerCase().includes(query));
}

export function groupThreadSidebarItems<TItem extends ThreadSidebarItem>(
  items: readonly TItem[]
): ThreadSidebarGroup<TItem>[] {
  const groups = new Map<ThreadSidebarPeriod, TItem[]>();

  for (const item of items) {
    const period = getThreadSidebarPeriod(item.updatedAt);
    const bucket = groups.get(period);
    if (bucket) {
      bucket.push(item);
    } else {
      groups.set(period, [item]);
    }
  }

  return THREAD_SIDEBAR_PERIOD_ORDER.flatMap((period) => {
    const periodItems = groups.get(period);
    return periodItems
      ? [
          {
            period,
            label: THREAD_SIDEBAR_PERIOD_LABELS[period],
            items: periodItems,
          },
        ]
      : [];
  });
}
