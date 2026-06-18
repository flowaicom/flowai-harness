import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router";
import { DEFAULT_EVAL_CONFIG } from "~/lib/domain/eval";
import { EvalConfigForm } from "./eval-config-form";

describe("EvalConfigForm", () => {
  test("renders only capability-provided eval mode choices", () => {
    const html = renderToStaticMarkup(
      <MemoryRouter>
        <EvalConfigForm
          config={{ ...DEFAULT_EVAL_CONFIG, mode: "specialist", targetAgentId: "insights" }}
          evalModes={[
            {
              mode: "specialist",
              label: "Insights",
              description: "Evaluate the insights specialist directly.",
              agentId: "insights",
              role: "specialist",
              targetAgentId: "insights",
            },
          ]}
          testCaseSets={[]}
          testCases={[]}
          onUpdate={() => {}}
          onSubmit={() => {}}
          isRunning={false}
        />
      </MemoryRouter>
    );

    expect(html).toContain("Insights");
    expect(html).not.toContain("Planner");
    expect(html).not.toContain("Executor");
    expect(html).not.toContain("Sequential");
    expect(html).not.toContain("Provider Override");
    expect(html).not.toContain("Model Override");
  });

  test("wraps eval mode choices in a horizontal scroll container", () => {
    const html = renderToStaticMarkup(
      <MemoryRouter>
        <EvalConfigForm
          config={{ ...DEFAULT_EVAL_CONFIG, mode: "specialist", targetAgentId: "agent-0" }}
          evalModes={Array.from({ length: 8 }, (_, i) => ({
            mode: "specialist",
            label: `Specialist ${i}`,
            description: `Evaluate specialist ${i} directly.`,
            agentId: `agent-${i}`,
            role: "specialist",
            targetAgentId: `agent-${i}`,
          }))}
          testCaseSets={[]}
          testCases={[]}
          onUpdate={() => {}}
          onSubmit={() => {}}
          isRunning={false}
        />
      </MemoryRouter>
    );

    expect(html).toContain('data-testid="eval-mode-scroll"');
    expect(html).toContain("overflow-x-auto");
    expect(html).toContain("overflow-y-hidden");
    expect(html).toContain("min-w-full");
  });

  test("does not expose uploaded test-case sets in local harness Studio", () => {
    const html = renderToStaticMarkup(
      <MemoryRouter>
        <EvalConfigForm
          config={DEFAULT_EVAL_CONFIG}
          testCaseSets={[
            {
              id: "set-1",
              name: "Uploaded set",
              description: "",
              testCases: [],
              createdAt: "2026-01-01T00:00:00.000Z",
            },
          ]}
          testCases={[]}
          onUpdate={() => {}}
          onSubmit={() => {}}
          isRunning={false}
        />
      </MemoryRouter>
    );

    expect(html).not.toContain("Uploaded Set");
    expect(html).not.toContain("Select a test case set");
  });
});
