import { ShieldIcon } from "lucide-react";
import { useCallback, useEffect } from "react";
import {
  isRouteErrorResponse,
  Links,
  Meta,
  Outlet,
  Scripts,
  ScrollRestoration,
} from "react-router";
import { BackgroundTasksIndicator } from "~/components/shared/background-tasks-indicator";
import { HarnessStudioShell } from "~/components/studio/harness-studio-shell";
import { WorkspaceChatScopeSync } from "~/components/workspace/workspace-chat-scope-sync";
import { HarnessRuntimeProvider } from "~/lib/runtime";
import { useSessionCleanup } from "~/lib/stores/session-registry";
import { useAgentConfig } from "~/lib/stores/settings-store";
import type { Route } from "./+types/root";

import "@radix-ui/themes/styles.css";
import "./app.css";
import "@studio/ui/styles.css";

export const links: Route.LinksFunction = () => [
  { rel: "icon", type: "image/svg+xml", href: "/favicon.svg" },
];

export function Layout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className="h-full" suppressHydrationWarning>
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <script
          dangerouslySetInnerHTML={{
            __html: `(function(){try{var w=JSON.parse(localStorage.getItem("studio-workspace")||"{}");var ns=(w.state&&w.state.activeWorkspaceId)||"default";var s=JSON.parse(localStorage.getItem("studio-settings-"+ns)||"{}");var t=(s.state&&s.state.theme)||"slate";if(t==="system"){t=matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light"}if(t==="dark"||t==="slate")document.documentElement.classList.add(t)}catch(e){}})()`,
          }}
        />
        <script src="/__flowai_config.js" />
        <Meta />
        <Links />
      </head>
      <body className="h-full bg-background text-foreground antialiased">
        {children}
        <ScrollRestoration />
        <Scripts />
      </body>
    </html>
  );
}

function DemoModeIndicator() {
  const enabled = useAgentConfig((s) => s.featureFlags.piiScramble);
  const setFeatureFlag = useAgentConfig((s) => s.setFeatureFlag);

  if (!enabled) return null;

  return (
    <button
      type="button"
      onClick={() => setFeatureFlag("piiScramble", false)}
      title="Click to disable Demo Mode"
      className="fixed bottom-4 right-4 z-50 flex items-center gap-1.5 bg-amber-500/90 text-white text-xs font-medium px-3 py-1.5 rounded-full shadow-lg backdrop-blur-sm hover:bg-amber-600/90 transition-colors animate-in fade-in-0 duration-300"
    >
      <ShieldIcon className="size-3" />
      Demo Mode
    </button>
  );
}

function ThemeApplicator() {
  const theme = useAgentConfig((s) => s.theme);

  useEffect(() => {
    const root = document.documentElement;

    const apply = (resolved: string) => {
      root.classList.remove("dark", "slate");
      if (resolved === "dark" || resolved === "slate") {
        root.classList.add(resolved);
      }
    };

    if (theme === "system") {
      const mq = window.matchMedia("(prefers-color-scheme: dark)");
      apply(mq.matches ? "dark" : "light");
      const handler = (e: MediaQueryListEvent) => apply(e.matches ? "dark" : "light");
      mq.addEventListener("change", handler);
      return () => mq.removeEventListener("change", handler);
    }

    apply(theme);
  }, [theme]);

  return null;
}

/**
 * Global keyboard shortcut: `,` (comma) opens settings dialog.
 *
 * Guards: skips when focus is in INPUT, TEXTAREA, or contentEditable elements.
 */
function useSettingsShortcut() {
  const toggleSettingsDialog = useAgentConfig((s) => s.toggleSettingsDialog);

  const handler = useCallback(
    (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      if (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable) {
        return;
      }
      if (e.key === "," && !e.isComposing) {
        e.preventDefault();
        toggleSettingsDialog();
      }
    },
    [toggleSettingsDialog]
  );

  useEffect(() => {
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [handler]);
}

export default function App() {
  useSessionCleanup();
  useSettingsShortcut();

  return (
    <>
      <ThemeApplicator />
      <HarnessRuntimeProvider>
        <WorkspaceChatScopeSync />
        <HarnessStudioShell>
          <Outlet />
        </HarnessStudioShell>
      </HarnessRuntimeProvider>
      <BackgroundTasksIndicator />
      <DemoModeIndicator />
    </>
  );
}

export function ErrorBoundary({ error }: Route.ErrorBoundaryProps) {
  let message = "Oops!";
  let details = "An unexpected error occurred.";
  let stack: string | undefined;

  if (isRouteErrorResponse(error)) {
    message = error.status === 404 ? "404" : "Error";
    details =
      error.status === 404 ? "The requested page could not be found." : error.statusText || details;
  } else if (import.meta.env.DEV && error && error instanceof Error) {
    details = error.message;
    stack = error.stack;
  }

  return (
    <main className="flex min-h-screen items-center justify-center p-4">
      <div className="max-w-md text-center">
        <h1 className="text-4xl font-bold text-foreground">{message}</h1>
        <p className="mt-4 text-muted-foreground">{details}</p>
        {stack && (
          <pre className="mt-4 overflow-auto rounded bg-muted p-4 text-left text-xs text-muted-foreground">
            {stack}
          </pre>
        )}
      </div>
    </main>
  );
}
