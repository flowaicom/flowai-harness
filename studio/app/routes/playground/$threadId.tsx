import { useEffect } from "react";
import { useParams } from "react-router";
import { ChatArea } from "~/components/chat/chat-area";
import { ChatErrorBoundary } from "~/components/error-boundary";
import { useConversation, useThreads } from "~/lib/stores";

export default function PlaygroundThread() {
  const { threadId } = useParams();
  const loadThread = useConversation((state) => state.setThreadId);
  const selectThread = useThreads((state) => state.selectThread);

  useEffect(() => {
    if (!threadId) return;

    selectThread(threadId);
    loadThread(threadId);

    return () => {
      selectThread(null);
    };
  }, [loadThread, selectThread, threadId]);

  if (!threadId) {
    return (
      <div className="flex flex-1 items-center justify-center text-muted-foreground">
        No thread selected
      </div>
    );
  }

  return (
    <ChatErrorBoundary threadId={threadId}>
      <ChatArea threadId={threadId} />
    </ChatErrorBoundary>
  );
}
