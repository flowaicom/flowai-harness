/**
 * Section card — the building block of every content page.
 *
 * A bordered container with an optional labeled header.
 * Composes via nesting: SectionCards can contain other SectionCards.
 *
 * Law L1: All visible content lives inside a SectionCard.
 *
 * @module components/shared/section-card
 */

import { cn } from "~/lib/utils";

interface SectionCardProps {
  readonly children: React.ReactNode;
  readonly className?: string;
}

export function SectionCard({ children, className }: SectionCardProps) {
  return (
    <div className={cn("rounded-lg border border-border/50 p-4 space-y-3", className)}>
      {children}
    </div>
  );
}

interface SectionHeaderProps {
  readonly children: React.ReactNode;
  readonly className?: string;
}

export function SectionHeader({ children, className }: SectionHeaderProps) {
  return <h3 className={cn("section-label", className)}>{children}</h3>;
}
