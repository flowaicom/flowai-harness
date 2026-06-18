import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import { DEFAULT_EVAL_CONFIG } from "~/lib/domain/eval";
import { isOk } from "~/lib/domain/result";
import { setApiConfig, setWorkspaceHeader } from "./client";
import { cancelEval, compareEvalRuns, createEvalCreateBody, startEvalStream } from "./evals";

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

function okSse(events: readonly unknown[]) {
  const encoder = new TextEncoder();
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const event of events) {
        controller.enqueue(encoder.encode(`data: ${JSON.stringify(event)}\n\n`));
      }
      controller.close();
    },
  });
  return Promise.resolve(
    new Response(stream, {
      status: 200,
      headers: { "Content-Type": "text/event-stream" },
    })
  );
}

beforeEach(resetApiConfig);

afterEach(() => {
  globalThis.fetch = originalFetch;
});

describe("createEvalCreateBody", () => {
  test("preserves specialist mode and target agent in the harness create body", () => {
    const body = createEvalCreateBody({
      ...DEFAULT_EVAL_CONFIG,
      mode: "specialist",
      targetAgentId: "insights",
      testCaseIds: ["tc-1"],
    });

    expect(body.testCaseIds).toEqual(["tc-1"]);
    expect(body.config).toMatchObject({
      mode: "specialist",
      targetAgentId: "insights",
    });
  });

  test("does not coerce unknown modes to sequential", () => {
    const body = createEvalCreateBody({
      ...DEFAULT_EVAL_CONFIG,
      mode: "custom-specialist",
      targetAgentId: null,
    });

    expect(body.config).toMatchObject({
      mode: "custom-specialist",
      targetAgentId: null,
    });
  });

  test("omits score weights when using harness defaults", () => {
    const body = createEvalCreateBody({
      ...DEFAULT_EVAL_CONFIG,
      testCaseIds: ["tc-1"],
      scoreWeights: null,
    });

    expect(body.config).not.toHaveProperty("scoreWeights");
  });

  test("passes canonical score weights through the harness config", () => {
    const body = createEvalCreateBody({
      ...DEFAULT_EVAL_CONFIG,
      testCaseIds: ["tc-1"],
      scoreWeights: { trajectory: 1, executed_actions: 2, final_response: 0.5 },
    });

    expect(body.testCaseIds).toEqual(["tc-1"]);
    expect(body.config).toMatchObject({
      scoreWeights: { trajectory: 1, executed_actions: 2, final_response: 0.5 },
    });
  });

  test("uses workspace-scoped harness paths for run, stream, cancel, and compare", async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    const terminalEvent = {
      runId: "eval-1",
      sequence: 1,
      type: "evalCompleted",
      data: {
        artifact: {
          runId: "eval-1",
          summary: {
            totalTestCases: 0,
            passed: 0,
            failed: 0,
            skipped: 0,
            aggregateScore: 0,
            totalDurationMs: 0,
            totalUsage: {},
          },
          testCases: [],
        },
      },
    };
    globalThis.fetch = mock((url: string | URL | Request, init?: RequestInit) => {
      calls.push({ url: String(url), init: init ?? {} });
      const urlString = String(url);
      const method = init?.method ?? "GET";
      if (method === "POST" && urlString.endsWith("/evals")) {
        return okJson({ eval: { id: "eval-1" } });
      }
      if (method === "GET" && urlString.endsWith("/evals/eval-1/stream")) {
        return okSse([terminalEvent]);
      }
      if (method === "POST" && urlString.endsWith("/evals/eval-1/cancel")) {
        return okJson({ status: "cancelled" });
      }
      if (method === "GET" && urlString.includes("/evals/compare?")) {
        return okJson({
          comparison: {
            left: { runId: "left", summary: null },
            right: { runId: "right", summary: null },
            testCases: [{ testCaseId: "tc-1", left: 0.5, right: 1, delta: 0.5 }],
          },
        });
      }
      return Promise.resolve(new Response("not found", { status: 404 }));
    }) as unknown as typeof fetch;

    const events: string[] = [];
    const started = await startEvalStream(
      { ...DEFAULT_EVAL_CONFIG, testCaseIds: ["tc-1"] },
      {
        onEvent: (event) => events.push(event.type),
        onComplete: () => events.push("complete"),
        onError: (error) => events.push(error.message),
      }
    );
    const cancelled = await cancelEval("eval-1");
    const compared = await compareEvalRuns("left", "right");
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(isOk(started)).toBe(true);
    expect(isOk(cancelled)).toBe(true);
    expect(isOk(compared)).toBe(true);
    expect(calls.map((call) => `${call.init.method ?? "GET"} ${call.url}`)).toEqual([
      "POST /api/workspaces/ws%201/evals",
      "GET /api/workspaces/ws%201/evals/eval-1/stream",
      "POST /api/workspaces/ws%201/evals/eval-1/cancel",
      "GET /api/workspaces/ws%201/evals/compare?left=left&right=right",
    ]);
    expect(JSON.parse(String(calls[0].init.body))).toMatchObject({
      testCaseIds: ["tc-1"],
      config: { mode: "sequential" },
    });
    expect(events).toContain("started");
    expect(events).toContain("completed");
    expect(events).toContain("complete");
  });
});
