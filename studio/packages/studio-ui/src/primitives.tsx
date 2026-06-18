import type { LucideIcon } from "lucide-react";
import type { ButtonHTMLAttributes, ComponentPropsWithoutRef, ElementType, ReactNode } from "react";
import { cn } from "./utils/cn";

export type StudioTone = "neutral" | "green" | "blue" | "violet" | "amber" | "orange" | "red";

const toneClass: Record<StudioTone, string> = {
  neutral: "border-[var(--layer-08)] bg-[var(--layer-04)] text-[var(--fg-3)]",
  green:
    "border-[var(--accent-green-border)] bg-[var(--accent-green-bg)] text-[var(--accent-green-fg)]",
  blue: "border-[var(--accent-blue-border)] bg-[var(--accent-blue-bg)] text-[var(--accent-blue-fg)]",
  violet:
    "border-[var(--accent-violet-border)] bg-[var(--accent-violet-bg)] text-[var(--accent-violet-fg)]",
  amber:
    "border-[var(--accent-amber-border)] bg-[var(--accent-amber-bg)] text-[var(--accent-amber-fg)]",
  orange:
    "border-[var(--accent-orange-border)] bg-[var(--accent-orange-bg)] text-[var(--accent-orange-fg)]",
  red: "border-[var(--destructive-border)] bg-[var(--destructive-bg)] text-[var(--destructive-fg)]",
};

const dotClass: Record<StudioTone, string> = {
  neutral: "bg-[var(--fg-5)]",
  green: "bg-[var(--dot-emerald)]",
  blue: "bg-[var(--dot-blue)]",
  violet: "bg-[var(--dot-purple)]",
  amber: "bg-[var(--dot-amber)]",
  orange: "bg-[var(--dot-orange)]",
  red: "bg-[var(--dot-red)]",
};

export type StudioButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type StudioButtonSize = "sm" | "md";

const buttonVariantClass: Record<StudioButtonVariant, string> = {
  primary: "border-[var(--fg-1)] bg-[var(--fg-1)] text-[var(--chrome-base)] hover:bg-[var(--fg-2)]",
  secondary:
    "border-[var(--layer-08)] bg-[var(--layer-04)] text-[var(--fg-2)] hover:bg-[var(--layer-06)]",
  ghost:
    "border-transparent text-[var(--fg-4)] hover:bg-[var(--layer-04)] hover:text-[var(--fg-1)]",
  danger:
    "border-[var(--destructive-border)] bg-[var(--destructive-bg)] text-[var(--destructive-fg)] hover:border-[var(--destructive-fg)]/40",
};

const buttonSizeClass: Record<StudioButtonSize, string> = {
  sm: "h-8 px-3 text-xs",
  md: "h-9 px-3.5 text-sm",
};

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  readonly variant?: StudioButtonVariant;
  readonly size?: StudioButtonSize;
}

export function Button({
  variant = "secondary",
  size = "md",
  className,
  type = "button",
  ...props
}: ButtonProps) {
  return (
    <button
      {...props}
      type={type}
      className={cn(
        "inline-flex min-w-0 items-center justify-center gap-2 rounded-lg border font-medium transition-colors disabled:pointer-events-none disabled:opacity-50",
        buttonVariantClass[variant],
        buttonSizeClass[size],
        className
      )}
    />
  );
}

export interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  readonly label: string;
  readonly size?: StudioButtonSize;
}

export function IconButton({ label, size = "md", className, children, ...props }: IconButtonProps) {
  return (
    <Button
      {...props}
      size={size}
      variant="ghost"
      aria-label={label}
      title={label}
      className={cn(size === "sm" ? "size-8 px-0" : "size-9 px-0", className)}
    >
      {children}
    </Button>
  );
}

export function Badge({
  tone = "neutral",
  className,
  children,
}: {
  readonly tone?: StudioTone;
  readonly className?: string;
  readonly children: ReactNode;
}) {
  return (
    <span
      className={cn(
        "inline-flex min-w-0 items-center rounded-md border px-2 py-0.5 text-[11px] font-medium leading-4",
        toneClass[tone],
        className
      )}
    >
      {children}
    </span>
  );
}

export function StatusDot({
  tone = "neutral",
  label,
  pulse = false,
  className,
}: {
  readonly tone?: StudioTone;
  readonly label?: string;
  readonly pulse?: boolean;
  readonly className?: string;
}) {
  const classes = cn(
    "inline-block size-2 rounded-full",
    dotClass[tone],
    pulse && "animate-pulse",
    className
  );

  if (label) {
    return <span role="img" aria-label={label} className={classes} />;
  }

  return <span aria-hidden="true" className={classes} />;
}

