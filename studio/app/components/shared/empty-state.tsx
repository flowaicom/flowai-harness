/**
 * Empty state — standardized zero-data display.
 *
 * Centered icon in a rounded container, title, description,
 * and optional CTA button.
 *
 * Law L6: Every fetch has 4 explicit states (loading, error, empty, populated).
 * This component handles the "empty" state consistently.
 *
 * @module components/shared/empty-state
 */

import type { LucideIcon } from "lucide-react";
import { cn } from "~/lib/utils";
import { StudioActionButton } from "./studio-action";

interface EmptyStateProps {
  readonly icon: LucideIcon;
  readonly title: string;
  readonly description: string;
  readonly action?: {
    readonly label: string;
    readonly icon?: LucideIcon;
    readonly onClick: () => void;
  };
  readonly className?: string;
}

export function EmptyState({ icon: Icon, title, description, action, className }: EmptyStateProps) {
  return (
    <div className={cn("flex-1 flex items-center justify-center text-muted-foreground", className)}>
      <div className="text-center max-w-sm">
        <div className="w-12 h-12 mx-auto mb-4 rounded-lg bg-muted/50 flex items-center justify-center">
          <Icon className="w-6 h-6 text-muted-foreground/30" />
        </div>
        <p className="text-sm font-medium text-foreground mb-1">{title}</p>
        <p className="text-xs text-muted-foreground mb-4">{description}</p>
        {action && (
          <StudioActionButton icon={action.icon} onClick={action.onClick} tone="strong">
            {action.label}
          </StudioActionButton>
        )}
      </div>
    </div>
  );
}
