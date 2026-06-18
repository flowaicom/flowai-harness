import { useEffect, useState } from "react";
import { Outlet } from "react-router";
import { ErrorBoundary } from "~/components/error-boundary";
import { ErrorBanner } from "~/components/shared/error-banner";
import { TestSidebar } from "~/components/tests/test-sidebar";
import { getToolCatalog } from "~/lib/api/tests";
import { isOk } from "~/lib/domain/result";
import { useTestSuiteActions } from "~/lib/stores";

/**
 * Tests layout with sidebar and main content area.
 * Loads the eval tool catalog on mount.
 */
export default function TestsLayout() {
  const { setAvailableTools } = useTestSuiteActions();
  const [toolsError, setToolsError] = useState<string | null>(null);

  useEffect(() => {
    getToolCatalog()
      .then((r) => {
        if (isOk(r)) {
          setAvailableTools(r.value);
          setToolsError(null);
        } else {
          setToolsError(r.error.message);
        }
      })
      .catch((e) => {
        setToolsError(e instanceof Error ? e.message : "Failed to load tools");
      });
  }, [setAvailableTools]);

  return (
    <main className="flex h-full min-h-0 bg-background">
      <aside className="w-64 flex-shrink-0 border-r bg-muted/30">
        <TestSidebar />
      </aside>
      <div className="flex-1 flex flex-col min-w-0">
        {toolsError && (
          <div className="px-6 pt-3">
            <ErrorBanner
              message={`Tool catalog: ${toolsError}`}
              onDismiss={() => setToolsError(null)}
            />
          </div>
        )}
        <ErrorBoundary>
          <Outlet />
        </ErrorBoundary>
      </div>
    </main>
  );
}
