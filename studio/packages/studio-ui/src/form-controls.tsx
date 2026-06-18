import type {
  InputHTMLAttributes,
  ReactNode,
  SelectHTMLAttributes,
  TextareaHTMLAttributes,
} from "react";
import { cn } from "./utils/cn";

type ControlSize = "sm" | "md";

const sizeClass: Record<ControlSize, string> = {
  sm: "h-7 text-[11px]",
  md: "h-8 text-xs",
};

export function Input({
  className,
  invalid,
  size = "md",
  ...props
}: Omit<InputHTMLAttributes<HTMLInputElement>, "size"> & {
  readonly invalid?: boolean;
  readonly size?: ControlSize;
}) {
  return (
    <input
      {...props}
      aria-invalid={invalid || undefined}
      className={cn("studio-input", sizeClass[size], className)}
    />
  );
}

export function Textarea({
  className,
  invalid,
  ...props
}: TextareaHTMLAttributes<HTMLTextAreaElement> & { readonly invalid?: boolean }) {
  return (
    <textarea
      {...props}
      aria-invalid={invalid || undefined}
      className={cn("studio-input", className)}
    />
  );
}

export function Select({
  className,
  invalid,
  size = "md",
  ...props
}: Omit<SelectHTMLAttributes<HTMLSelectElement>, "size"> & {
  readonly invalid?: boolean;
  readonly size?: ControlSize;
}) {
  return (
    <select
      {...props}
      aria-invalid={invalid || undefined}
      className={cn("studio-input", sizeClass[size], className)}
    />
  );
}

export function FieldLabel({
  htmlFor,
  children,
  hint,
  className,
}: {
  readonly htmlFor?: string;
  readonly children: ReactNode;
  readonly hint?: ReactNode;
  readonly className?: string;
}) {
  return (
    <label
      htmlFor={htmlFor}
      className={cn("studio-eyebrow mb-1.5 flex items-baseline gap-2", className)}
    >
      <span>{children}</span>
      {hint ? <span className="normal-case tracking-normal text-[var(--fg-5)]">{hint}</span> : null}
    </label>
  );
}
