import { NavLink, Outlet } from "react-router";
import { ErrorBoundary } from "~/components/error-boundary";
import { cn } from "~/lib/utils";

const TABS = [
  { to: "/runs", label: "Runs", end: true },
  { to: "/runs/approvals", label: "Approvals", end: false },
] as const;

/**
 * Runs / activity section: run inspection (M8.3) + approval inbox (M8.4).
 */
export default function RunsLayout() {
  return (
    <main className="flex h-full min-h-0 flex-col bg-background">
      <header className="flex items-center gap-1 border-b px-6 py-3">
        <h1 className="mr-4 text-sm font-semibold">Runs &amp; Activity</h1>
        {TABS.map((tab) => (
          <NavLink
            key={tab.to}
            to={tab.to}
            end={tab.end}
            className={({ isActive }) =>
              cn(
                "rounded-md px-3 py-1.5 text-sm transition-colors",
                isActive ? "bg-muted font-medium" : "text-muted-foreground hover:bg-muted/50"
              )
            }
          >
            {tab.label}
          </NavLink>
        ))}
      </header>
      <div className="min-h-0 flex-1 overflow-auto">
        <ErrorBoundary>
          <Outlet />
        </ErrorBoundary>
      </div>
    </main>
  );
}
