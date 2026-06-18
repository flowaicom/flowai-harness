import { XIcon } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { IconButton } from "./primitives";
import { cn } from "./utils/cn";

const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export type DialogSize = "sm" | "md" | "lg";

const sizeClass: Record<DialogSize, string> = {
  sm: "max-w-sm",
  md: "max-w-lg",
  lg: "max-w-3xl",
};

export interface DialogProps {
  readonly open: boolean;
  readonly onClose: () => void;
  readonly labelledBy: string;
  readonly describedBy?: string;
  readonly size?: DialogSize;
  readonly dismissOnBackdrop?: boolean;
  readonly className?: string;
  readonly children: ReactNode;
}

export function Dialog({
  open,
  onClose,
  labelledBy,
  describedBy,
  size = "md",
  dismissOnBackdrop = true,
  className,
  children,
}: DialogProps) {
  const panelRef = useRef<HTMLDivElement | null>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!open) return;
    previousFocusRef.current = document.activeElement as HTMLElement | null;
    const panel = panelRef.current;
    const focusables = panel?.querySelectorAll<HTMLElement>(FOCUSABLE);
    focusables?.[0]?.focus();

    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        onClose();
        return;
      }
      if (event.key !== "Tab" || !panel) return;
      const items = Array.from(panel.querySelectorAll<HTMLElement>(FOCUSABLE));
      if (items.length === 0) return;
      const first = items[0];
      const last = items[items.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };

    document.addEventListener("keydown", onKey);
    const { body } = document;
    const prevOverflow = body.style.overflow;
    body.style.overflow = "hidden";
    return () => {
      document.removeEventListener("keydown", onKey);
      body.style.overflow = prevOverflow;
      previousFocusRef.current?.focus();
    };
  }, [open, onClose]);

  const onBackdrop = useCallback(() => {
    if (dismissOnBackdrop) onClose();
  }, [dismissOnBackdrop, onClose]);

  if (!open || typeof document === "undefined") return null;

  return createPortal(
    // biome-ignore lint/a11y/noStaticElementInteractions: backdrop only handles outside-click dismissal; Escape handles keyboard dismissal.
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4 backdrop-blur-sm"
      onMouseDown={onBackdrop}
    >
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={labelledBy}
        aria-describedby={describedBy}
        // biome-ignore lint/a11y/noStaticElementInteractions: stops backdrop dismissal for clicks inside the dialog.
        onMouseDown={(event) => event.stopPropagation()}
        className={cn(
          "flex max-h-[calc(100dvh-32px)] w-full flex-col overflow-hidden rounded-2xl border border-[var(--layer-10)] bg-[var(--chrome-card)] shadow-[0_32px_80px_-12px_rgba(0,0,0,0.7)]",
          sizeClass[size],
          className
        )}
      >
        {children}
      </div>
    </div>,
    document.body
  );
}

export function DialogHeader({
  id,
  title,
  description,
  onClose,
  icon,
}: {
  readonly id: string;
  readonly title: ReactNode;
  readonly description?: ReactNode;
  readonly onClose?: () => void;
  readonly icon?: ReactNode;
}) {
  return (
    <div className="flex items-start gap-3 border-b border-[var(--layer-06)] px-5 py-3.5">
      {icon ? (
        <div className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-lg bg-[var(--layer-06)] text-[var(--fg-2)]">
          {icon}
        </div>
      ) : null}
      <div className="min-w-0 flex-1">
        <h2 id={id} className="text-sm font-medium text-[var(--fg-1)]">
          {title}
        </h2>
        {description ? (
          <p className="mt-0.5 text-xs leading-5 text-[var(--fg-5)]">{description}</p>
        ) : null}
      </div>
      {onClose ? (
        <IconButton label="Close" size="sm" onClick={onClose}>
          <XIcon className="size-4" />
        </IconButton>
      ) : null}
    </div>
  );
}

export function DialogBody({
  id,
  className,
  children,
}: {
  readonly id?: string;
  readonly className?: string;
  readonly children: ReactNode;
}) {
  return (
    <div id={id} className={cn("min-h-0 flex-1 overflow-y-auto px-5 py-4", className)}>
      {children}
    </div>
  );
}

export function DialogFooter({
  className,
  children,
}: {
  readonly className?: string;
  readonly children: ReactNode;
}) {
  return (
    <div
      className={cn(
        "flex items-center justify-end gap-2 border-t border-[var(--layer-06)] bg-[var(--chrome-base)] px-5 py-3",
        className
      )}
    >
      {children}
    </div>
  );
}
