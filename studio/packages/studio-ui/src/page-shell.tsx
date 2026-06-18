import type { ReactNode } from "react";
import { cn } from "./utils/cn";

export type StudioLinkRenderer = (props: {
  readonly href: string;
  readonly className: string;
  readonly children: ReactNode;
}) => ReactNode;

export function defaultLinkRenderer({
  href,
  className,
  children,
}: {
  readonly href: string;
  readonly className: string;
  readonly children: ReactNode;
}) {
  return (
    <a href={href} className={className}>
      {children}
    </a>
  );
}

export function PageShell({
  children,
  className,
}: {
  readonly children: ReactNode;
  readonly className?: string;
}) {
  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className={cn("mx-auto w-full max-w-6xl px-6 py-8", className)}>{children}</div>
    </div>
  );
}

export interface SurfaceCrumb {
  readonly label: ReactNode;
  readonly href?: string;
}

export function SurfaceHeader({
  crumbs,
  actions,
  className,
  renderLink = defaultLinkRenderer,
}: {
  readonly crumbs: readonly SurfaceCrumb[];
  readonly actions?: ReactNode;
  readonly className?: string;
  readonly renderLink?: StudioLinkRenderer;
}) {
  return (
    <header
      className={cn(
        "flex h-12 shrink-0 items-center gap-3 border-b border-[var(--layer-06)] bg-[var(--chrome-mid)]/95 px-6 backdrop-blur",
        className
      )}
    >
      <nav aria-label="Breadcrumb" className="flex min-w-0 items-center gap-1.5 text-xs">
        {crumbs.map((crumb, index) => {
          const last = index === crumbs.length - 1;
          const toneClass = last
            ? "min-w-0 truncate text-[var(--fg-1)]"
            : "font-medium text-[var(--fg-4)] transition-colors hover:text-[var(--fg-2)]";
          return (
            <div
              key={`${crumb.href ?? "crumb"}-${index}`}
              className="flex min-w-0 items-center gap-1.5"
            >
              {index > 0 ? <span className="text-[var(--fg-5)]">/</span> : null}
              {crumb.href && !last ? (
                renderLink({ href: crumb.href, className: toneClass, children: crumb.label })
              ) : (
                <span className={toneClass}>{crumb.label}</span>
              )}
            </div>
          );
        })}
      </nav>
      {actions ? <div className="ml-auto flex items-center gap-1.5">{actions}</div> : null}
    </header>
  );
}

export function StudioHeader({
  eyebrow,
  title,
  description,
  actions,
  className,
}: {
  readonly eyebrow?: ReactNode;
  readonly title: ReactNode;
  readonly description?: ReactNode;
  readonly actions?: ReactNode;
  readonly className?: string;
}) {
  return (
    <header
      className={cn(
        "mb-8 flex flex-col gap-4 md:flex-row md:items-start md:justify-between",
        className
      )}
    >
      <div className="min-w-0">
        {eyebrow ? <div className="studio-eyebrow mb-2">{eyebrow}</div> : null}
        <h1 className="text-3xl font-medium tracking-tight text-[var(--fg-1)]">{title}</h1>
        {description ? (
          <p className="mt-2 max-w-2xl text-sm leading-6 text-[var(--fg-5)]">{description}</p>
        ) : null}
      </div>
      {actions ? <div className="flex shrink-0 items-center gap-2">{actions}</div> : null}
    </header>
  );
}
