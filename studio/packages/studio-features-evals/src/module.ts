import { defineStudioModule } from "@studio/core";

export const EVALS_STUDIO_MODULE = defineStudioModule({
  id: "evals",
  label: "Evals",
  route: "/evals",
  scope: "shared",
  surface: "evals",
  packageName: "@studio/features-evals",
  requiredCapabilities: ["evals.run"],
});
