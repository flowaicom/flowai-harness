/**
 * Top-level navigation bar with tab links.
 *
 * Playground, Tests, Evals tabs. Active tab gets primary border.
 * Tests and Evals tabs show live badge counts from their respective stores.
 * Multi-session aware — shows running counts for evals and streaming threads.
 *
 * @module components/nav/app-nav
 */

import {
  ClipboardCheckIcon,
  DatabaseIcon,
  FlaskConicalIcon,
  MessageSquareIcon,
} from "lucide-react";
import { NavLink } from "react-router";
import {
  selectImportIsRunning,
  selectProfilingIsRunning,
  selectRunningEvalCount,
  selectSourceCount,
  selectStreamingThreadCount,
  selectTestCaseCount,
  useConversation,
  useEvaluation,
  useImportPipeline,
  useProfilingPipeline,
  useSourceCatalog,
  useTestSuite,
} from "~/lib/stores";
import { cn } from "~/lib/utils";

const navItems = [
  { to: "/connect", label: "Connect", icon: DatabaseIcon, badge: "sourceCount" },
  { to: "/playground", label: "Playground", icon: MessageSquareIcon, badge: "streamingCount" },
  { to: "/tests", label: "Tests", icon: ClipboardCheckIcon, badge: "testCount" },
  { to: "/evals", label: "Evals", icon: FlaskConicalIcon, badge: "evalCount" },
] as const;

type BadgeKind = (typeof navItems)[number]["badge"];

/** Render a badge for a nav item. Pure function of badge kind + store state. */
function NavBadge({ kind }: { readonly kind: BadgeKind }) {
  const testCount = useTestSuite(selectTestCaseCount);
  const runningEvalCount = useEvaluation(selectRunningEvalCount);
  const sourceCount = useSourceCatalog(selectSourceCount);
  const isProfilingRunning = useProfilingPipeline(selectProfilingIsRunning);
  const isImportRunning = useImportPipeline(selectImportIsRunning);
  const streamingThreadCount = useConversation(selectStreamingThreadCount);

  switch (kind) {
    case "sourceCount": {
      // Show pulsing dot when profiling or import is active, badge count otherwise
      const connectActive = isProfilingRunning || isImportRunning;
      if (connectActive) {
        return <span className="size-1.5 rounded-full bg-[var(--dot-blue)] animate-pulse ml-0.5" />;
      }
      return sourceCount > 0 ? (
        <span className="text-[9px] font-mono tabular-nums text-muted-foreground/60 ml-0.5">
          {sourceCount}
        </span>
      ) : null;
    }
    case "streamingCount":
      return streamingThreadCount > 0 ? (
        <span className="flex items-center gap-0.5 ml-0.5">
          <span className="size-1.5 rounded-full bg-[var(--dot-blue)] animate-pulse" />
          {streamingThreadCount > 1 && (
            <span className="text-[9px] font-mono tabular-nums text-muted-foreground/60">
              {streamingThreadCount}
            </span>
          )}
        </span>
      ) : null;
    case "testCount":
      return testCount > 0 ? (
        <span className="text-[9px] font-mono tabular-nums text-muted-foreground/60 ml-0.5">
          {testCount}
        </span>
      ) : null;
    case "evalCount":
      return runningEvalCount > 0 ? (
        <span className="flex items-center gap-0.5 ml-0.5">
          <span className="size-1.5 rounded-full bg-[var(--dot-blue)] animate-pulse" />
          {runningEvalCount > 1 && (
            <span className="text-[9px] font-mono tabular-nums text-muted-foreground/60">
              {runningEvalCount}
            </span>
          )}
        </span>
      ) : null;
    default:
      return null;
  }
}

export function AppNav() {
  return (
    <nav className="flex border-b items-stretch justify-center gap-1 px-1">
      {navItems.map(({ to, label, icon: Icon, badge }) => (
        <NavLink
          key={to}
          to={to}
          prefetch="intent"
          className={({ isActive }) =>
            cn(
              "group relative flex items-center justify-center gap-0.5 px-3 py-2.5 text-xs font-medium transition-colors",
              isActive
                ? "border-b-2 border-foreground text-foreground"
                : "text-muted-foreground hover:text-foreground"
            )
          }
        >
          <Icon className="size-3.5 flex-shrink-0" />
          <NavBadge kind={badge} />
          <span className="pointer-events-none absolute left-1/2 top-full z-50 -translate-x-1/2 mt-1 rounded bg-foreground px-2 py-1 text-[10px] text-background opacity-0 transition-opacity group-hover:opacity-100 whitespace-nowrap">
            {label}
          </span>
        </NavLink>
      ))}
    </nav>
  );
}
