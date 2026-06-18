/**
 * Inline copy-to-clipboard button with success feedback.
 *
 * Shows a copy icon; on click, copies text and shows a check icon for 1.5s.
 * Designed for compact inline use next to IDs, code, and labels.
 *
 * @module components/shared/copy-button
 */

import { CheckIcon, CopyIcon } from "lucide-react";
import { memo, useCallback, useRef, useState } from "react";
import { cn } from "~/lib/utils";

interface CopyButtonProps {
  readonly text: string;
  readonly className?: string;
  readonly label?: string;
}

export const CopyButton = memo(function CopyButton({ text, className, label }: CopyButtonProps) {
  const [copied, setCopied] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => setCopied(false), 1500);
    });
  }, [text]);

  return (
    <button
      type="button"
      onClick={handleCopy}
      className={cn(
        "inline-flex items-center justify-center p-0.5 rounded hover:bg-muted transition-colors",
        copied
          ? "text-[var(--dot-emerald)]"
          : "text-muted-foreground/50 hover:text-muted-foreground",
        className
      )}
      title={label ?? "Copy to clipboard"}
      aria-label={label ?? "Copy to clipboard"}
    >
      {copied ? <CheckIcon className="size-3" /> : <CopyIcon className="size-3" />}
    </button>
  );
});
