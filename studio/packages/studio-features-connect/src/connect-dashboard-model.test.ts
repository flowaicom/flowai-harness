import { describe, expect, test } from "bun:test";
import {
  buildConnectExplorePrompt,
  summarizeConnectAvailableContext,
} from "./connect-dashboard-model";

describe("connect dashboard model", () => {
  test("summarizes table, knowledge, and document context", () => {
    expect(
      summarizeConnectAvailableContext({
        tableCount: 2,
        tableNames: ["orders", "customers"],
        knowledgeCount: 3,
        documentCount: 1,
      })
    ).toEqual(["2 tables (orders, customers)", "3 knowledge items", "1 document"]);
  });

  test("builds the explore prompt with context", () => {
    expect(
      buildConnectExplorePrompt({
        tableCount: 1,
        tableNames: ["orders"],
        knowledgeCount: 0,
        documentCount: 0,
      })
    ).toContain("I have 1 table (orders) available.");
  });
});
