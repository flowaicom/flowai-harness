import { SharedMessageInput, type SharedMessageInputProps } from "@studio/features-chat";

type MessageInputProps = SharedMessageInputProps;

export function MessageInput(props: MessageInputProps) {
  return <SharedMessageInput {...props} />;
}
