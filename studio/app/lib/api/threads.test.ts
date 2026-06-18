import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import { isOk } from "~/lib/domain/result";
import { setApiConfig, setWorkspaceHeader } from "./client";
import { deleteThread } from "./threads";

const originalFetch = globalThis.fetch;

function resetApiConfig() {
  setApiConfig({
    baseUrl: "/api",
    timeout: 30_000,
    headers: { "Content-Type": "application/json" },
  });
  setWorkspaceHeader("customer workspace");
}

beforeEach(resetApiConfig);

afterEach(() => {
  globalThis.fetch = originalFetch;
});

describe("thread API", () => {
  test("deletes threads through workspace-scoped harness paths", async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    globalThis.fetch = mock((url: string | URL | Request, init?: RequestInit) => {
      calls.push({ url: String(url), init: init ?? {} });
      return Promise.resolve(new Response("", { status: 204 }));
    }) as unknown as typeof fetch;

    const result = await deleteThread("thread-1");

    expect(isOk(result)).toBe(true);
    expect(calls.map((call) => `${call.init.method ?? "GET"} ${call.url}`)).toEqual([
      "DELETE /api/workspaces/customer%20workspace/threads/thread-1",
    ]);
  });
});
