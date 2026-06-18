import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import { isOk } from "~/lib/domain/result";
import type { AuthoredTestCase } from "~/lib/domain/test-case";
import { setApiConfig, setWorkspaceHeader } from "./client";
import {
  createTestCase,
  deleteTestCase,
  getTestCase,
  listTestCases,
  updateTestCase,
  validateTestCase,
} from "./tests";

const originalFetch = globalThis.fetch;

function resetApiConfig() {
  setApiConfig({
    baseUrl: "/api",
    timeout: 30_000,
    headers: { "Content-Type": "application/json" },
  });
  setWorkspaceHeader("ws 1");
}

function okJson(body: unknown) {
  return Promise.resolve(
    new Response(JSON.stringify(body), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    })
  );
}

const harnessRow = {
  id: "tc-1",
  testCase: {
    id: "tc-1",
    input: "Find the top product",
    tags: ["demo"],
    expectedTrajectory: ["call_agent"],
    trajectoryMode: "unordered",
    structuredGroundTruth: { expected: "Sparkling Water 12pk" },
    sourceThreadId: null,
  },
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
};

const authoredCase: Omit<AuthoredTestCase, "id" | "createdAt" | "updatedAt"> = {
  name: "tc-new",
  description: null,
  input: "Find the top product",
  status: "draft",
  expectedTrajectory: ["call_agent"],
  trajectoryMode: "unordered",
  groundTruth: null,
  structuredGroundTruth: { kind: "structured", payload: { expected: "Sparkling Water 12pk" } },
  tags: ["demo"],
  trajectoryProvenance: [],
  trajectorySources: [],
  sourceThreadId: null,
  sourceSessionId: null,
};

beforeEach(resetApiConfig);

afterEach(() => {
  globalThis.fetch = originalFetch;
});

describe("tests harness adapter", () => {
  test("uses workspace-scoped harness CRUD paths", async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    globalThis.fetch = mock((url: string | URL | Request, init?: RequestInit) => {
      calls.push({ url: String(url), init: init ?? {} });
      const method = init?.method ?? "GET";
      if (method === "GET" && String(url).endsWith("/tests")) {
        return okJson({ tests: [harnessRow] });
      }
      if (method === "GET" && String(url).endsWith("/tests/tc-1"))
        return okJson({ test: harnessRow });
      if (method === "POST") return okJson({ test: harnessRow });
      if (method === "PUT") return okJson({ test: harnessRow });
      if (method === "DELETE") return Promise.resolve(new Response("", { status: 204 }));
      return Promise.resolve(new Response("not found", { status: 404 }));
    }) as unknown as typeof fetch;

    const listed = await listTestCases();
    const created = await createTestCase(authoredCase);
    const fetched = await getTestCase("tc-1");
    const updated = await updateTestCase("tc-1", { input: "Updated prompt" });
    const deleted = await deleteTestCase("tc-1");

    expect(isOk(listed)).toBe(true);
    expect(isOk(created)).toBe(true);
    expect(isOk(fetched)).toBe(true);
    expect(isOk(updated)).toBe(true);
    expect(isOk(deleted)).toBe(true);
    expect(calls.map((call) => `${call.init.method ?? "GET"} ${call.url}`)).toEqual([
      "GET /api/workspaces/ws%201/tests",
      "POST /api/workspaces/ws%201/tests",
      "GET /api/workspaces/ws%201/tests/tc-1",
      "GET /api/workspaces/ws%201/tests/tc-1",
      "PUT /api/workspaces/ws%201/tests/tc-1",
      "DELETE /api/workspaces/ws%201/tests/tc-1",
    ]);

    expect(JSON.parse(String(calls[1].init.body))).toMatchObject({
      id: "tc-new",
      input: "Find the top product",
      expectedTrajectory: ["call_agent"],
      structuredGroundTruth: {
        kind: "structured",
        payload: { expected: "Sparkling Water 12pk" },
      },
    });
    expect(JSON.parse(String(calls[4].init.body))).toMatchObject({
      id: "tc-1",
      input: "Updated prompt",
      expectedTrajectory: ["call_agent"],
    });
  });

  test("validateTestCase performs a harness read and validates locally", async () => {
    const calls: string[] = [];
    globalThis.fetch = mock((url: string | URL | Request, init?: RequestInit) => {
      calls.push(`${init?.method ?? "GET"} ${String(url)}`);
      return okJson({ test: harnessRow });
    }) as unknown as typeof fetch;

    const result = await validateTestCase("tc-1");

    expect(isOk(result)).toBe(true);
    if (isOk(result)) {
      expect(result.value).toEqual({ valid: true, issues: [] });
    }
    expect(calls).toEqual(["GET /api/workspaces/ws%201/tests/tc-1"]);
  });
});
