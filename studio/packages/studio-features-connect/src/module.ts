import { defineStudioModule } from "@studio/core";

export const CONNECT_STUDIO_MODULE = defineStudioModule({
  id: "connect",
  label: "Connect",
  route: "/connect",
  scope: "shared",
  surface: "connect",
  packageName: "@studio/features-connect",
  requiredCapabilities: ["data.sources", "data.profile", "knowledge.ingest", "tools.inspect"],
  capabilityMode: "any",
});
