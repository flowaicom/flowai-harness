import { describe, expect, test } from "bun:test";
import { parseApprovalPartialJson } from "./approval-response";

describe("parseApprovalPartialJson", () => {
  test("omits partial for an empty field", () => {
    expect(parseApprovalPartialJson("   ")).toEqual({ ok: true });
  });

  test("rejects invalid JSON", () => {
    const parsed = parseApprovalPartialJson("{bad");
    expect(parsed.ok).toBe(false);
  });

  test("rejects non-object JSON", () => {
    expect(parseApprovalPartialJson("[1, 2]")).toEqual({
      ok: false,
      error: "Partial JSON must be a JSON object.",
    });
  });

  test("sends parsed object partial for valid JSON", () => {
    expect(parseApprovalPartialJson('{"query":"updated"}')).toEqual({
      ok: true,
      partial: { query: "updated" },
    });
  });
});
