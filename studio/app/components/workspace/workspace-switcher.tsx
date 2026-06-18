/**
 * Workspace switcher — fixed pebble in the top-right corner of the viewport.
 *
 * Shows a small circle with the first letter of the project name.
 * Clicking opens a dropdown showing workspaces, each with its
 * component databases (catalog, embeddings, target, workspace).
 *
 * A workspace is a full setup — all sub-databases together form one unit.
 *
 * @module components/workspace/workspace-switcher
 */

import { CheckIcon, ChevronDownIcon, DatabaseIcon, LayersIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { getProjectConfig } from "~/lib/api/studio";
import { isOk } from "~/lib/domain/result";
import {
  missingBundleRolesLabel,
  workspaceBundleComplete,
  workspaceBundleLabel,
} from "~/lib/domain/workspace";
import { useHarnessRuntime, workspaceSummaryToWorkspace } from "~/lib/runtime";
import {
  selectActiveWorkspaceId,
  selectSetActiveWorkspace,
  selectSetWorkspaceListState,
  selectSetWorkspaces,
  selectWorkspaceIsLoading,
  selectWorkspaces,
  useWorkspace,
} from "~/lib/stores/workspace-store";
import { cn } from "~/lib/utils";

// ============================================================================
// Main Component
// ============================================================================

export interface WorkspaceSwitcherProps {
  readonly variant?: "fixed" | "inline";
  readonly className?: string;
}

export function WorkspaceSwitcher({ variant = "fixed", className }: WorkspaceSwitcherProps) {
  const workspaces = useWorkspace(selectWorkspaces);
  const activeWorkspaceId = useWorkspace(selectActiveWorkspaceId);
  const isLoading = useWorkspace(selectWorkspaceIsLoading);
  const setWorkspaces = useWorkspace(selectSetWorkspaces);
  const setActiveWorkspace = useWorkspace(selectSetActiveWorkspace);
  const setListState = useWorkspace(selectSetWorkspaceListState);
  const { adapter } = useHarnessRuntime();

  const [isOpen, setIsOpen] = useState(false);
  const [projectName, setProjectName] = useState<string | null>(null);
  const dropdownRef = useRef<HTMLDivElement>(null);

  // Load workspaces + project config on mount
  useEffect(() => {
    const load = async () => {
      setListState({ status: "loading" });
      const [wsResult, projResult] = await Promise.all([
        adapter.listWorkspaces(),
        getProjectConfig(),
      ]);
      if (isOk(wsResult)) {
        setWorkspaces(wsResult.value.workspaces.map(workspaceSummaryToWorkspace));
      } else {
        setListState({ status: "error", error: wsResult.error.message });
      }
      if (isOk(projResult)) {
        setProjectName(projResult.value.config.project.name);
      }
    };
    load();
  }, [setWorkspaces, setListState, adapter]);

  // Close dropdown on outside click
  useEffect(() => {
    if (!isOpen) return;
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener("mousedown", handler as unknown as EventListener);
    return () => document.removeEventListener("mousedown", handler as unknown as EventListener);
  }, [isOpen]);

  // Close on Escape
  useEffect(() => {
    if (!isOpen) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") setIsOpen(false);
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [isOpen]);

  const displayName = projectName ?? "Workspace";
  const initial = displayName.charAt(0).toUpperCase();
  const activeWorkspace = workspaces.find((workspace) => workspace.id === activeWorkspaceId);
  const dropdownAlignment = variant === "inline" ? "left-0" : "right-0";

  return (
    <div
      ref={dropdownRef}
      className={cn(variant === "fixed" ? "fixed top-2.5 right-3 z-50" : "relative", className)}
    >
      {/* Pebble trigger */}
      <button
        type="button"
        onClick={() => setIsOpen((prev) => !prev)}
        disabled={isLoading}
        title={displayName}
        className={cn(
          "transition-colors disabled:opacity-50 disabled:cursor-not-allowed",
          variant === "fixed"
            ? "flex size-7 items-center justify-center rounded-full bg-muted text-foreground hover:bg-muted/80"
            : "flex w-full items-center gap-2 rounded-[10px] border border-[var(--layer-08)] bg-[var(--layer-04)] px-2.5 py-2 text-left text-[var(--fg-1)] hover:bg-[var(--layer-06)]",
          isOpen && "ring-2 ring-ring"
        )}
      >
        {variant === "fixed" ? (
          <span className="text-[10px] font-semibold leading-none">{initial}</span>
        ) : (
          <>
            <span className="flex size-6 shrink-0 items-center justify-center rounded-md bg-[var(--chrome-raised)] text-[10px] font-semibold">
              {initial}
            </span>
            <span className="min-w-0 flex-1">
              <span className="studio-eyebrow block text-[9px]">Workspace</span>
              <span className="block truncate text-xs font-medium">
                {activeWorkspace?.displayName ?? activeWorkspaceId ?? displayName}
              </span>
            </span>
            <ChevronDownIcon className="size-3.5 shrink-0 text-[var(--fg-5)]" />
          </>
        )}
      </button>

      {/* Dropdown — right-aligned */}
      {isOpen && (
        <div
          className={cn(
            "absolute top-full z-50 mt-1.5 w-64 overflow-hidden rounded-lg border bg-popover shadow-lg",
            dropdownAlignment
          )}
        >
          {/* Workspaces list */}
          <div className="py-1">
            <div className="px-3 pt-1.5 pb-1">
              <span className="text-[10px] font-medium text-muted-foreground/60 uppercase tracking-wider">
                Workspaces
              </span>
            </div>
            {workspaces.map((ws) => (
              <div key={ws.id}>
                {/* Workspace header — clickable to switch */}
                <button
                  type="button"
                  onClick={() => {
                    setActiveWorkspace(ws.id);
                    setIsOpen(false);
                  }}
                  className={cn(
                    "flex items-center gap-2 w-full px-3 py-1.5 text-left transition-colors",
                    ws.id === activeWorkspaceId
                      ? "bg-primary/5 text-primary"
                      : "hover:bg-muted text-foreground"
                  )}
                >
                  <LayersIcon className="size-3.5 shrink-0" />
                  <span className="text-xs font-medium truncate flex-1">{ws.displayName}</span>
                  <span
                    className={cn(
                      "rounded-full border px-1.5 py-0.5 text-[9px] uppercase tracking-wide",
                      workspaceBundleComplete(ws)
                        ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
                        : "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300"
                    )}
                  >
                    {workspaceBundleLabel(ws)}
                  </span>
                  {ws.id === activeWorkspaceId && <CheckIcon className="size-3 shrink-0" />}
                </button>

                {/* Sub-databases — read-only info, only shown for active */}
                {ws.id === activeWorkspaceId && ws.databases.length > 0 && (
                  <div className="pl-6 pr-3 pb-1">
                    {!workspaceBundleComplete(ws) && (
                      <p className="py-1 text-[10px] text-amber-700 dark:text-amber-300">
                        Missing bundle roles: {missingBundleRolesLabel(ws)}
                      </p>
                    )}
                    {ws.databases.map((db) => (
                      <div
                        key={db.id}
                        className="flex items-center gap-2 py-0.5 text-[11px] text-muted-foreground"
                      >
                        <DatabaseIcon className="size-2.5 shrink-0 opacity-50" />
                        <span className="truncate flex-1">{db.displayName}</span>
                        {db.databaseUrl && (
                          <span className="text-[9px] text-muted-foreground/40 font-mono shrink-0">
                            {dbTypeLabel(db.databaseUrl)}
                          </span>
                        )}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            ))}
            {workspaces.length === 0 && !isLoading && (
              <p className="px-3 py-1.5 text-xs text-muted-foreground/60">
                No workspaces configured
              </p>
            )}
          </div>

          <div className="border-t px-3 py-2 text-[11px] text-muted-foreground/70">
            Workspaces are registered by the harness app.
          </div>
        </div>
      )}
    </div>
  );
}

/** Extract a short label from a database URL. */
function dbTypeLabel(url: string): string {
  if (url.startsWith("sqlite:")) return "sqlite";
  if (url.startsWith("postgresql://") || url.startsWith("postgres://")) return "pg";
  return "";
}
