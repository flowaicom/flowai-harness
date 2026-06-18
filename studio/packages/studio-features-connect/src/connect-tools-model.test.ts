import { describe, expect, test } from "bun:test";
import {
  CONNECT_TOOLS_SECTION_TITLE,
  getConnectToolDescriptionBlocks,
  getConnectToolsSection,
} from "./connect-tools-model";

describe("connect tools model", () => {
  test("defines the single catalog tools section", () => {
    expect(CONNECT_TOOLS_SECTION_TITLE).toBe("Catalog");
  });

  test("keeps all tools in the catalog section", () => {
    const tools = [{ id: "a" }, { id: "b" }] as const;

    expect(getConnectToolsSection(tools)).toEqual({
      title: "Catalog",
      tools,
    });
  });

  test("groups normalized doc comments into readable description blocks", () => {
    expect(
      getConnectToolDescriptionBlocks(
        [
          "Discover candidate catalog entities from search text.",
          "",
          "Use this when:",
          "- You need relevant tables or columns.",
          "- You are starting a catalog workflow.",
          "",
          "Inputs:",
          "- query: Natural-language search phrase. Required string.",
        ].join("\n")
      )
    ).toEqual([
      {
        kind: "paragraph",
        text: "Discover candidate catalog entities from search text.",
      },
      {
        kind: "heading",
        text: "Use this when:",
      },
      {
        items: ["You need relevant tables or columns.", "You are starting a catalog workflow."],
        kind: "list",
      },
      {
        kind: "heading",
        text: "Inputs:",
      },
      {
        items: ["query: Natural-language search phrase. Required string."],
        kind: "list",
      },
    ]);
  });
});
