export type StudioModuleScope = "local" | "enterprise" | "shared";
export type StudioModuleState = "enabled" | "disabled" | "hidden";
export type StudioHostMode = "local" | "enterprise";
export type CapabilityMode = "all" | "any";

export type StudioModuleSurface =
  | "overview"
  | "chat"
  | "connect"
  | "tests"
  | "evals"
  | "research"
  | "runs"
  | "approvals"
  | "enterprise"
  | (string & {});

export type StudioFeaturePackageName =
  | "@studio/features-chat"
  | "@studio/features-connect"
  | "@studio/features-tests"
  | "@studio/features-evals"
  | (string & {});

export interface StudioCapabilityLike {
  readonly id: string;
  readonly enabled: boolean;
  readonly scope?: string;
  readonly reason?: string;
  readonly requirements?: readonly string[];
}

export interface StudioModuleDefinition {
  readonly id: string;
  readonly label: string;
  readonly route: string;
  readonly scope: StudioModuleScope;
  readonly requiredCapabilities: readonly string[];
  readonly packageName?: StudioFeaturePackageName;
  readonly surface?: StudioModuleSurface;
  readonly optionalCapabilities?: readonly string[];
  readonly capabilityMode?: CapabilityMode;
  readonly enterpriseOnly?: boolean;
  readonly localOnly?: boolean;
  readonly planned?: boolean;
  readonly plannedVisibility?: "disabled" | "hidden";
}

export interface StudioModuleView {
  readonly definition: StudioModuleDefinition;
  readonly state: StudioModuleState;
  readonly reason?: string;
  readonly active: boolean;
  readonly enabledCapabilities: readonly string[];
  readonly disabledCapabilities: readonly string[];
  readonly missingCapabilities: readonly string[];
}

export interface StudioSurfaceNavItem {
  readonly id: string;
  readonly label: string;
  readonly href: string;
  readonly enabled: boolean;
  readonly active: boolean;
  readonly reason?: string;
  readonly requiredCapabilities: readonly string[];
}

export interface DeriveStudioModulesInput {
  readonly capabilities: readonly StudioCapabilityLike[];
  readonly pathname?: string;
  readonly hostMode?: StudioHostMode;
  readonly modules?: readonly StudioModuleDefinition[];
}

export function defineStudioModule<const TDefinition extends StudioModuleDefinition>(
  definition: TDefinition
): TDefinition {
  return definition;
}