export interface SegmentedOption<K extends string> {
  readonly value: K;
  readonly label: string;
}

export type SegmentedSize = "sm" | "md";

const segmentedSizeClass: Record<SegmentedSize, string> = {
  sm: "h-6 rounded-[5px] px-2 text-[11px]",
  md: "h-7 rounded-md px-2.5 text-xs",
};

export function SegmentedControl<K extends string>({
  label,
  value,
  options,
  onChange,
  size = "md",
  className,
}: {
  readonly label: string;
  readonly value: K;
  readonly options: readonly SegmentedOption<K>[];
  readonly onChange: (value: K) => void;
  readonly size?: SegmentedSize;
  readonly className?: string;
}) {
  return (
    <fieldset
      className={cn(
        "inline-grid grid-flow-col gap-0.5 rounded-lg border border-[var(--layer-08)] bg-[var(--layer-04)] p-0.5",
        className
      )}
    >
      <legend className="sr-only">{label}</legend>
      {options.map((option) => {
        const active = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            aria-pressed={active}
            onClick={() => onChange(option.value)}
            className={cn(
              "font-medium transition-colors",
              segmentedSizeClass[size],
              active
                ? "bg-[var(--chrome-card)] text-[var(--fg-1)] shadow-sm"
                : "text-[var(--fg-5)] hover:text-[var(--fg-1)]"
            )}
          >
            {option.label}
          </button>
        );
      })}
    </fieldset>
  );
}

type DeltaTone = "up" | "down" | "neutral";

const deltaToneClass: Record<DeltaTone, string> = {
  up: "text-[var(--accent-green-fg)]",
  down: "text-[var(--destructive-fg)]",
  neutral: "text-[var(--fg-5)]",
};

export function StatCard({
  label,
  value,
  meta,
  delta,
  deltaTone = "neutral",
  hint,
  tone,
  className,
}: {
  readonly label: string;
  readonly value: ReactNode;
  readonly meta?: ReactNode;
  readonly delta?: ReactNode;
  readonly deltaTone?: DeltaTone;
  readonly hint?: ReactNode;
  readonly tone?: StudioTone;
  readonly className?: string;
}) {
  return (
    <PagePanel className={cn("p-4", className)}>
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="text-[11px] font-medium tracking-wide text-[var(--fg-5)]">{label}</div>
          <div className="mt-1.5 flex items-baseline gap-2">
            <div className="text-[22px] font-medium leading-7 tracking-tight tabular-nums text-[var(--fg-1)]">
              {value}
            </div>
            {delta ? (
              <div className={cn("font-mono text-[11px]", deltaToneClass[deltaTone])}>{delta}</div>
            ) : null}
          </div>
          {hint ? <div className="mt-1 text-[11px] text-[var(--fg-5)]">{hint}</div> : null}
          {meta ? <div className="mt-1 text-xs text-[var(--fg-5)]">{meta}</div> : null}
        </div>
        {tone ? <span className={cn("mt-0.5 size-2 rounded-full", dotClass[tone])} /> : null}
      </div>
    </PagePanel>
  );
}

export function JsonPane({
  value,
  ariaLabel,
  className,
}: {
  readonly value: unknown;
  readonly ariaLabel: string;
  readonly className?: string;
}) {
  const text = typeof value === "string" ? value : JSON.stringify(value, null, 2);
  return (
    <section aria-label={ariaLabel}>
      <pre
        className={cn(
          "max-h-80 overflow-auto rounded-lg border border-[var(--layer-08)] bg-[var(--chrome-raised)] p-3 font-mono text-xs leading-5 text-[var(--fg-3)]",
          className
        )}
      >
        {text}
      </pre>
    </section>
  );
}

export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
  className,
}: {
  readonly icon: LucideIcon;
  readonly title: string;
  readonly description: string;
  readonly action?: ReactNode;
  readonly className?: string;
}) {
  return (
    <div className={cn("flex min-h-56 items-center justify-center text-center", className)}>
      <div className="max-w-sm">
        <div className="mx-auto mb-4 flex size-11 items-center justify-center rounded-xl border border-[var(--layer-08)] bg-[var(--layer-04)] text-[var(--fg-4)]">
          <Icon className="size-5" />
        </div>
        <div className="text-sm font-medium text-[var(--fg-1)]">{title}</div>
        <div className="mt-1 text-xs leading-5 text-[var(--fg-5)]">{description}</div>
        {action ? <div className="mt-4">{action}</div> : null}
      </div>
    </div>
  );
}

