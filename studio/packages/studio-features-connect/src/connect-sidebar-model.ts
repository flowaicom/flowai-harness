export interface ConnectSourceLike {
  readonly isActive: boolean;
}

export interface ConnectSourceStatusLike {
  readonly status: "connected" | "disconnected" | "error" | string;
  readonly message?: string | null;
}

export function getConnectSourceStatusDotClass(
  source: ConnectSourceLike,
  status?: ConnectSourceStatusLike
): string {
  if (!source.isActive) return "bg-muted-foreground/30";
  switch (status?.status) {
    case "connected":
      return "bg-[var(--dot-emerald)]";
    case "disconnected":
      return "bg-[var(--dot-amber)]";
    case "error":
      return "bg-[var(--dot-red)]";
    default:
      return "bg-[var(--dot-emerald)]/50";
  }
}

export function getConnectSourceStatusTitle(
  source: ConnectSourceLike,
  status?: ConnectSourceStatusLike
): string {
  if (!source.isActive) return "Inactive";
  switch (status?.status) {
    case "connected":
      return "Connected";
    case "disconnected":
      return "Disconnected";
    case "error":
      return `Error: ${status.message ?? ""}`.trim();
    default:
      return "Unknown";
  }
}
