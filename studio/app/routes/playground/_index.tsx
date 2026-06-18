import { MessageSquareIcon, PlusIcon } from "lucide-react";
import { useCallback } from "react";
import { useNavigate } from "react-router";
import { EmptyState } from "~/components/shared/empty-state";
import { createLocalThread } from "~/lib/runtime";
import { selectActiveWorkspaceId, useThreads, useWorkspace } from "~/lib/stores";

export default function PlaygroundIndex() {
  const navigate = useNavigate();
  const activeWorkspaceId = useWorkspace(selectActiveWorkspaceId);
  const addThread = useThreads((state) => state.addThread);

  const handleNewChat = useCallback(() => {
    const threadId = `thread_${crypto.randomUUID().replaceAll("-", "")}`;
    const thread = createLocalThread(threadId, activeWorkspaceId);
    addThread(thread);
    navigate(`/playground/${thread.id}`);
  }, [activeWorkspaceId, addThread, navigate]);

  return (
    <EmptyState
      icon={MessageSquareIcon}
      title="Agent Playground"
      description="Start a conversation with your agent to test its capabilities."
      action={{ label: "New Conversation", icon: PlusIcon, onClick: handleNewChat }}
    />
  );
}
