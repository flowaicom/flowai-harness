import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { SharedTestCaseDetailPage } from "./test-case-detail-page";

describe("SharedTestCaseDetailPage", () => {
  test("does not render a draft status pill in the detail header", () => {
    const html = renderToStaticMarkup(
      <SharedTestCaseDetailPage
        loaded={true}
        testCaseId="tc-executor-price-change"
        testCase={{
          createdAt: "2026-06-15T00:00:00Z",
          updatedAt: "2026-06-17T00:00:00Z",
          tags: ["eval-role:executor", "example"],
        }}
        status="draft"
        isDirty={false}
        saving={false}
        deleting={false}
        trajectoryStepCount={2}
        validationIssues={[]}
        traceExpanded={false}
        trace={null}
        evalHistory={[]}
        activeTestCaseCount={1}
        transitions={[]}
        form={<div>Test case form</div>}
        onDismissError={() => {}}
        onToggleTraceExpanded={() => {}}
        onSave={() => {}}
        onTryInChat={() => {}}
        onRunEval={() => {}}
        onRefineInBuilder={() => {}}
        onClone={() => {}}
        onStatusChange={() => {}}
        onDelete={() => {}}
        onOpenSourceThread={() => {}}
        onOpenEvalRun={() => {}}
        onOpenMoreEvalRuns={() => {}}
      />
    );

    expect(html).not.toContain(">draft<");
    expect(html).not.toContain(">DRAFT<");
    expect(html).toContain("tc-executor-price-change");
    expect(html).toContain("2 trajectory steps");
  });
});
