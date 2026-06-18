import { useCallback, useMemo } from "react";
import { cn } from "./utils/cn";

export function JsonEditor({
  value,
  onChange,
  ariaLabel,
  placeholder,
  invalid,
  readOnly,
  rows = 8,
  className,
}: {
  readonly value: string;
  readonly onChange?: (next: string) => void;
  readonly ariaLabel: string;
  readonly placeholder?: string;
  readonly invalid?: boolean;
  readonly readOnly?: boolean;
  readonly rows?: number;
  readonly className?: string;
}) {
  const handleChange = useCallback(
    (event: React.ChangeEvent<HTMLTextAreaElement>) => {
      onChange?.(event.target.value);
    },
    [onChange]
  );

  const parseError = useMemo(() => {
    if (!value.trim()) return null;
    try {
      JSON.parse(value);
      return null;
    } catch (err) {
      return err instanceof Error ? err.message : String(err);
    }
  }, [value]);

  const handleFormat = useCallback(() => {
    if (!onChange) return;
    try {
      const next = JSON.stringify(JSON.parse(value), null, 2);
      onChange(next);
    } catch {
      // The parse error is already surfaced next to the editor.
    }
  }, [onChange, value]);

  return (
    <div className={cn("flex min-w-0 flex-col gap-1.5", className)}>
      <textarea
        aria-label={ariaLabel}
        value={value}
        onChange={handleChange}
        placeholder={placeholder}
        readOnly={readOnly}
        rows={rows}
        spellCheck={false}
        aria-invalid={invalid || Boolean(parseError) || undefined}
        className={cn(
          "studio-input font-mono text-[12px] leading-[18px]",
          parseError && "border-[var(--destructive-border)]"
        )}
      />
      <div className="flex items-center justify-between gap-2 text-[10px] text-[var(--fg-5)]">
        <span className={parseError ? "text-[var(--destructive-fg)]" : undefined}>
          {parseError ? parseError : "JSON"}
        </span>
        {onChange ? (
          <button
            type="button"
            onClick={handleFormat}
            disabled={Boolean(parseError) || readOnly}
            className="rounded border border-[var(--layer-08)] bg-[var(--layer-04)] px-1.5 py-0.5 text-[10px] font-medium text-[var(--fg-3)] transition-colors hover:text-[var(--fg-1)] disabled:opacity-50"
          >
            Format
          </button>
        ) : null}
      </div>
    </div>
  );
}
