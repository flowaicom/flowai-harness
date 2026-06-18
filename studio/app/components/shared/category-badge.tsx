/**
 * Category badge — semantic color label for taxonomic categories.
 *
 * Maps a category string to a 4-color tuple (bg, text, border, hoverBorder).
 * Used for tool tiers, knowledge types, relationship direction, column keys.
 *
 * Law L2: Every colored indicator derives from StatusColor or CategoryColor.
 *
 * @module components/shared/category-badge
 */

import { cn } from "~/lib/utils";

/**
 * The four stable category color families.
 * Each key maps to Tailwind classes for bg, text, border, and hoverBorder.
 */
export const CATEGORY_COLORS: Record<
  string,
  { bg: string; text: string; border: string; hoverBorder: string }
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

const FALLBACK_COLOR = {
  bg: "bg-primary/8",
  text: "text-primary",
  border: "border-primary/20",
  hoverBorder: "hover:border-primary/40",
};

/** Look up the color tuple for a category, falling back to primary. */
export function categoryColor(category: string | undefined) {
  return (category && CATEGORY_COLORS[category]) || FALLBACK_COLOR;
}

interface CategoryBadgeProps {
  readonly label: string;
  readonly category: string;
  readonly className?: string;
}

export function CategoryBadge({ label, category, className }: CategoryBadgeProps) {
  const color = categoryColor(category);

  return (
    <span
      className={cn(
        "px-1.5 py-0.5 rounded text-xs font-medium border",
        color.bg,
        color.text,
        color.border,
        className
      )}
    >
      {label}
    </span>
  );
}
