/**
 * Inline source picker dropdown for Connect subsections.
 *
 * Shows the active data source and lets the user switch between sources.
 * Same visual style as ProfilingModelSelector (px-3 py-1.5 text-xs border rounded-md).
 *
 * @module components/connect/source-picker
 */

import { ChevronDownIcon, DatabaseIcon } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import type { WorkspaceRole } from "~/lib/domain/workspace";
import { ROLE_LABELS, WORKSPACE_ROLES } from "~/lib/domain/workspace";
import { selectSources, useSourceCatalog } from "~/lib/stores/source-catalog";
import { cn } from "~/lib/utils";

interface SourcePickerProps {
  readonly sourceId: string | undefined;
  readonly onSourceChange: (id: string) => void;
  readonly disabled?: boolean;
}

/** Check if a source ID matches a known workspace role. */
function sourceRole(id: string): WorkspaceRole | null {
  return (WORKSPACE_ROLES as readonly string[]).includes(id) ? (id as WorkspaceRole) : null;
}

/** Format display label: "neondb — Target" or just the source name. */
function sourceLabel(id: string, name: string): string {
  const role = sourceRole(id);
  return role ? `${name} — ${ROLE_LABELS[role]}` : name;
}

const DB_ROLES = new Set<WorkspaceRole>(["target", "catalog"]);
const INFRA_ROLES = new Set<WorkspaceRole>(["embeddings", "workspace"]);

export function SourcePicker({ sourceId, onSourceChange, disabled }: SourcePickerProps) {
  const sources = useSourceCatalog(selectSources);
  const [isOpen, setIsOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  const selected = sources.find((s) => s.id === sourceId);

  // Close on outside click or Escape
  useEffect(() => {
    if (!isOpen) return;
    const handleClick = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setIsOpen(false);
    };
    document.addEventListener("mousedown", handleClick);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleClick);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [isOpen]);

  const handleSelect = useCallback(
    (id: string) => {
      onSourceChange(id);
      setIsOpen(false);
    },
    [onSourceChange]
  );

  // Group sources into Databases vs Infrastructure
  const databases = sources.filter((s) => {
    const role = sourceRole(s.id);
    return !role || DB_ROLES.has(role);
  });
  const infrastructure = sources.filter((s) => {
    const role = sourceRole(s.id);
    return role != null && INFRA_ROLES.has(role);
  });

  return (
    <div className="relative" ref={dropdownRef}>
      <button
        type="button"
        onClick={() => !disabled && setIsOpen(!isOpen)}
        disabled={disabled}
        aria-expanded={isOpen}
        aria-label="Select data source"
        className="flex items-center gap-2 px-3 py-1.5 text-xs border rounded-md hover:bg-muted transition-colors disabled:opacity-50 min-w-[180px] focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      >
        <DatabaseIcon className="size-3.5 text-muted-foreground shrink-0" />
        <span className="text-muted-foreground">Source:</span>
        <span className="font-medium truncate">
          {selected ? sourceLabel(selected.id, selected.name) : "None"}
        </span>
        <ChevronDownIcon className="size-3.5 ml-auto shrink-0 text-muted-foreground" />
      </button>

      {isOpen && (
        <div className="absolute top-full left-0 mt-1 w-[280px] bg-popover border rounded-md shadow-lg z-50 max-h-[300px] overflow-y-auto scroll-container">
          {/* Databases group */}
          {databases.length > 0 && (
            <>
              <div className="px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground bg-muted/50">
                Databases
              </div>
              {databases.map((s) => (
                <button
                  key={s.id}
                  type="button"
                  onClick={() => handleSelect(s.id)}
                  className={cn(
                    "w-full text-left px-3 py-2 text-xs hover:bg-muted transition-colors flex items-center justify-between focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                    s.id === sourceId && "bg-primary/5 font-medium"
                  )}
                >
                  <span className="truncate">{sourceLabel(s.id, s.name)}</span>
                  {sourceRole(s.id) && (
                    <span className="text-muted-foreground text-[10px] shrink-0 ml-2">
                      {sourceRole(s.id)}
                    </span>
                  )}
                </button>
              ))}
            </>
          )}

          {/* Infrastructure group */}
          {infrastructure.length > 0 && (
            <>
              <div className="px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground bg-muted/50">
                Infrastructure
              </div>
              {infrastructure.map((s) => (
                <button
                  key={s.id}
                  type="button"
                  onClick={() => handleSelect(s.id)}
                  className={cn(
                    "w-full text-left px-3 py-2 text-xs hover:bg-muted transition-colors flex items-center justify-between focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                    s.id === sourceId && "bg-primary/5 font-medium"
                  )}
                >
                  <span className="truncate">{sourceLabel(s.id, s.name)}</span>
                  <span className="text-muted-foreground text-[10px] shrink-0 ml-2">
                    {sourceRole(s.id)}
                  </span>
                </button>
              ))}
            </>
          )}
        </div>
      )}
    </div>
  );
}
