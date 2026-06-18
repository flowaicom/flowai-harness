/**
 * Generic tool output card renderer.
 *
 * Framework-agnostic renderer that displays tool name, props,
 * and action buttons.
 */

import { CheckIcon, XIcon } from "lucide-react";
import React, { useState } from "react";
import { cn } from "~/lib/utils";

interface CommandCardRendererProps {
  readonly dsl: string;
  readonly onAction?: (action: string, payload?: Record<string, unknown>) => void;
  readonly isReadOnly?: boolean;
  readonly decision?: "approved" | "cancelled" | "modification_requested";
}

interface ParsedCard {
  readonly component: string;
  readonly props: Record<string, unknown>;
  readonly actions?: Array<{
    label: string;
    action: string;
    variant?: "primary" | "secondary" | "destructive";
  }>;
}

function tryParseDSL(dsl: string): ParsedCard | null {
  try {
    const parsed = JSON.parse(dsl);
    return parsed as ParsedCard;
  } catch {
    return null;
  }
}

export const CommandCardRenderer = React.memo(function CommandCardRenderer({
  dsl,
  onAction,
  isReadOnly,
  decision,
}: CommandCardRendererProps) {
  const [selectedAction, setSelectedAction] = useState<string | null>(null);

  const card = tryParseDSL(dsl);
  if (!card) {
    return (
      <div className="rounded-lg border border-border p-3 text-xs text-muted-foreground">
        <pre className="whitespace-pre-wrap">{dsl}</pre>
      </div>
    );
  }

  const handleAction = (action: string) => {
    setSelectedAction(action);
    onAction?.(action);
  };

  const showActions = !isReadOnly && !decision && !selectedAction;

  return (
    <div className="rounded-lg border border-border overflow-hidden">
      {/* Header */}
      <div className="px-3 py-2 bg-muted/50 border-b border-border flex items-center justify-between">
        <span className="text-xs font-medium text-foreground">{card.component}</span>
        {decision && <DecisionBadge decision={decision} />}
      </div>

      {/* Props */}
      <div className="p-3 space-y-1.5">
        {Object.entries(card.props).map(([key, value]) => (
          <div key={key} className="flex gap-2 text-xs">
            <span className="text-muted-foreground font-medium min-w-[80px]">{key}:</span>
            <span className="text-foreground font-mono">
              {typeof value === "object" ? JSON.stringify(value, null, 2) : String(value)}
            </span>
          </div>
        ))}
      </div>

      {/* Actions */}
      {showActions && card.actions && card.actions.length > 0 && (
        <div className="px-3 py-2 border-t border-border flex gap-2 justify-end">
          {card.actions.map((action) => (
            <button
              key={action.action}
              type="button"
              onClick={() => handleAction(action.action)}
              className={cn(
                "px-3 py-1 text-xs font-medium rounded-md transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                action.variant === "primary" &&
                  "bg-primary text-primary-foreground hover:opacity-90",
                action.variant === "destructive" &&
                  "bg-destructive text-destructive-foreground hover:bg-destructive/90",
                action.variant !== "primary" &&
                  action.variant !== "destructive" &&
                  "border border-border hover:bg-muted"
              )}
            >
              {action.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
});

function DecisionBadge({ decision }: { decision: string }) {
  const config: Record<string, { icon: typeof CheckIcon; label: string; className: string }> = {
    approved: {
      icon: CheckIcon,
      label: "Approved",
      className: "text-[var(--dot-emerald)] bg-[var(--accent-emerald)]",
    },
    cancelled: {
      icon: XIcon,
      label: "Cancelled",
      className: "text-[var(--dot-red)] bg-[var(--accent-red)]",
    },
    modification_requested: {
      icon: XIcon,
      label: "Modified",
      className: "text-[var(--dot-amber)] bg-[var(--accent-amber)]",
    },
  };

  const entry = config[decision];
  if (!entry) return null;

  const Icon = entry.icon;
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 px-2 py-0.5 rounded text-xs font-medium",
        entry.className
      )}
    >
      <Icon className="size-3" />
      {entry.label}
    </span>
  );
}
