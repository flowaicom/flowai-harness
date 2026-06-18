import { buildDisplayPartKeys, type DisplayPart, groupParts, type MessagePart } from "@studio/core";
import { Loader2Icon } from "lucide-react";
import { memo, type ReactNode, useMemo } from "react";

export interface SharedStreamingMessageProps {
  readonly parts: readonly MessagePart[];
  readonly isStreaming?: boolean;
  readonly loadingLabel?: string;
  readonly renderPart: (part: DisplayPart, key: string) => ReactNode;
}

const areStreamingPartsEqual = (
  prev: SharedStreamingMessageProps,
  next: SharedStreamingMessageProps
): boolean => {
  if (prev.isStreaming !== next.isStreaming) return false;
  if (prev.loadingLabel !== next.loadingLabel) return false;
  if (prev.parts.length !== next.parts.length) return false;
  if (prev.parts.length === 0) return true;

  for (let index = 0; index < prev.parts.length; index += 1) {
    const prevPart = prev.parts[index];
    const nextPart = next.parts[index];
    if (prevPart.type !== nextPart.type) return false;
    if (prevPart.type === "tool-invocation" && nextPart.type === "tool-invocation") {
      if (prevPart.state !== nextPart.state || prevPart.toolCallId !== nextPart.toolCallId) {
        return false;
      }
      if ((prevPart.progress?.phases.length ?? 0) !== (nextPart.progress?.phases.length ?? 0)) {
        return false;
      }
    }
  }

  const prevLast = prev.parts[prev.parts.length - 1];
  const nextLast = next.parts[next.parts.length - 1];

  if (prevLast.type === "text" && nextLast.type === "text") {
    return prevLast.text === nextLast.text;
  }
  if (prevLast.type === "reasoning" && nextLast.type === "reasoning") {
    return prevLast.text.length === nextLast.text.length;
  }

  return prevLast === nextLast;
};

export const SharedStreamingMessage = memo(function SharedStreamingMessage({
  parts,
  isStreaming = true,
  loadingLabel = "Thinking...",
  renderPart,
}: SharedStreamingMessageProps) {
  const groupedParts = useMemo(() => groupParts(parts), [parts]);
  const partKeys = useMemo(() => buildDisplayPartKeys(groupedParts), [groupedParts]);
  const keyedParts = useMemo(
    () => groupedParts.map((part, index) => ({ key: partKeys[index], part })),
    [groupedParts, partKeys]
  );

  return (
    <div className="mx-auto mt-4 max-w-3xl">
      {groupedParts.length > 0 ? (
        <div className="space-y-2">
          {keyedParts.map(({ key, part }) => (
            <div key={key}>{renderPart(part, key)}</div>
          ))}
        </div>
      ) : null}
      {isStreaming ? (
        <div className="mt-2 flex items-center gap-2 text-muted-foreground">
          <Loader2Icon className="size-4 animate-spin" />
          <span className="text-sm">{loadingLabel}</span>
        </div>
      ) : null}
    </div>
  );
}, areStreamingPartsEqual);
