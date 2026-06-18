import { describe, expect, test } from "bun:test";
import { readFile } from "node:fs/promises";

const packageBackedChatComponents = [
  { file: "app/components/chat/message-input.tsx", component: "SharedMessageInput" },
  { file: "app/components/chat/thread-sidebar.tsx", component: "SharedThreadSidebarView" },
  {
    file: "app/components/chat/virtualized-message-list.tsx",
    component: "SharedVirtualizedMessageList",
  },
] as const;

describe("Chat package-backed component composition", () => {
  for (const target of packageBackedChatComponents) {
    test(`${target.file} delegates rendering to ${target.component}`, async () => {
      const source = await readFile(target.file, "utf8");

      expect(source).toContain("@studio/features-chat");
      expect(source).toContain(target.component);
    });
  }
});
