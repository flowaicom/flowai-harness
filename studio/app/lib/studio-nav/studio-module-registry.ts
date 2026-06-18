import {
  type DeriveStudioModulesInput,
  defineStudioModule,
  type StudioCapabilityLike,
  type StudioModuleDefinition,
  type StudioModuleView,
  type StudioSurfaceNavItem,
} from "@studio/core";
import { CHAT_STUDIO_MODULE } from "@studio/features-chat";
import { CONNECT_STUDIO_MODULE } from "@studio/features-connect";
import { EVALS_STUDIO_MODULE } from "@studio/features-evals";
import { TESTS_STUDIO_MODULE } from "@studio/features-tests";

export type {
  CapabilityMode,
  DeriveStudioModulesInput,
  StudioCapabilityLike,
  StudioHostMode,
  StudioModuleDefinition,
  StudioModuleScope,
  StudioModuleState,
  StudioModuleView,
  StudioSurfaceNavItem,
} from "@studio/core";

export const DEFAULT_STUDIO_MODULES: readonly StudioModuleDefinition[] = [
  defineStudioModule({
    id: "overview",
    label: "Overview",
    route: "/workspace",
    scope: "shared",
    surface: "overview",
    requiredCapabilities: ["runtime.inspect"],
  }),
  CHAT_STUDIO_MODULE,
  CONNECT_STUDIO_MODULE,
  TESTS_STUDIO_MODULE,
  EVALS_STUDIO_MODULE,
  defineStudioModule({
    id: "runs",
    label: "Runs",
    route: "/runs",
    scope: "shared",
    surface: "runs",
    requiredCapabilities: ["runs.list", "traces.read"],
    capabilityMode: "any",
  }),
  defineStudioModule({
    id: "approvals",
    label: "Approvals",
    route: "/approvals",
    scope: "shared",
    surface: "approvals",
    requiredCapabilities: ["approvals.decide"],
    planned: true,
    plannedVisibility: "hidden",
  }),
  defineStudioModule({
    id: "enterprise-admin",
    label: "Admin",
    route: "/enterprise/admin",
    scope: "enterprise",
    surface: "enterprise",
    packageName: "@studio/features-enterprise",
    requiredCapabilities: ["enterprise.orgAdmin"],
    enterpriseOnly: true,
  }),
];

function capabilityMap(
  capabilities: readonly StudioCapabilityLike[]
): Map<string, StudioCapabilityLike> {
  return new Map(capabilities.map((capability) => [capability.id, capability]));
}

function routeIsActive(pathname: string, route: string): boolean {
  return pathname === route || pathname.startsWith(`${route}/`);
}

function capabilityRequirementSatisfied(
  definition: StudioModuleDefinition,
  enabledCapabilities: readonly string[]
): boolean {
  if (definition.requiredCapabilities.length === 0) return true;
  if (definition.capabilityMode === "any") return enabledCapabilities.length > 0;
  return enabledCapabilities.length === definition.requiredCapabilities.length;
}

function disabledReason(
  definition: StudioModuleDefinition,
  disabledCapabilities: readonly string[],
  missingCapabilities: readonly string[]
): string {
  if (disabledCapabilities.length > 0) {
    return `Disabled capabilities: ${disabledCapabilities.join(", ")}`;
  }
  return `Missing capabilities: ${missingCapabilities.join(", ") || definition.requiredCapabilities.join(", ")}`;
}

export function deriveStudioModules({
  capabilities,
  pathname = "/",
  hostMode = "local",
  modules = DEFAULT_STUDIO_MODULES,
}: DeriveStudioModulesInput): readonly StudioModuleView[] {
  const byId = capabilityMap(capabilities);

  return modules.map((definition) => {
    if (definition.enterpriseOnly && hostMode !== "enterprise") {
      return {
        definition,
        state: "hidden",
        reason: "Enterprise module is hidden for local Studio.",
        active: false,
        enabledCapabilities: [],
        disabledCapabilities: [],
        missingCapabilities: definition.requiredCapabilities,
      };
    }

    if (definition.localOnly && hostMode !== "local") {
      return {
        definition,
        state: "hidden",
        reason: "Local module is hidden for Enterprise Studio.",
        active: false,
        enabledCapabilities: [],
        disabledCapabilities: [],
        missingCapabilities: definition.requiredCapabilities,
      };
    }

    const enabledCapabilities = definition.requiredCapabilities.filter(
      (capabilityId) => byId.get(capabilityId)?.enabled === true
    );
    const disabledCapabilities = definition.requiredCapabilities.filter((capabilityId) => {
      const capability = byId.get(capabilityId);
      return capability && !capability.enabled;
    });
    const missingCapabilities = definition.requiredCapabilities.filter(
      (capabilityId) => !byId.has(capabilityId)
    );

    const active = routeIsActive(pathname, definition.route);
    if (definition.planned) {
      const state = definition.plannedVisibility ?? "disabled";
      return {
        definition,
        state,
        reason: "Module is planned for a later Studio milestone.",
        active: state === "hidden" ? false : active,
        enabledCapabilities,
        disabledCapabilities,
        missingCapabilities,
      };
    }

    const satisfied = capabilityRequirementSatisfied(definition, enabledCapabilities);
    if (satisfied) {
      return {
        definition,
        state: "enabled",
        active,
        enabledCapabilities,
        disabledCapabilities,
        missingCapabilities,
      };
    }

    const state =
      missingCapabilities.length === definition.requiredCapabilities.length ? "hidden" : "disabled";
    return {
      definition,
      state,
      reason: disabledReason(definition, disabledCapabilities, missingCapabilities),
      active: state === "hidden" ? false : active,
      enabledCapabilities,
      disabledCapabilities,
      missingCapabilities,
    };
  });
}

export function studioModulesToNavItems(
  modules: readonly StudioModuleView[]
): readonly StudioSurfaceNavItem[] {
  return modules
    .filter((module) => module.state !== "hidden")
    .map((module) => ({
      id: module.definition.id,
      label: module.definition.label,
      href: module.definition.route,
      enabled: module.state === "enabled",
      active: module.active,
      reason: module.reason,
      requiredCapabilities: module.definition.requiredCapabilities,
    }));
}
