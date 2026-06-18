import { SendHorizontalIcon, SquareIcon } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

export interface SharedMessageInputProps {
  readonly onSend: (content: string) => void;
  readonly onCancel: () => void;
  readonly isStreaming: boolean;
  readonly disabled?: boolean;
  readonly placeholder?: string;
  readonly pendingInput?: string;
}

export function SharedMessageInput({
  onSend,
  onCancel,
  isStreaming,
  disabled = false,
  placeholder = "Send a message...",
  pendingInput,
}: SharedMessageInputProps) {
  const [value, setValue] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const resizeTextarea = useCallback(() => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    textarea.style.height = "auto";
    textarea.style.height = `${Math.min(textarea.scrollHeight, 200)}px`;
  }, []);

  const setValueAndResize = useCallback(
    (next: string) => {
      setValue(next);
      requestAnimationFrame(resizeTextarea);
    },
    [resizeTextarea]
  );

  useEffect(() => {
    if (!pendingInput) return;
    setValueAndResize(pendingInput);
    textareaRef.current?.focus();
  }, [pendingInput, setValueAndResize]);

  useEffect(() => {
    textareaRef.current?.focus();
  }, []);

  const handleSubmit = useCallback(() => {
    if (!value.trim() || isStreaming || disabled) return;
    onSend(value.trim());
    setValueAndResize("");
  }, [disabled, isStreaming, onSend, setValueAndResize, value]);

  const handleKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (event.key === "Enter" && !event.shiftKey) {
        event.preventDefault();
        handleSubmit();
      }
    },
    [handleSubmit]
  );

  return (
    <div className="flex items-end gap-2 rounded-lg border bg-background p-2 focus-within:ring-2 focus-within:ring-ring">
      <textarea
        ref={textareaRef}
        value={value}
        onChange={(event) => setValueAndResize(event.target.value)}
        onKeyDown={handleKeyDown}
        placeholder={placeholder}
        disabled={disabled || isStreaming}
        rows={1}
        className="min-h-[40px] max-h-[200px] flex-1 resize-none border-0 bg-transparent px-2 py-1.5 text-sm outline-none placeholder:text-muted-foreground"
      />

      {isStreaming ? (
        <button
          type="button"
          onClick={onCancel}
          aria-label="Stop streaming"
          className="rounded-md bg-destructive p-2 text-destructive-foreground transition-colors hover:bg-destructive/90"
        >
          <SquareIcon className="size-4" />
        </button>
      ) : (
        <button
          type="button"
          onClick={handleSubmit}
          disabled={!value.trim() || disabled}
          aria-label="Send message"
          className={cx(
            "rounded-md bg-primary p-2 text-primary-foreground transition-colors hover:bg-primary/90",
            "disabled:cursor-not-allowed disabled:opacity-50"
          )}
        >
          <SendHorizontalIcon className="size-4" />
        </button>
      )}
    </div>
  );
}
