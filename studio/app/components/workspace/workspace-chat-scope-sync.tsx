import { useEffect, useRef, useState } from "react";
import { matchPath, useLocation, useNavigate } from "react-router";
import { selectActiveWorkspaceId, useConversation, useThreads, useWorkspace } from "~/lib/stores";

const CHAT_THREAD_PATTERNS = ["/chat/:threadId", "/playground/:threadId"] as const;

function chatIndexPath(pathname: string): string | null {
  for (const pattern of CHAT_THREAD_PATTERNS) {
    if (matchPath({ path: pattern, end: true }, pathname)) {
      return pattern.replace("/:threadId", "");
    }
  }
  return null;
}

/**
 * Re-scope chat state to the active workspace and clear any focused thread.
 */
export function WorkspaceChatScopeSync() {
  const activeWorkspaceId = useWorkspace(selectActiveWorkspaceId);
  const setThreadResourceId = useThreads((state) => state.setResourceId);
  const loadThread = useConversation((state) => state.setThreadId);
  const setConversationResourceId = useConversation((state) => state.setResourceId);
  const location = useLocation();
  const navigate = useNavigate();
  const [workspaceHydrated, setWorkspaceHydrated] = useState(() =>
    useWorkspace.persist.hasHydrated()
  );
  const previousWorkspaceIdRef = useRef<string | null>(null);
  const pathnameRef = useRef(location.pathname);

  useEffect(() => {
    pathnameRef.current = location.pathname;
  }, [location.pathname]);

  useEffect(() => {
    if (workspaceHydrated) {
      return;
    }

    return useWorkspace.persist.onFinishHydration(() => {
      setWorkspaceHydrated(true);
    });
  }, [workspaceHydrated]);

  useEffect(() => {
    if (!workspaceHydrated) {
      return;
    }

    setThreadResourceId(activeWorkspaceId);
    setConversationResourceId(activeWorkspaceId);

    const previousWorkspaceId = previousWorkspaceIdRef.current;
    previousWorkspaceIdRef.current = activeWorkspaceId;

    if (previousWorkspaceId === null || previousWorkspaceId === activeWorkspaceId) {
      return;
    }

    loadThread(null);

    const indexPath = chatIndexPath(pathnameRef.current);
    if (indexPath) {
      navigate(indexPath, { replace: true });
    }
  }, [
    activeWorkspaceId,
    loadThread,
    navigate,
    setConversationResourceId,
    setThreadResourceId,
    workspaceHydrated,
  ]);

  return null;
}
