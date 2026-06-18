import type { CapabilitySummary } from "@studio/core/runtime";
import {
  Badge,
  StatusDot,
  type StudioLinkRenderer,
  StudioShell,
  type StudioShellNavItem,
} from "@studio/ui";
import {
  ClipboardCheckIcon,
  DatabaseIcon,
  FlaskConicalIcon,
  LayoutDashboardIcon,
  MessageSquareIcon,
  PlayCircleIcon,
  SettingsIcon,
  ShieldCheckIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useMemo, useState } from "react";
import { Link, useLocation } from "react-router";
import { SettingsDialog } from "~/components/settings";
import { WorkspaceSwitcher } from "~/components/workspace/workspace-switcher";
import { useHarnessRuntime } from "~/lib/runtime";
import { getFlowAIStudioConfig } from "~/lib/studio-config/flowai-config";
import {
  deriveStudioModules,
  type StudioSurfaceNavItem,
  studioModulesToNavItems,
} from "~/lib/studio-nav/studio-module-registry";

type CapabilityState =
  | { readonly kind: "loading" }
  | { readonly kind: "ready"; readonly capabilities: readonly CapabilitySummary[] }
  | { readonly kind: "fallback"; readonly reason: string };

const FALLBACK_NAV: readonly StudioSurfaceNavItem[] = [
  {
    id: "overview",
    label: "Overview",
    href: "/workspace",
    enabled: true,
    active: false,
    requiredCapabilities: ["runtime.inspect"],
  },
  {
    id: "playground",
    label: "Playground",
    href: "/playground",
    enabled: true,
    active: false,
    requiredCapabilities: ["chat.stream"],
  },
  {
    id: "connect",
    label: "Connect",
    href: "/connect",
    enabled: true,
    active: false,
    requiredCapabilities: ["data.sources", "data.profile", "knowledge.ingest", "tools.inspect"],
  },
  {
    id: "tests",
    label: "Tests",
    href: "/tests",
    enabled: true,
    active: false,
    requiredCapabilities: ["tests.manage"],
  },
  {
    id: "evals",
    label: "Evals",
    href: "/evals",
    enabled: true,
    active: false,
    requiredCapabilities: ["evals.run"],
  },
];

const iconByModuleId: Record<string, StudioShellNavItem["icon"]> = {
  overview: LayoutDashboardIcon,
  playground: MessageSquareIcon,
  connect: DatabaseIcon,
  tests: ClipboardCheckIcon,
  evals: FlaskConicalIcon,
  runs: PlayCircleIcon,
  approvals: ShieldCheckIcon,
};

const renderLink: StudioLinkRenderer = ({ href, className, children }) => (
  <Link to={href} className={className}>
    {children}
  </Link>
);

function isActive(pathname: string, href: string, id: string): boolean {
  if (id === "playground" && pathname.startsWith("/chat")) return true;
  return pathname === href || pathname.startsWith(`${href}/`);
}

function toShellNavItems(
  items: readonly StudioSurfaceNavItem[],
  pathname: string
): readonly StudioShellNavItem[] {
  return items.map((item) => ({
    id: item.id,
    label: item.label,
    href: item.href,
    icon: iconByModuleId[item.id],
    active: isActive(pathname, item.href, item.id),
    disabledReason: item.enabled ? undefined : (item.reason ?? "Capability is not available."),
  }));
}

function RuntimeStatus({ state }: { readonly state: CapabilityState }) {
  if (state.kind === "ready") {
    const enabledCount = state.capabilities.filter((capability) => capability.enabled).length;
    return (
      <div className="flex items-center justify-between gap-2 text-[11px] text-[var(--fg-5)]">
        <span className="inline-flex items-center gap-1.5">
          <StatusDot tone="green" label="Runtime ready" />
          Runtime ready
        </span>
        <Badge tone="neutral">{enabledCount} caps</Badge>
      </div>
    );
  }

  if (state.kind === "fallback") {
    return (
      <div className="flex items-center gap-1.5 text-[11px] text-[var(--fg-5)]">
        <StatusDot tone="amber" label="Static navigation fallback" />
        Static nav fallback
      </div>
    );
  }

  return (
    <div className="flex items-center gap-1.5 text-[11px] text-[var(--fg-5)]">
      <StatusDot tone="blue" pulse label="Loading runtime capabilities" />
      Loading capabilities
    </div>
  );
}

function SettingsUtility() {
  return (
    <SettingsDialog>
      <span className="flex items-center gap-2 rounded-lg px-2.5 py-2 text-xs font-medium text-[var(--fg-4)] transition-colors hover:bg-[var(--layer-04)] hover:text-[var(--fg-1)]">
        <SettingsIcon className="size-4" />
        Settings
      </span>
    </SettingsDialog>
  );
}

export function HarnessStudioShell({ children }: { readonly children: ReactNode }) {
  const location = useLocation();
  const { adapter, scope } = useHarnessRuntime();
  const config = useMemo(() => getFlowAIStudioConfig(), []);
  const [capabilityState, setCapabilityState] = useState<CapabilityState>({ kind: "loading" });

  useEffect(() => {
    let cancelled = false;
    setCapabilityState({ kind: "loading" });

    adapter
      .getCapabilities(scope)
      .then((result) => {
        if (cancelled) return;
        if (result._tag === "Ok") {
          setCapabilityState({ kind: "ready", capabilities: result.value.capabilities });
        } else {
          setCapabilityState({ kind: "fallback", reason: result.error.message });
        }
      })
      .catch((error) => {
        if (cancelled) return;
        setCapabilityState({
          kind: "fallback",
          reason: error instanceof Error ? error.message : "Could not load capabilities.",
        });
      });

    return () => {
      cancelled = true;
    };
  }, [adapter, scope]);

  const navItems = useMemo(() => {
    if (capabilityState.kind !== "ready") {
      return toShellNavItems(FALLBACK_NAV, location.pathname);
    }

    const moduleViews = deriveStudioModules({
      capabilities: capabilityState.capabilities,
      pathname: location.pathname,
      hostMode: "local",
    });
    return toShellNavItems(studioModulesToNavItems(moduleViews), location.pathname);
  }, [capabilityState, location.pathname]);

  return (
    <StudioShell
      appName={config.appName}
      navItems={navItems}
      workspaceControl={<WorkspaceSwitcher variant="inline" />}
      runtimeStatus={<RuntimeStatus state={capabilityState} />}
      utilitySlot={<SettingsUtility />}
      renderLink={renderLink}
    >
      {children}
    </StudioShell>
  );
}
