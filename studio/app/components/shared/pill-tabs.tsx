/**
 * Pill tab bar — generic horizontal tab selector.
 *
 * Parameterized over a string union K for type-safe tab IDs.
 * Matches eval sample-tab-bar styling: rounded-full pills with
 * optional count badges.
 *
 * @module components/shared/pill-tabs
 */

import { useCallback, useRef } from "react";
import { cn } from "~/lib/utils";

interface Tab<K extends string> {
  readonly id: K;
  readonly label: string;
  readonly count?: number;
}

interface PillTabsProps<K extends string> {
  readonly tabs: readonly Tab<K>[];
  readonly active: K;
  readonly onChange: (id: K) => void;
  readonly className?: string;
}

export function PillTabs<K extends string>({
  tabs,
  active,
  onChange,
  className,
}: PillTabsProps<K>) {
  const containerRef = useRef<HTMLDivElement>(null);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      const idx = tabs.findIndex((t) => t.id === active);
      if (idx === -1) return;
      let next = -1;
      if (e.key === "ArrowRight") next = (idx + 1) % tabs.length;
      else if (e.key === "ArrowLeft") next = (idx - 1 + tabs.length) % tabs.length;
      if (next !== -1) {
        e.preventDefault();
        onChange(tabs[next].id);
        const buttons = containerRef.current?.querySelectorAll<HTMLButtonElement>('[role="tab"]');
        buttons?.[next]?.focus();
      }
    },
    [tabs, active, onChange]
  );

  return (
    <div
      ref={containerRef}
      className={cn("flex items-center gap-1", className)}
      role="tablist"
      onKeyDown={handleKeyDown}
    >
      {tabs.map((tab) => {
        const selected = active === tab.id;
        return (
          <button
            key={tab.id}
            type="button"
            role="tab"
            aria-selected={selected}
            tabIndex={selected ? 0 : -1}
            onClick={() => onChange(tab.id)}
            className={cn(
              "px-3 py-1 rounded-md text-xs font-medium transition-colors whitespace-nowrap focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
              selected
                ? "bg-foreground/8 text-foreground"
                : "text-muted-foreground hover:bg-muted/60 hover:text-foreground"
            )}
          >
            {tab.label}
            {tab.count != null && (
              <span className={cn("ml-1.5 tabular-nums", selected ? "opacity-80" : "opacity-60")}>
                {tab.count}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}
