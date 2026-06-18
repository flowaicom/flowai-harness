import type { LucideIcon } from "lucide-react";
import { XIcon } from "lucide-react";
import type { ReactNode } from "react";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

const CATEGORY_COLORS: Record<
  string,
  { readonly bg: string; readonly text: string; readonly border: string }
> = {
  discovery: {
    bg: "bg-[var(--accent-blue)]",
    text: "text-[var(--dot-blue)]",
    border: "border-[var(--dot-blue)]/25",
  },
  planning: {
    bg: "bg-[var(--accent-purple)]",
    text: "text-[var(--dot-purple)]",
    border: "border-[var(--dot-purple)]/25",
  },
  execution: {
    bg: "bg-[var(--accent-amber)]",
    text: "text-[var(--dot-amber)]",
    border: "border-[var(--dot-amber)]/25",
  },
  knowledge: {
    bg: "bg-[var(--accent-emerald)]",
    text: "text-[var(--dot-emerald)]",
    border: "border-[var(--dot-emerald)]/25",
  },
};

const FALLBACK_COLOR = {
  bg: "bg-primary/8",
  text: "text-primary",
  border: "border-primary/20",
};

function badgeColor(category: string | undefined) {
  return (category && CATEGORY_COLORS[category]) || FALLBACK_COLOR;
}

export function ConnectSectionCard({
  children,
  className,
}: {
  readonly children: ReactNode;
  readonly className?: string;
}) {
  return (
    <div className={cx("rounded-lg border border-border/50 p-4 space-y-3", className)}>
      {children}
    </div>
  );
}

export function ConnectSectionHeader({
  children,
  className,
}: {
  readonly children: ReactNode;
  readonly className?: string;
}) {
  return <h3 className={cx("section-label", className)}>{children}</h3>;
}

export function ConnectCategoryBadge({
  label,
  category,
  className,
}: {
  readonly label: string;
  readonly category: string;
  readonly className?: string;
}) {
  const color = badgeColor(category);

  return (
    <span
      className={cx(
        "px-1.5 py-0.5 rounded text-xs font-medium border",
        color.bg,
        color.text,
        color.border,
        className
      )}
    >
      {label}
    </span>
  );
}

export function ConnectPillTabs<K extends string>({
  tabs,
  active,
  onChange,
  className,
}: {
  readonly tabs: readonly { readonly id: K; readonly label: string; readonly count?: number }[];
  readonly active: K;
  readonly onChange: (id: K) => void;
  readonly className?: string;
}) {
  return (
    <div className={cx("flex items-center gap-1", className)} role="tablist">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          role="tab"
          aria-label={tab.label}
          aria-selected={active === tab.id}
          onClick={() => onChange(tab.id)}
          className={cx(
            "px-3 py-1 rounded-md text-xs font-medium transition-colors whitespace-nowrap",
            active === tab.id
              ? "bg-foreground/8 text-foreground"
              : "text-muted-foreground hover:bg-muted/60 hover:text-foreground"
          )}
        >
          {tab.label}
          {tab.count != null ? (
            <span
              aria-hidden="true"
              className={cx("ml-1.5 tabular-nums", active === tab.id ? "opacity-80" : "opacity-60")}
            >
              {tab.count}
            </span>
          ) : null}
        </button>
      ))}
    </div>
  );
}

export function ConnectEmptyState({
  icon: Icon,
  title,
  description,
  action,
  className,
}: {
  readonly icon: LucideIcon;
  readonly title: string;
  readonly description: string;
  readonly action?: { readonly label: string; readonly onClick: () => void };
  readonly className?: string;
}) {
  return (
    <div className={cx("flex-1 flex items-center justify-center text-muted-foreground", className)}>
      <div className="text-center max-w-sm">
        <div className="w-12 h-12 mx-auto mb-4 rounded-lg bg-muted/50 flex items-center justify-center">
          <Icon className="w-6 h-6 text-muted-foreground/30" />
        </div>
        <p className="text-sm font-medium text-foreground mb-1">{title}</p>
        <p className="text-xs text-muted-foreground mb-4">{description}</p>
        {action ? (
          <button
            type="button"
            onClick={action.onClick}
            className="inline-flex items-center gap-2 px-4 py-1.5 bg-primary text-primary-foreground rounded-lg hover:bg-primary/90 transition-colors text-sm font-medium"
          >
            {action.label}
          </button>
        ) : null}
      </div>
    </div>
  );
}

export function ConnectErrorBanner({
  message,
  onDismiss,
  onRetry,
  className,
}: {
  readonly message: string;
  readonly onDismiss: () => void;
  readonly onRetry?: () => void;
  readonly className?: string;
}) {
  return (
    <div
      className={cx(
        "flex items-center justify-between rounded-lg accent-bar-red bg-[var(--accent-red)] px-4 py-2.5 text-[var(--dot-red)] text-sm",
        className
      )}
    >
      <span>{message}</span>
      <div className="ml-3 shrink-0 flex items-center gap-1">
        {onRetry ? (
          <button
            type="button"
            onClick={onRetry}
            className="px-2 py-0.5 rounded border border-[var(--dot-red)]/20 text-xs text-[var(--dot-red)]/80 hover:text-[var(--dot-red)] transition-colors"
          >
            Retry
          </button>
        ) : null}
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
