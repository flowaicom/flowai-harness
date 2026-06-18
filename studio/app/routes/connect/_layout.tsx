import { Outlet } from "react-router";
import { ConnectSidebar } from "~/components/connect/connect-sidebar";
import { ErrorBoundary } from "~/components/error-boundary";

/**
 * Connect layout with sidebar and main content area.
 */
export default function ConnectLayout() {
  return (
    <main className="flex h-full min-h-0 bg-background">
      {/* Connect sidebar */}
      <aside className="w-64 flex-shrink-0 border-r bg-muted/30">
        <ConnectSidebar />
      </aside>

      {/* Main content area — error boundary prevents route crash from destroying sidebar */}
      <div className="flex-1 flex flex-col min-w-0">
        <ErrorBoundary>
          <Outlet />
        </ErrorBoundary>
      </div>
    </main>
  );
}