export function Sparkline({
  values,
  width = 96,
  height = 22,
  color = "var(--accent-blue-fg)",
  fill = true,
  className,
}: {
  readonly values: readonly number[];
  readonly width?: number;
  readonly height?: number;
  readonly color?: string;
  readonly fill?: boolean;
  readonly className?: string;
}) {
  if (!values || values.length < 2) return null;
  const max = Math.max(...values);
  const min = Math.min(...values);
  const range = max - min || 1;
  const step = width / (values.length - 1);
  const points = values.map((value, index) => {
    const x = index * step;
    const y = height - ((value - min) / range) * (height - 2) - 1;
    return `${x.toFixed(1)} ${y.toFixed(1)}`;
  });
  const path = points.map((point, index) => (index === 0 ? "M" : "L") + point).join(" ");
  const area = `${path} L ${width} ${height} L 0 ${height} Z`;
  const gradientId = `sp-${values.join("-").slice(0, 20)}-${width}`;
  return (
    <svg
      width={width}
      height={height}
      className={cn("block", className)}
      aria-hidden="true"
      focusable="false"
    >
      {fill ? (
        <>
          <defs>
            <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0" stopColor={color} stopOpacity="0.22" />
              <stop offset="1" stopColor={color} stopOpacity="0" />
            </linearGradient>
          </defs>
          <path d={area} fill={`url(#${gradientId})`} />
        </>
      ) : null}
      <path d={path} stroke={color} strokeWidth={1.3} fill="none" />
    </svg>
  );
}

export function FlowMark({
  size = 22,
  className,
}: {
  readonly size?: number;
  readonly className?: string;
}) {
  const height = Math.round((size * 109) / 140);
  return (
    <svg
      width={size}
      height={height}
      viewBox="0 0 140 109"
      className={cn("block shrink-0", className)}
      aria-hidden="true"
      focusable="false"
    >
      <defs>
        <linearGradient id="flow-mark-bg" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#2a2e33" />
          <stop offset="1" stopColor="#0c0d0f" />
        </linearGradient>
        <linearGradient id="flow-mark-mark" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#d7dbe0" />
          <stop offset="1" stopColor="#8a8f96" />
        </linearGradient>
      </defs>
      <rect width="140" height="109" rx="18" fill="url(#flow-mark-bg)" />
      <path d="M27 24 L87 24 L67 49 L27 49 Z" fill="url(#flow-mark-mark)" />
      <path d="M27 60 L51 60 L32 85 L27 85 Z" fill="url(#flow-mark-mark)" />
      <path d="M103 32 L109 32 L109 85 L56 85 Z" fill="url(#flow-mark-mark)" />
    </svg>
  );
}

export function FormSection({
  label,
  hint,
  step,
  compact,
  children,
  className,
}: {
  readonly label: ReactNode;
  readonly hint?: ReactNode;
  readonly step?: number;
  readonly compact?: boolean;
  readonly children: ReactNode;
  readonly className?: string;
}) {
  return (
    <div className={className}>
      <div className={cn("flex items-baseline gap-2", compact ? "mb-1.5" : "mb-2")}>
        {step ? (
          <span className="inline-flex size-4 items-center justify-center rounded border border-[var(--layer-08)] bg-[var(--layer-06)] font-mono text-[10px] font-medium text-[var(--fg-3)]">
            {step}
          </span>
        ) : null}
        <label className="text-xs font-medium text-[var(--fg-2)]">{label}</label>
        {hint ? <span className="text-[11px] text-[var(--fg-5)]">- {hint}</span> : null}
      </div>
      {children}
    </div>
  );
}

export function Kbd({
  children,
  className,
}: {
  readonly children: ReactNode;
  readonly className?: string;
}) {
  return (
    <kbd
      className={cn(
        "inline-flex h-[18px] min-w-[18px] items-center justify-center rounded border border-[var(--layer-08)] bg-[var(--layer-05)] px-1 font-mono text-[10px] leading-none text-[var(--fg-4)]",
        className
      )}
    >
      {children}
    </kbd>
  );
}

type PagePanelProps<T extends ElementType> = {
  readonly as?: T;
  readonly interactive?: boolean;
  readonly className?: string;
  readonly children: ReactNode;
} & Omit<ComponentPropsWithoutRef<T>, "as" | "className" | "children">;

export function PagePanel<T extends ElementType = "div">({
  as,
  interactive = false,
  className,
  children,
  ...props
}: PagePanelProps<T>) {
  const Component = as ?? "div";
  return (
    <Component
      {...props}
      className={cn(
        "studio-panel",
        interactive &&
          "transition-colors hover:border-[var(--layer-12)] hover:bg-[var(--layer-03)]",
        className
      )}
    >
      {children}
    </Component>
  );
}
