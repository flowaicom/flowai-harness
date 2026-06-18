/**
 * Activity Center — floating indicator for active background sessions.
 *
 * Renders a compact pill (fixed bottom-left) when any session is active.
 * Clicking toggles an expanded panel listing all sessions from the
 * session registry. Each row links to the relevant page.
 *
 * Sessions with a `jobId` show a cancel button that fires
 * `POST /api/jobs/{jobId}/cancel` and marks the session terminal.
 *
 * Subscribes to the session registry store — re-renders only when
 * session count changes.
 *
 * @module components/shared/background-tasks-indicator
 */

import { StopCircleIcon, XIcon } from "lucide-react";
import { useCallback, useState } from "react";
import { useNavigate } from "react-router";
import { apiFetch } from "~/lib/api/client";
import { isOk } from "~/lib/domain/result";
import {
  type SessionEntry,
  useActiveSessionCount,
  useActiveSessionsForWorkspace,
  useOtherWorkspaceSessionCount,
  useSessionRegistry,
} from "~/lib/stores";

const KIND_ICONS: Record<string, string> = {
  "chat-stream": "\u{1F4AC}",
  "eval-run": "\u{1F9EA}",
  profiling: "\u{1F4CA}",
  import: "\u{1F4E5}",
  builder: "\u{1F6E0}",
};

function formatElapsed(startedAt: number): string {
  const seconds = Math.floor((Date.now() - startedAt) / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const secs = seconds % 60;
  return `${minutes}:${secs.toString().padStart(2, "0")}`;
}

function SessionRow({
  session,
  onNavigate,
  onCancel,
}: {
  readonly session: SessionEntry;
  readonly onNavigate: (to: string) => void;
  readonly onCancel: (session: SessionEntry) => void;
}) {
  const icon = KIND_ICONS[session.kind] ?? "\u{2699}";

  return (
    <div className="w-full flex items-center gap-2.5 px-3 py-2 rounded-md hover:bg-muted/60 transition-colors text-left">
      <span className="text-sm shrink-0">{icon}</span>
      <button
        type="button"
        onClick={() => onNavigate(session.routeTo)}
        className="flex-1 min-w-0 text-left"
      >
        <div className="text-xs font-medium truncate">{session.label}</div>
        <div className="text-[10px] text-muted-foreground tabular-nums">
          {formatElapsed(session.startedAt)}
        </div>
      </button>
      {session.jobId && session.status === "active" && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onCancel(session);
          }}
          className="p-0.5 rounded hover:bg-destructive/10 transition-colors shrink-0 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          aria-label={`Cancel ${session.label}`}
          title="Cancel"
        >
          <StopCircleIcon className="size-3.5 text-destructive/70 hover:text-destructive" />
        </button>
      )}
      <button
        type="button"
        onClick={() => onNavigate(session.routeTo)}
        className="text-[10px] text-primary hover:underline shrink-0 rounded focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      >
        View
      </button>
    </div>
  );
}

export function BackgroundTasksIndicator() {
  const navigate = useNavigate();
  const count = useActiveSessionCount();
  const sessions = useActiveSessionsForWorkspace();
  const otherCount = useOtherWorkspaceSessionCount();
  const [isExpanded, setIsExpanded] = useState(false);

  const handleNavigate = useCallback(
    (to: string) => {
      setIsExpanded(false);
      navigate(to);
    },
    [navigate]
  );

  const handleCancel = useCallback(async (session: SessionEntry) => {
    if (!session.jobId) return;
    const result = await apiFetch(`/api/jobs/${session.jobId}/cancel`, {
      method: "POST",
    });
    if (isOk(result)) {
      useSessionRegistry.getState().markTerminal(session.id, "cancelled");
    }
  }, []);

  if (count === 0 && otherCount === 0) return null;

  const totalCount = count + otherCount;
  const label = totalCount === 1 ? "1 active" : `${totalCount} active`;

  if (!isExpanded) {
    return (
      <button
        type="button"
        onClick={() => setIsExpanded(true)}
        className="fixed bottom-4 left-4 z-50 flex items-center gap-2 bg-foreground/90 text-background text-xs font-medium px-3 py-1.5 rounded-full shadow-lg backdrop-blur-sm hover:bg-foreground transition-colors animate-in fade-in-0 duration-300"
        title={`${label} — click to view`}
      >
        <span className="relative flex size-2">
          <span className="absolute inline-flex size-full animate-ping rounded-full bg-[var(--dot-blue)] opacity-75" />
          <span className="relative inline-flex size-2 rounded-full bg-[var(--dot-blue)]" />
        </span>
        {label}
      </button>
    );
  }

  return (
    <div className="fixed bottom-4 left-4 z-50 w-72 bg-popover border rounded-xl shadow-xl animate-in fade-in-0 slide-in-from-bottom-2 duration-200">
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2.5 border-b">
        <span className="text-xs font-semibold">Active Sessions ({totalCount})</span>
        <button
          type="button"
          onClick={() => setIsExpanded(false)}
          className="p-0.5 rounded hover:bg-muted transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          aria-label="Close activity center"
        >
          <XIcon className="size-3.5" />
        </button>
      </div>

      {/* Session list */}
      <div className="p-1.5 max-h-64 overflow-y-auto scroll-container space-y-0.5">
        {sessions.map((session) => (
          <SessionRow
            key={session.id}
            session={session}
            onNavigate={handleNavigate}
            onCancel={handleCancel}
          />
        ))}
        {otherCount > 0 && (
          <div className="px-3 py-1.5 text-[10px] text-muted-foreground/60">
            + {otherCount} in other workspace{otherCount > 1 ? "s" : ""}
          </div>
        )}
      </div>
    </div>
  );
}
