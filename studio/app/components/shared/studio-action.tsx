import type { LucideIcon } from "lucide-react";
import type { ButtonHTMLAttributes, ReactNode } from "react";
import type { LinkProps } from "react-router";
import { Link } from "react-router";
import { cn } from "~/lib/utils";

type StudioActionTone = "default" | "strong";

const studioActionBaseClass =
  "inline-flex items-center justify-center gap-1.5 whitespace-nowrap rounded-md border border-border/70 bg-background/80 px-2.5 py-1.5 text-xs font-medium transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

function studioActionClass(tone: StudioActionTone, className?: string) {
  return cn(
    studioActionBaseClass,
    tone === "strong" ? "text-foreground" : "text-muted-foreground",
    className
  );
}

function ActionContents({
  icon: Icon,
  children,
}: {
  readonly icon?: LucideIcon;
  readonly children: ReactNode;
}) {
  return (
    <>
      {Icon && <Icon aria-hidden="true" className="size-3.5" />}
      {children}
    </>
  );
}

export function StudioActionButton({
  icon,
  tone = "default",
  className,
  children,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  readonly icon?: LucideIcon;
  readonly tone?: StudioActionTone;
}) {
  return (
    <button
      type="button"
      className={studioActionClass(
        tone,
        cn("disabled:cursor-not-allowed disabled:opacity-40", className)
      )}
      {...props}
    >
      <ActionContents icon={icon}>{children}</ActionContents>
    </button>
  );
}

export function StudioActionLink({
  icon,
  tone = "default",
  className,
  children,
  ...props
}: LinkProps & {
  readonly icon?: LucideIcon;
  readonly tone?: StudioActionTone;
}) {
  return (
    <Link className={studioActionClass(tone, className)} {...props}>
      <ActionContents icon={icon}>{children}</ActionContents>
    </Link>
  );
}
