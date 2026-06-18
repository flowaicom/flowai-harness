/**
 * Error banner — standardized dismissible error display.
 *
 * Law L6: Every fetch has 4 explicit states (loading, error, empty, populated).
 * This component handles the "error" state consistently.
 *
 * @module components/shared/error-banner
 */

import { RotateCcwIcon, XIcon } from "lucide-react";
import { cn } from "~/lib/utils";

interface ErrorBannerProps {
  readonly message: string;
  readonly onDismiss: () => void;
  readonly onRetry?: () => void;
  readonly className?: string;
}

export function ErrorBanner({ message, onDismiss, onRetry, className }: ErrorBannerProps) {
  return (
    <div
      role="alert"
      className={cn(
        "flex items-center justify-between rounded-lg accent-bar-red bg-[var(--accent-red)] px-4 py-2.5 text-[var(--dot-red)] text-sm",
        className
      )}
    >
      <span className="flex-1 min-w-0">{message}</span>
      <div className="ml-3 flex items-center gap-1 shrink-0">
        {onRetry && (
          <button
            type="button"
            onClick={onRetry}
            className="p-0.5 rounded text-[var(--dot-red)]/60 hover:text-[var(--dot-red)] transition-colors"
            aria-label="Retry"
          >
            <RotateCcwIcon className="size-4" />
          </button>
        )}
        <button
          type="button"
          onClick={onDismiss}
          className="p-0.5 rounded text-[var(--dot-red)]/60 hover:text-[var(--dot-red)] transition-colors"
          aria-label="Dismiss error"
        >
          <XIcon className="size-4" />
        </button>
      </div>
    </div>
  );
}
