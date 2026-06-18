/**
 * Studio-specific API functions.
 *
 * @module api/studio
 */

import type { Result } from "~/lib/domain/result";
import type { StudioCapabilities, StudioRuntimeKind } from "~/lib/domain/studio-capabilities";
import type { ApiError } from "./client";
import { get, post } from "./client";

export interface StudioStatus {
  readonly runtime: RuntimeStatus;
  readonly runtimeKind?: StudioRuntimeKind;
  readonly project: string | null;
  readonly capabilities?: StudioCapabilities;
}

export type RuntimeStatus =
  | "starting"
  | "healthy"
  | { unhealthy: { error: string } }
  | "stopped"
  | "restarting";

export interface Preset {
  readonly label: string;
  readonly prompt: string;
}

export interface ProjectRoleResponseContract {
  readonly modelRef: string;
  readonly modelName: string;
  readonly schema: Record<string, unknown>;
}

export interface ProjectRoleConfig {
  readonly tools: readonly string[];
  readonly delegatesTo?: readonly string[];
  readonly responseContract?: ProjectRoleResponseContract;
}

export interface ProjectTopologyRole {
  readonly name: string;
  readonly tools: readonly string[];
  readonly delegatesTo: readonly string[];
}

export interface ProjectConfig {
  readonly config: {
    readonly project: { name: string; version: string };
    readonly agent: {
      provider: string;
      model: string;
      roles: Record<string, ProjectRoleConfig>;
      topology?: {
        readonly roles: readonly ProjectTopologyRole[];
      };
    };
    readonly workspaces: Record<string, { display_name: string; database_url?: string }>;
    readonly studio?: {
      license_key?: string;
      presets?: Preset[];
    };
  };
  readonly rootDir: string;
}

export interface ModelConfig {
  readonly models: ReadonlyArray<{
    readonly key: string;
    readonly model: string;
    readonly displayName: string;
    readonly description: string;
    readonly available: boolean;
    readonly endpointTransport?: string | null;
    readonly endpointTransportDisplayName?: string | null;
    readonly endpointTransportDescription?: string | null;
    readonly endpointSettings: ReadonlyArray<{
      readonly key: string;
      readonly displayName: string;
      readonly description: string;
      readonly kind: "secret" | "text" | "select";
      readonly required?: boolean;
      readonly defaultValue?: string | null;
      readonly options: ReadonlyArray<{
        readonly key: string;
        readonly displayName: string;
        readonly description: string;
      }>;
    }>;
    readonly settings: ReadonlyArray<{
      readonly key: string;
      readonly displayName: string;
      readonly description: string;
      readonly kind: "secret" | "text" | "select";
      readonly required?: boolean;
      readonly defaultValue?: string | null;
      readonly options: ReadonlyArray<{
        readonly key: string;
        readonly displayName: string;
        readonly description: string;
      }>;
    }>;
  }>;
  readonly agents: ReadonlyArray<{
    readonly role: string;
    readonly displayName: string;
    readonly description: string;
  }>;
  readonly defaultModels: Readonly<Record<string, string>>;
}

export async function getStudioStatus(): Promise<Result<StudioStatus, ApiError>> {
  return get<StudioStatus>("/studio/status");
}

export async function getProjectConfig(): Promise<Result<ProjectConfig, ApiError>> {
  return get<ProjectConfig>("/studio/project");
}

export async function getModelConfig(): Promise<Result<ModelConfig, ApiError>> {
  return get<ModelConfig>("/model-config");
}

export interface VerifyConnectionRequest {
  readonly provider?: string;
  readonly verifier?: string;
  readonly settings: Readonly<Record<string, string>>;
}

export interface VerifyConnectionResponse {
  readonly connected: boolean;
  readonly latencyMs?: number | null;
  readonly error?: string | null;
  readonly models?: ReadonlyArray<string> | null;
}

export async function verifyConnection(
  request: VerifyConnectionRequest
): Promise<Result<VerifyConnectionResponse, ApiError>> {
  return post<VerifyConnectionResponse>("/verify-connection", request);
}
