/**
 * Workspace creation dialog with database type selection.
 *
 * Replaces the old `window.prompt()` flow. Users can choose between:
 * - Local SQLite (default, zero-config)
 * - NeonDB branch (serverless Postgres, requires credentials in Settings)
 * - External database (user-provided connection URL)
 *
 * Creating a workspace always creates a parent + 4 child databases.
 *
 * @module components/workspace/create-workspace-dialog
 */

import { Loader2Icon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "~/components/ui/dialog";
import { createWorkspace, provisionWorkspaces } from "~/lib/api/workspaces";
import { isOk } from "~/lib/domain/result";
import type { DerivedWorkspaceUrl, Workspace, WorkspaceDatabaseType } from "~/lib/domain/workspace";
import { deriveWorkspaceUrls } from "~/lib/domain/workspace";
import { useAgentConfig } from "~/lib/stores/settings-store";
import { cn } from "~/lib/utils";

// ============================================================================
// Types
// ============================================================================

export interface CreateWorkspaceDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (workspace: Workspace) => void;
}

// ============================================================================
// Main Component
// ============================================================================

export function CreateWorkspaceDialog({
  open,
  onOpenChange,
  onCreated,
}: CreateWorkspaceDialogProps) {
  const [name, setName] = useState("");
  const [dbType, setDbType] = useState<WorkspaceDatabaseType>("sqlite");
  const [externalUrl, setExternalUrl] = useState("");
  const [isCreating, setIsCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [derivedUrls, setDerivedUrls] = useState<DerivedWorkspaceUrl[] | null>(null);
  const nameInputRef = useRef<HTMLInputElement>(null);

  const neondbApiKey = useAgentConfig((s) => s.neondbApiKey);
  const neondbProjectId = useAgentConfig((s) => s.neondbProjectId);
  const hasNeondbCredentials = !!neondbApiKey && !!neondbProjectId;

  // Reset on open
  useEffect(() => {
    if (open) {
      setName("");
      setDbType("sqlite");
      setExternalUrl("");
      setError(null);
      setIsCreating(false);
      setDerivedUrls(null);
      setTimeout(() => nameInputRef.current?.focus(), 50);
    }
  }, [open]);

  // Live preview of derived URLs when external URL changes
  useEffect(() => {
    if (dbType === "external" && externalUrl.trim()) {
      setDerivedUrls(deriveWorkspaceUrls(externalUrl));
    } else {
      setDerivedUrls(null);
    }
  }, [dbType, externalUrl]);

  const isProvisionMode = dbType === "external" && derivedUrls !== null;

  const canCreate =
    name.trim().length > 0 &&
    !isCreating &&
    (dbType !== "external" || externalUrl.trim().length > 0);

  const handleCreate = async () => {
    if (!canCreate) return;
    setIsCreating(true);
    setError(null);

    if (isProvisionMode) {
      const result = await provisionWorkspaces({
        databaseUrl: externalUrl.trim(),
        displayNamePrefix: name.trim() || undefined,
      });
      if (isOk(result)) {
        onCreated(result.value);
        onOpenChange(false);
      } else {
        setError(result.error.message);
      }
    } else {
      const result = await createWorkspace({
        displayName: name.trim(),
        databaseType: dbType,
        databaseUrl: dbType === "external" ? externalUrl.trim() : undefined,
        neondbApiKey: dbType === "neondb" ? neondbApiKey : undefined,
        neondbProjectId: dbType === "neondb" ? neondbProjectId : undefined,
      });
      if (isOk(result)) {
        onCreated(result.value);
        onOpenChange(false);
      } else {
        setError(result.error.message);
      }
    }
    setIsCreating(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Create Workspace</DialogTitle>
          <DialogDescription>
            Set up a new workspace with its own set of databases for scenarios and data.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-4">
          {/* Name */}
          <div className="space-y-2">
            <label htmlFor="ws-name" className="text-sm font-medium">
              Name
            </label>
            <input
              ref={nameInputRef}
              id="ws-name"
              type="text"
              placeholder="My Workspace"
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && handleCreate()}
              className={cn(
                "w-full text-sm px-3 py-2 rounded-md border bg-background",
                "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                "placeholder:text-muted-foreground/50"
              )}
            />
          </div>

          {/* Database type */}
          <div className="space-y-2">
            <label className="text-sm font-medium">Database</label>
            <div className="space-y-2">
              {/* SQLite */}
              <label
                className={cn(
                  "flex items-start gap-3 p-3 rounded-md border cursor-pointer transition-colors",
                  dbType === "sqlite" ? "border-primary bg-primary/5" : "hover:bg-muted/50"
                )}
              >
                <input
                  type="radio"
                  name="dbType"
                  value="sqlite"
                  checked={dbType === "sqlite"}
                  onChange={() => setDbType("sqlite")}
                  className="mt-0.5"
                />
                <div className="flex-1">
                  <p className="text-sm font-medium">Local SQLite</p>
                  <p className="text-xs text-muted-foreground">
                    Auto-created on disk. Best for local development.
                  </p>
                </div>
              </label>

              {/* NeonDB */}
              <label
                className={cn(
                  "flex items-start gap-3 p-3 rounded-md border transition-colors",
                  !hasNeondbCredentials && "opacity-50 cursor-not-allowed",
                  hasNeondbCredentials && "cursor-pointer",
                  dbType === "neondb"
                    ? "border-primary bg-primary/5"
                    : hasNeondbCredentials
                      ? "hover:bg-muted/50"
                      : ""
                )}
                title={
                  !hasNeondbCredentials
                    ? "Configure NeonDB credentials in Settings → Infrastructure"
                    : undefined
                }
              >
                <input
                  type="radio"
                  name="dbType"
                  value="neondb"
                  checked={dbType === "neondb"}
                  onChange={() => setDbType("neondb")}
                  disabled={!hasNeondbCredentials}
                  className="mt-0.5"
                />
                <div className="flex-1">
                  <p className="text-sm font-medium">NeonDB Branch</p>
                  <p className="text-xs text-muted-foreground">
                    {hasNeondbCredentials
                      ? "Serverless Postgres. Will provision a new branch on create."
                      : "Requires API key in Settings → Infrastructure."}
                  </p>
                </div>
              </label>

              {/* External */}
              <label
                className={cn(
                  "flex items-start gap-3 p-3 rounded-md border cursor-pointer transition-colors",
                  dbType === "external" ? "border-primary bg-primary/5" : "hover:bg-muted/50"
                )}
              >
                <input
                  type="radio"
                  name="dbType"
                  value="external"
                  checked={dbType === "external"}
                  onChange={() => setDbType("external")}
                  className="mt-0.5"
                />
                <div className="flex-1">
                  <p className="text-sm font-medium">External Database</p>
                  <p className="text-xs text-muted-foreground">
                    Provide your own PostgreSQL connection URL.
                  </p>
                </div>
              </label>
            </div>
          </div>

          {/* External URL input (conditional) */}
          {dbType === "external" && (
            <div className="space-y-2">
              <label htmlFor="ws-db-url" className="text-sm font-medium">
                Connection URL
              </label>
              <input
                id="ws-db-url"
                type="text"
                placeholder="postgresql://user:pass@host:5432/dbname"
                value={externalUrl}
                onChange={(e) => setExternalUrl(e.target.value)}
                className={cn(
                  "w-full text-sm px-3 py-2 rounded-md border bg-background font-mono",
                  "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                  "placeholder:text-muted-foreground/50"
                )}
              />
            </div>
          )}

          {/* Derived URL preview */}
          {derivedUrls && (
            <div className="space-y-1.5 rounded-md border bg-muted/30 p-3">
              <p className="text-xs font-medium text-muted-foreground">Will create 4 databases:</p>
              {derivedUrls.map((d) => (
                <div key={d.role} className="flex items-baseline gap-2 text-[11px]">
                  <span className="w-20 shrink-0 font-medium text-foreground">{d.role}</span>
                  <span className="font-mono text-muted-foreground truncate">{d.databaseUrl}</span>
                </div>
              ))}
            </div>
          )}

          {/* Error */}
          {error && <p className="text-sm text-destructive">{error}</p>}
        </div>

        {/* Actions */}
        <div className="flex items-center justify-end gap-2 pt-4 border-t">
          <button
            type="button"
            onClick={() => onOpenChange(false)}
            disabled={isCreating}
            className="px-4 py-2 text-sm rounded-md border hover:bg-muted transition-colors"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={handleCreate}
            disabled={!canCreate}
            className={cn(
              "flex items-center gap-2 px-4 py-2 text-sm rounded-md transition-colors",
              "bg-primary text-primary-foreground hover:bg-primary/90",
              !canCreate && "opacity-50 cursor-not-allowed"
            )}
          >
            {isCreating && <Loader2Icon className="size-4 animate-spin" />}
            {isProvisionMode ? "Provision Workspace" : "Create"}
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
