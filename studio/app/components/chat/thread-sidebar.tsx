import { SharedThreadSidebarView } from "@studio/features-chat";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router";
import { ConfirmDialog } from "~/components/shared/confirm-dialog";
import { isOk } from "~/lib/domain/result";
import { isEvalThread } from "~/lib/domain/thread";
import { createLocalThread, threadSummaryToThread, useHarnessRuntime } from "~/lib/runtime";
import { useScramble } from "~/lib/scramble";
import {
  selectActiveWorkspaceId,
  selectCancelStreaming,
  selectSelectedThreadId,
  selectStreamSessions,
  selectThreads,
  selectThreadsLoading,
  useConversation,
  useThreadActions,
  useThreads,
  useWorkspace,
} from "~/lib/stores";

export function ThreadSidebar() {
  const navigate = useNavigate();
  const threads = useThreads(selectThreads);
  const activeThreadId = useThreads(selectSelectedThreadId);
  const isLoading = useThreads(selectThreadsLoading);
  const activeWorkspaceId = useWorkspace(selectActiveWorkspaceId);
  const streamSessions = useConversation(selectStreamSessions);
  const abortStream = useConversation(selectCancelStreaming);
  const { adapter, scope } = useHarnessRuntime();
  const { s } = useScramble();
  const { setThreads, addThread, removeThread, setLoadPhase } = useThreadActions();
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);

  useEffect(() => {
    const loadThreads = async () => {
      setLoadPhase({ phase: "loading" });
      const result = await adapter.listThreads(scope);
      if (isOk(result)) {
        setThreads(result.value.map((thread) => threadSummaryToThread(thread, activeWorkspaceId)));
      } else {
        setLoadPhase({ phase: "failed", reason: result.error.message });
      }
    };

    void loadThreads();
  }, [setThreads, setLoadPhase, activeWorkspaceId, adapter, scope]);

  const handleNewThread = useCallback(() => {
    const threadId = `thread_${crypto.randomUUID().replaceAll("-", "")}`;
    const thread = createLocalThread(threadId, activeWorkspaceId);
    addThread(thread);
    navigate(`/chat/${thread.id}`);
  }, [activeWorkspaceId, addThread, navigate]);

  const handleDeleteThread = useCallback((threadId: string) => {
    setDeleteError(null);
    setPendingDeleteId(threadId);
  }, []);

  const confirmDeleteThread = useCallback(async () => {
    if (!pendingDeleteId || deleteBusy) {
      return;
    }

    const threadId = pendingDeleteId;
    setDeleteBusy(true);
    setDeleteError(null);

    const result = await adapter.deleteThread(scope, threadId);
    if (isOk(result)) {
      abortStream(threadId);
      removeThread(threadId);
      setPendingDeleteId(null);
      if (activeThreadId === threadId) {
        navigate("/playground");
      }
    } else {
      setDeleteError(result.error.message);
    }

    setDeleteBusy(false);
  }, [
    abortStream,
    activeThreadId,
    adapter,
    deleteBusy,
    navigate,
    pendingDeleteId,
    removeThread,
    scope,
  ]);

  const chatThreads = useMemo(
    () => threads.filter((thread) => !isEvalThread(thread.id)),
    [threads]
  );
  const items = useMemo(
    () =>
      chatThreads.map((thread) => ({
        id: thread.id,
        title: thread.title,
        updatedAt: thread.updatedAt,
        href: `/chat/${thread.id}`,
        isSelected: thread.id === activeThreadId,
        isStreaming: streamSessions.has(thread.id),
      })),
    [activeThreadId, chatThreads, streamSessions]
  );

  return (
    <>
      <SharedThreadSidebarView
        title="Chats"
        items={items}
        isLoading={isLoading}
        transientError={deleteError}
        onCreate={handleNewThread}
        onDelete={handleDeleteThread}
        createAriaLabel="New chat"
        emptyStateLabel="No conversations yet"
        emptySearchLabel="No matches"
        emptyActionLabel="Start a new chat"
        searchPlaceholder="Search chats..."
        formatTitle={s}
      />
      <ConfirmDialog
        open={pendingDeleteId !== null}
        title="Delete chat"
        description="Delete this chat thread and its local messages from the Studio store? This cannot be undone."
        confirmLabel={deleteBusy ? "Deleting..." : "Delete"}
        tone="danger"
        onConfirm={confirmDeleteThread}
        onCancel={() => {
          if (!deleteBusy) {
            setPendingDeleteId(null);
          }
        }}
      />
    </>
  );
}
