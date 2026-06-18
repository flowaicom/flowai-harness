import type { StudioStatus } from "~/lib/api/studio";

export type StudioRuntimeKind = "python" | "rust" | (string & {});

export interface StudioBridgeCapability {
  readonly name: string;
  readonly prefix: string;
  readonly backend: string;
  readonly configured: boolean;
  readonly available: boolean;
  readonly mode: "bridge" | (string & {});
  readonly reason?: string;
  readonly message?: string;
  readonly baseUrl?: string;
  readonly statusCode?: number;
}

export interface StudioCapabilities {
  readonly native: readonly string[];
  readonly bridges: Readonly<Record<string, StudioBridgeCapability>>;
  readonly available: readonly string[];
}
