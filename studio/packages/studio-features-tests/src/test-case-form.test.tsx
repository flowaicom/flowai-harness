import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { SharedTestCaseForm } from "./test-case-form";

describe("SharedTestCaseForm", () => {
  test("renders trajectory modes and tool catalog chips without app aliases", () => {
    const html = renderToStaticMarkup(
      <SharedTestCaseForm
        identity={{
          name: "Revenue test",
          onNameChange: () => {},
          description: "Checks revenue analysis",
          onDescriptionChange: () => {},
        }}
        input="Show revenue by customer"
        onInputChange={() => {}}
        trajectory="lookup_customer"
        onTrajectoryChange={() => {}}
        mode="strict"
        onModeChange={() => {}}
        tags="revenue"
        onTagsChange={() => {}}
        status="draft"
        onStatusChange={() => {}}
        availableTools={[
          {
            name: "lookup_customer",
            description: "Find a customer",
            category: "discovery",
          },
          {
            name: "summarize_revenue",
            description: "Summarize revenue",
            category: "execution",
          },
        ]}
        toolCatalogInitiallyExpanded={true}
        groundTruthEditor={<textarea aria-label="Ground truth" defaultValue="ok" />}
      />
    );

    expect(html).toContain("Strict sequence");
    expect(html).toContain("Unordered exact");
    expect(html).toContain("Actual subset");
    expect(html).toContain("Required present");
    expect(html).toContain("Required in order");
    expect(html).toContain("lookup_customer");
    expect(html).toContain("summarize_revenue");
  });
});
