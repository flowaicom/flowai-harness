import { describe, expect, test } from "bun:test";
import {
  buildConnectSearchAskPrompt,
  CONNECT_SEARCH_TABS,
  getConnectSearchItemCategory,
  parseConnectToolSearchRows,
  resolveConnectSearchRequest,
} from "./connect-search-model";

describe("connect search model", () => {
  test("exposes the supported search tabs", () => {
    expect(CONNECT_SEARCH_TABS.map((tab) => tab.id)).toEqual([
      "unified",
      "semantic",
      "fuzzyTable",
      "fuzzyColumn",
      "resolveTerm",
    ]);
  });

  test("maps known item types to semantic categories", () => {
    expect(getConnectSearchItemCategory("table")).toBe("discovery");
    expect(getConnectSearchItemCategory("metric")).toBe("execution");
    expect(getConnectSearchItemCategory("unknown")).toBe("knowledge");
  });

  test("builds an ask prompt from search result metadata", () => {
    expect(
      buildConnectSearchAskPrompt({
        itemType: "table",
        name: "orders",
        description: "Contains order facts",
      })
    ).toBe('Tell me about table "orders". Contains order facts');
  });

  test("resolves tool-backed search modes into tool requests", () => {
    expect(resolveConnectSearchRequest("fuzzyTable", "orders")).toEqual({
      kind: "tool",
      toolId: "search_catalog",
      input: { query: "orders", kinds: ["table"] },
    });
    expect(resolveConnectSearchRequest("resolveTerm", "gross margin")).toEqual({
      kind: "tool",
      toolId: "search_catalog",
      input: { query: "gross margin" },
    });
  });

  test("parses tool rows from a merged tool payload", () => {
    expect(
      parseConnectToolSearchRows(
        JSON.stringify({
          tables: [{ tableName: "orders", type: "table", description: "fact table", score: 0.9 }],
        })
      )
    ).toEqual([
      {
        name: "orders",
        itemType: "table",
        description: "fact table",
        score: 0.9,
      },
    ]);
  });
});
