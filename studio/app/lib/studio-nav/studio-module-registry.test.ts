import { describe, expect, test } from "bun:test";
import {
  deriveStudioModules,
  type StudioCapabilityLike,
  type StudioModuleDefinition,
  studioModulesToNavItems,
} from "./studio-module-registry";

const localCapabilities: readonly StudioCapabilityLike[] = [
  { id: "runtime.inspect", enabled: true, scope: "local" },
  { id: "chat.stream", enabled: true, scope: "local" },
  { id: "data.profile", enabled: true, scope: "local" },
  { id: "tests.manage", enabled: false, scope: "local", reason: "Not wired" },
  { id: "evals.run", enabled: true, scope: "local" },
  { id: "approvals.decide", enabled: true, scope: "local" },
  { id: "enterprise.orgAdmin", enabled: false, scope: "enterprise" },
];

describe("Studio module registry", () => {
  test("derives nav from enabled, disabled, hidden, and planned modules", () => {
    const modules = deriveStudioModules({
      capabilities: localCapabilities,
      pathname: "/playground/thread-1",
    });
    const nav = studioModulesToNavItems(modules);

    expect(nav.map((item) => item.id)).toEqual([
      "overview",
      "playground",
      "connect",
      "tests",
      "evals",
    ]);
    expect(nav.find((item) => item.id === "playground")?.active).toBe(true);
    expect(nav.find((item) => item.id === "connect")?.enabled).toBe(true);
    expect(nav.find((item) => item.id === "tests")?.enabled).toBe(false);
    expect(nav.some((item) => item.id === "approvals")).toBe(false);
    expect(nav.some((item) => item.id === "enterprise-admin")).toBe(false);
  });

  test("supports enterprise-only modules from the same registry", () => {
    const modules = deriveStudioModules({
      hostMode: "enterprise",
      capabilities: [
        { id: "runtime.inspect", enabled: true },
        { id: "enterprise.orgAdmin", enabled: true, scope: "enterprise" },
      ],
    });
    const nav = studioModulesToNavItems(modules);

    expect(nav.find((item) => item.id === "enterprise-admin")?.enabled).toBe(true);
  });

  test("hides modules when all required capabilities are absent", () => {
    const modules = deriveStudioModules({
      capabilities: [],
      modules: [
        {
          id: "connect",
          label: "Connect",
          route: "/connect",
          scope: "shared",
          requiredCapabilities: ["data.sources", "data.profile"],
          capabilityMode: "any",
        } satisfies StudioModuleDefinition,
      ],
    });

    expect(modules[0]?.state).toBe("hidden");
    expect(studioModulesToNavItems(modules)).toEqual([]);
  });
});
