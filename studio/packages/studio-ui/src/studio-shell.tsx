import { MenuIcon } from "lucide-react";
import type { ComponentType, ReactNode } from "react";
import { useState } from "react";
import { defaultLinkRenderer, type StudioLinkRenderer } from "./page-shell";
import { FlowMark } from "./primitives";
import { cn } from "./utils/cn";

export interface StudioShellNavItem {
  readonly id: string;
  readonly label: string;
  readonly href: string;
  readonly icon?: ComponentType<{ readonly className?: string }>;
  readonly active?: boolean;
  readonly badge?: ReactNode;
  readonly disabledReason?: string;
}

export interface StudioShellProps {
  readonly appName: string;
  readonly navItems: readonly StudioShellNavItem[];
  readonly children: ReactNode;
  readonly workspaceControl?: ReactNode;
  readonly runtimeStatus?: ReactNode;
  readonly footerItems?: readonly StudioShellNavItem[];
  readonly utilitySlot?: ReactNode;
  readonly renderLink?: StudioLinkRenderer;
  readonly className?: string;
}

function NavItem({
  item,
  renderLink,
}: {
  readonly item: StudioShellNavItem;
  readonly renderLink: StudioLinkRenderer;
}) {
  const Icon = item.icon;
  const content = (
    <>
      {Icon ? <Icon className="size-4 shrink-0" /> : null}
      <span className="flex-1 truncate">{item.label}</span>
      {item.badge ? <span className="shrink-0">{item.badge}</span> : null}
    </>
  );
  const classes = cn(
    "flex items-center gap-2.5 rounded-lg px-2.5 py-[7px] text-xs font-medium leading-4 transition-colors",
    item.active
      ? "bg-[var(--layer-06)] text-[var(--fg-1)]"
      : "text-[var(--fg-4)] hover:bg-[var(--layer-04)] hover:text-[var(--fg-1)]",
    item.disabledReason &&
      "cursor-not-allowed opacity-45 hover:bg-transparent hover:text-[var(--fg-4)]"
  );

  if (item.disabledReason) {
    return (
      <span className={classes} title={item.disabledReason} aria-disabled="true">
        {content}
      </span>
    );
  }

  return <>{renderLink({ href: item.href, className: classes, children: content })}</>;
}

function StudioSidebar({
  appName,
  navItems,
  footerItems = [],
  workspaceControl,
  runtimeStatus,
  utilitySlot,
  renderLink = defaultLinkRenderer,
  mobile = false,
}: Omit<StudioShellProps, "children" | "className"> & {
  readonly mobile?: boolean;
}) {
  return (
    <aside
      className={cn(
        "w-[232px] shrink-0 flex-col border-r border-[var(--layer-08)] bg-[var(--chrome-base)] text-[var(--fg-1)]",
        mobile ? "flex h-full" : "hidden md:flex"
      )}
    >
      <div className="px-3 pb-3 pt-3.5">
        {workspaceControl ?? (
          <div className="flex items-center gap-2 rounded-[10px] border border-[var(--layer-08)] bg-[var(--layer-04)] px-2.5 py-2">
            <FlowMark size={24} />
            <div className="min-w-0">
              <span className="studio-eyebrow block text-[9px]">Studio</span>
              <span className="block truncate text-xs font-medium">{appName}</span>
            </div>
          </div>
        )}
      </div>

      <nav className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-2 py-1">
        {navItems.map((item) => (
          <NavItem key={item.id} item={item} renderLink={renderLink} />
        ))}
      </nav>

      <div className="flex flex-col gap-0.5 border-t border-[var(--layer-08)] p-2">
        {runtimeStatus ? <div className="px-2.5 py-1.5">{runtimeStatus}</div> : null}
        {footerItems.map((item) => (
          <NavItem key={item.id} item={item} renderLink={renderLink} />
        ))}
        {utilitySlot ? <div className="mt-1.5">{utilitySlot}</div> : null}
      </div>
    </aside>
  );
}

export function StudioShell({
  appName,
  navItems,
  footerItems,
  workspaceControl,
  runtimeStatus,
  utilitySlot,
  renderLink = defaultLinkRenderer,
  className,
  children,
}: StudioShellProps) {
  const [mobileOpen, setMobileOpen] = useState(false);

  return (
    <main
      className={cn(
        "flex h-dvh overflow-hidden bg-[var(--chrome-mid)] text-[var(--fg-1)]",
        className
      )}
    >
      <StudioSidebar
        appName={appName}
        navItems={navItems}
        footerItems={footerItems}
        workspaceControl={workspaceControl}
        runtimeStatus={runtimeStatus}
        utilitySlot={utilitySlot}
        renderLink={renderLink}
      />
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <div className="flex h-12 shrink-0 items-center border-b border-[var(--layer-08)] bg-[var(--chrome-base)] px-3 md:hidden">
          <button
            type="button"
            onClick={() => setMobileOpen(true)}
            className="flex size-8 items-center justify-center rounded-lg text-[var(--fg-4)] hover:bg-[var(--layer-04)] hover:text-[var(--fg-1)]"
            aria-label="Open navigation"
          >
            <MenuIcon className="size-4" />
          </button>
          <span className="ml-2 truncate text-sm font-semibold">{appName}</span>
        </div>
        {children}
      </div>

      {mobileOpen ? (
        <div className="fixed inset-0 z-50 md:hidden">
          <button
            type="button"
            className="absolute inset-0 bg-black/50"
            aria-label="Close navigation"
            onClick={() => setMobileOpen(false)}
          />
          <div className="relative h-full w-[min(84vw,260px)] border-r border-[var(--layer-08)] bg-[var(--chrome-base)] shadow-xl">
            <StudioSidebar
              appName={appName}
              navItems={navItems}
              footerItems={footerItems}
              workspaceControl={workspaceControl}
              runtimeStatus={runtimeStatus}
              utilitySlot={utilitySlot}
              renderLink={renderLink}
              mobile
            />
          </div>
        </div>
      ) : null}
    </main>
  );
}
