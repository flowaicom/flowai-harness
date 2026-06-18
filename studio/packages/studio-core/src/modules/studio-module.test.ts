import { describe, expect, test } from "bun:test";
import { CHAT_STUDIO_MODULE } from "@studio/features-chat";
import { CONNECT_STUDIO_MODULE } from "@studio/features-connect";
import { EVALS_STUDIO_MODULE } from "@studio/features-evals";
import { TESTS_STUDIO_MODULE } from "@studio/features-tests";
import {
  defineStudioModule,
  type StudioModuleDefinition,
  type StudioModuleSurface,
} from "./studio-module";

const featureModules = [
  CHAT_STUDIO_MODULE,
  CONNECT_STUDIO_MODULE,
  EVALS_STUDIO_MODULE,
  TESTS_STUDIO_MODULE,
] satisfies readonly StudioModuleDefinition[];

describe("Studio module definitions", () => {
  test("feature packages export manifests through the shared module contract", () => {
    expect(featureModules.map((module) => module.id)).toEqual([
      "playground",
      "connect",
      "evals",
      "tests",
    ]);
    expect(featureModules.map((module) => module.surface satisfies StudioModuleSurface)).toEqual([
      "chat",
      "connect",
      "evals",
      "tests",
    ]);
    expect(
      featureModules.every((module) => module.packageName?.startsWith("@studio/features-"))
    ).toBe(true);
  });

  test("keeps host gating metadata explicit on module definitions", () => {
    const module: StudioModuleDefinition = defineStudioModule({
      id: "enterprise-admin",
      label: "Admin",
      route: "/enterprise/admin",
      scope: "enterprise",
      surface: "enterprise",
      packageName: "@studio/features-enterprise",
      requiredCapabilities: ["enterprise.orgAdmin"],
      enterpriseOnly: true,
    });

    expect(module.enterpriseOnly).toBe(true);
    expect(module.localOnly).toBeUndefined();
    expect(module.capabilityMode ?? "all").toBe("all");
  });
});
