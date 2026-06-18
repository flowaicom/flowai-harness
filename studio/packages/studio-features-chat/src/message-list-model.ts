export interface TextPartLike {
  readonly type: string;
  readonly text?: string;
}

export interface ChatMessageLike<
  TPart extends TextPartLike = TextPartLike,
  TRole extends string = string,
> {
  readonly id: string;
  readonly role: TRole;
  readonly parts: readonly TPart[];
}

export interface ChatHistoryEntry<TRole extends string = string> {
  readonly role: TRole;
  readonly content: string;
}

export function getMessageText(parts: readonly TextPartLike[]): string {
  return parts
    .filter(
      (part): part is TextPartLike & { readonly text: string } =>
        part.type === "text" && typeof part.text === "string"
    )
    .map((part) => part.text)
    .join("");
}

export function serializeChatHistory<TMessage extends ChatMessageLike>(
  messages: readonly TMessage[]
): ChatHistoryEntry<TMessage["role"]>[] {
  return messages.map((message) => ({
    role: message.role,
    content: getMessageText(message.parts),
  }));
}

export function getStreamingMessageId<TMessage extends { readonly id: string }>(
  messages: readonly TMessage[],
  isStreaming: boolean
): string | undefined {
  if (!isStreaming || messages.length === 0) {
    return undefined;
  }

  return messages[messages.length - 1]?.id;
}
