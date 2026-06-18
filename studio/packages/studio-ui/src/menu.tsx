import type { LucideIcon } from "lucide-react";
import {
  type ButtonHTMLAttributes,
  cloneElement,
  type ReactElement,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { cn } from "./utils/cn";

type Align = "left" | "right";

export interface MenuProps {
  readonly trigger: ReactElement<ButtonHTMLAttributes<HTMLButtonElement>>;
  readonly align?: Align;
  readonly children: ReactNode;
  readonly className?: string;
}

export function Menu({ trigger, align = "right", children, className }: MenuProps) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDoc = (event: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(false);
        wrapRef.current?.querySelector<HTMLButtonElement>("button")?.focus();
      }
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const first = menuRef.current?.querySelector<HTMLButtonElement>(
      '[role="menuitem"]:not([disabled])'
    );
    first?.focus();
  }, [open]);

  const triggerOnClick = trigger.props.onClick;
  const triggerEl = cloneElement(trigger, {
    onClick: (event) => {
      triggerOnClick?.(event);
      setOpen((prev) => !prev);
    },
    "aria-haspopup": "menu",
    "aria-expanded": open,
  });

  const onMenuKey = useCallback((event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    const items = Array.from(
      menuRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not([disabled])') ??
        []
    );
    if (items.length === 0) return;
    const active = document.activeElement as HTMLButtonElement | null;
    const index = active ? items.indexOf(active) : -1;
    const next =
      event.key === "ArrowDown"
        ? items[(index + 1) % items.length]
        : items[(index - 1 + items.length) % items.length];
    next?.focus();
  }, []);

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: wrapper coordinates trigger and menuitem keyboard navigation.
    <div ref={wrapRef} className={cn("relative inline-flex", className)} onKeyDown={onMenuKey}>
      {triggerEl}
      {open ? (
        // biome-ignore lint/a11y/noStaticElementInteractions: menuitem buttons own interaction; container only closes after click.
        <div
          ref={menuRef}
          role="menu"
          className={cn(
            "absolute top-full z-50 mt-1 flex min-w-44 flex-col gap-px rounded-xl border border-[var(--layer-10)] bg-[var(--layer-04)] p-1 shadow-[var(--shadow-panel)] backdrop-blur",
            align === "right" ? "right-0" : "left-0"
          )}
          onClick={() => setOpen(false)}
        >
          {children}
        </div>
      ) : null}
    </div>
  );
}

type ItemTone = "default" | "destructive";

export function MenuItem({
  icon: Icon,
  children,
  onClick,
  tone = "default",
  disabled = false,
  shortcut,
}: {
  readonly icon?: LucideIcon;
  readonly children: ReactNode;
  readonly onClick?: () => void;
  readonly tone?: ItemTone;
  readonly disabled?: boolean;
  readonly shortcut?: string;
}) {
  return (
    <button
      role="menuitem"
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "flex items-center gap-2 rounded-md border-0 px-2.5 py-1.5 text-left text-xs font-medium leading-none transition-colors",
        "focus-visible:bg-[var(--layer-06)] focus-visible:outline-none",
        "disabled:cursor-not-allowed disabled:opacity-50",
        tone === "destructive"
          ? "text-[var(--destructive-fg)] hover:bg-[var(--destructive-bg)]"
          : "text-[var(--fg-2)] hover:bg-[var(--layer-06)] hover:text-[var(--fg-1)]"
      )}
    >
      {Icon ? <Icon className="size-3.5 opacity-90" /> : null}
      <span className="flex-1">{children}</span>
      {shortcut ? (
        <span className="font-mono text-[10px] text-[var(--fg-5)]">{shortcut}</span>
      ) : null}
    </button>
  );
}

export function MenuSeparator() {
  return <hr className="mx-0.5 my-1 h-px border-0 bg-[var(--layer-08)]" />;
}
