/**
 * Minimal Dialog component matching shadcn/ui Dialog API surface.
 *
 * Uses React Portal + overlay pattern (no Radix dependency).
 * Supports: Dialog, DialogTrigger, DialogContent, DialogHeader, DialogTitle, DialogDescription.
 *
 * @module components/ui/dialog
 */

import {
  createContext,
  type MouseEvent,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useId,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { cn } from "~/lib/utils";

// ============================================================================
// Context
// ============================================================================

interface DialogContextValue {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  titleId?: string;
  descriptionId?: string;
}

const DialogContext = createContext<DialogContextValue>({
  open: false,
  onOpenChange: () => {},
});

// ============================================================================
// Dialog (root wrapper)
// ============================================================================

interface DialogProps {
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  children: ReactNode;
}

export function Dialog({ open: controlledOpen, onOpenChange, children }: DialogProps) {
  const [internalOpen, setInternalOpen] = useState(false);
  const isControlled = controlledOpen !== undefined;
  const open = isControlled ? controlledOpen : internalOpen;

  const handleOpenChange = useCallback(
    (nextOpen: boolean) => {
      if (!isControlled) setInternalOpen(nextOpen);
      onOpenChange?.(nextOpen);
    },
    [isControlled, onOpenChange]
  );

  return (
    <DialogContext.Provider value={{ open, onOpenChange: handleOpenChange }}>
      {children}
    </DialogContext.Provider>
  );
}

// ============================================================================
// DialogTrigger
// ============================================================================

interface DialogTriggerProps {
  asChild?: boolean;
  children: ReactNode;
}

export function DialogTrigger({ children }: DialogTriggerProps) {
  const { onOpenChange } = useContext(DialogContext);

  return (
    <button
      type="button"
      onClick={() => onOpenChange(true)}
      className="inline-flex appearance-none bg-transparent border-0 p-0 cursor-pointer"
    >
      {children}
    </button>
  );
}

// ============================================================================
// DialogContent (portal + overlay)
// ============================================================================

interface DialogContentProps {
  className?: string;
  children: ReactNode;
}

export function DialogContent({ className, children }: DialogContentProps) {
  const ctx = useContext(DialogContext);
  const { open, onOpenChange } = ctx;
  const contentRef = useRef<HTMLDivElement>(null);
  const reactId = useId();
  const titleId = `${reactId}-title`;
  const descriptionId = `${reactId}-desc`;

  // Close on Escape
  useEffect(() => {
    if (!open) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onOpenChange(false);
      }
    };

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [open, onOpenChange]);

  // Prevent body scroll when open
  useEffect(() => {
    if (open) {
      document.body.style.overflow = "hidden";
      return () => {
        document.body.style.overflow = "";
      };
    }
  }, [open]);

  if (!open) return null;

  const handleOverlayClick = (e: MouseEvent) => {
    if (e.target === e.currentTarget) {
      onOpenChange(false);
    }
  };

  return createPortal(
    // biome-ignore lint/a11y/noStaticElementInteractions: backdrop overlay
    // biome-ignore lint/a11y/useKeyWithClickEvents: Escape key handler covers keyboard dismissal
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      onClick={handleOverlayClick}
    >
      {/* Overlay */}
      <div className="fixed inset-0 bg-black/50" />

      {/* Content */}
      <div
        ref={contentRef}
        className={cn(
          "relative z-50 w-full rounded-lg border bg-background p-6 shadow-lg",
          "animate-in fade-in-0 zoom-in-95",
          className
        )}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={descriptionId}
      >
        {/* Close button */}
        <button
          type="button"
          onClick={() => onOpenChange(false)}
          className="absolute right-4 top-4 rounded-sm opacity-70 ring-offset-background transition-opacity hover:opacity-100 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          aria-label="Close"
        >
          <svg
            className="size-4"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            aria-hidden="true"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M6 18L18 6M6 6l12 12"
            />
          </svg>
        </button>

        <DialogContext.Provider value={{ open, onOpenChange, titleId, descriptionId }}>
          {children}
        </DialogContext.Provider>
      </div>
    </div>,
    document.body
  );
}

// ============================================================================
// DialogHeader, DialogTitle, DialogDescription
// ============================================================================

interface DialogHeaderProps {
  children: ReactNode;
  className?: string;
}

export function DialogHeader({ children, className }: DialogHeaderProps) {
  return (
    <div className={cn("flex flex-col space-y-1.5 text-center sm:text-left", className)}>
      {children}
    </div>
  );
}

interface DialogTitleProps {
  children: ReactNode;
  className?: string;
}

export function DialogTitle({ children, className }: DialogTitleProps) {
  const { titleId } = useContext(DialogContext);
  return (
    <h2 id={titleId} className={cn("text-lg font-semibold leading-none tracking-tight", className)}>
      {children}
    </h2>
  );
}

interface DialogDescriptionProps {
  children: ReactNode;
  className?: string;
}

export function DialogDescription({ children, className }: DialogDescriptionProps) {
  const { descriptionId } = useContext(DialogContext);
  return (
    <p id={descriptionId} className={cn("text-sm text-muted-foreground", className)}>
      {children}
    </p>
  );
}
