/**
 * Item card — composable card for list items with badges and actions.
 *
 * Used in Knowledge (documents, items, metrics), Discovery (relationships),
 * and Profiling (tables). Title row with badges, optional description,
 * optional children (SQL blocks, progress bars, etc.), hover-reveal delete.
 *
 * @module components/shared/item-card
 */

import { TrashIcon } from "lucide-react";
import { cn } from "~/lib/utils";
import { CategoryBadge } from "./category-badge";

interface Badge {
  readonly label: string;
  readonly category: string;
}

interface ItemCardProps {
  readonly title: string;
  readonly badges?: readonly Badge[];
  readonly metadata?: string;
  readonly onDelete?: () => void;
  readonly children?: React.ReactNode;
  readonly className?: string;
}

export function ItemCard({
  title,
  badges,
  metadata,
  onDelete,
  children,
  className,
}: ItemCardProps) {
  return (
    <div className={cn("group border rounded-lg p-3", className)}>
      {/* Title row */}
      <div className="flex items-center gap-2">
        <span className="font-medium text-sm truncate flex-1">{title}</span>
        {badges?.map((b) => (
          <CategoryBadge key={b.label} label={b.label} category={b.category} />
        ))}
        {onDelete && (
          <button
            type="button"
            onClick={onDelete}
            className="ml-auto p-1 rounded opacity-0 group-hover:opacity-100 hover:bg-destructive/10 hover:text-destructive transition-all shrink-0"
            aria-label={`Delete ${title}`}
          >
            <TrashIcon className="size-3.5" />
          </button>
        )}
      </div>

      {/* Optional metadata */}
      {metadata && <p className="text-xs text-muted-foreground mt-1">{metadata}</p>}

      {/* Optional children (SQL, progress bar, etc.) */}
      {children && <div className="mt-2">{children}</div>}
    </div>
  );
}
