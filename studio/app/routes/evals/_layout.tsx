import { Outlet } from "react-router";
import { ErrorBoundary } from "~/components/error-boundary";
import { EvalSidebar } from "~/components/eval/eval-sidebar";

/**
 * Evals layout with sidebar and main content area.
 *
 * Layout:
 * ┌─────────────┬─────────────────────────────────┐
 * │   Sidebar   │         Main Content            │
 * │   (Evals)   │         (Detail/New)            │
 * │             │                                 │
 * │             │                                 │
 * └─────────────┴─────────────────────────────────┘
 */
export default function EvalsLayout() {
  return (
    <main className="flex h-full min-h-0 bg-background">
      {/* Eval sidebar */}
      <aside className="w-64 flex-shrink-0 border-r bg-muted/30">
        <EvalSidebar />
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
