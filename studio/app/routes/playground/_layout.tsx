import { Outlet } from "react-router";
import { ThreadSidebar } from "~/components/chat/thread-sidebar";
import { ErrorBoundary } from "~/components/error-boundary";

export default function PlaygroundLayout() {
  return (
    <div className="flex h-full min-h-0 overflow-hidden">
      <aside className="flex min-h-0 w-64 flex-shrink-0 flex-col border-r bg-muted/30">
        <ThreadSidebar />
      </aside>
      <main className="flex min-h-0 flex-1 flex-col min-w-0 overflow-hidden">
        <ErrorBoundary>
          <Outlet />
        </ErrorBoundary>
      </main>
    </div>
  );
}
