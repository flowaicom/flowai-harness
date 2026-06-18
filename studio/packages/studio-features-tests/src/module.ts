import { defineStudioModule } from "@studio/core";

export const TESTS_STUDIO_MODULE = defineStudioModule({
  id: "tests",
  label: "Tests",
  route: "/tests",
  scope: "shared",
  surface: "tests",
  packageName: "@studio/features-tests",
  requiredCapabilities: ["tests.manage"],
});
