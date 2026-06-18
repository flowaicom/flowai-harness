import { defineStudioModule } from "@studio/core";

export const CHAT_STUDIO_MODULE = defineStudioModule({
  id: "playground",
  label: "Playground",
  route: "/playground",
  scope: "shared",
  surface: "chat",
  packageName: "@studio/features-chat",
  requiredCapabilities: ["chat.stream"],
});
