import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { ApprovalCard } from "~/components/approvals/approval-card";

describe("ApprovalCard", () => {
  test("renders feedback and partial JSON fields for pending revise", () => {
    const html = renderToStaticMarkup(
      <ApprovalCard
        approval={{
          approvalId: "approval-1",
          threadId: "thread-1",
          runId: "run-1",
          status: "pending",
          payload: { title: "Review query" },
          createdAt: "2026-01-01T00:00:00Z",
          updatedAt: "2026-01-01T00:00:00Z",
        }}
        onResolved={() => {}}
      />
    );

    expect(html).toContain("Feedback");
    expect(html).toContain("Partial JSON");
    expect(html).toContain("Revise");
    expect(html).toContain("border-border/70 bg-background/80 px-2.5 py-1.5 text-xs");
    expect(html).toContain("hover:bg-muted hover:text-foreground");
    expect(html).not.toContain("bg-primary");
    expect(html).not.toContain("text-destructive");
    expect(html).not.toContain("bg-green-600");
    expect(html).not.toContain("bg-red-600");
    expect(html).not.toContain("text-[var(--dot-blue)]");
  });

  test("renders plan actions and rationale under review", () => {
    // Shape captured from a live `approval.required` event: the part payload is
    // the runtime's `raw` plan envelope, whose `.payload` carries the plan body
    // (`actions` as a {head, tail} ActionSeq, `rationale` nested under context).
    const html = renderToStaticMarkup(
      <ApprovalCard
        approval={{
          approvalId: "approval-1",
          status: "pending",
          payload: {
            kind: "plan",
            payload: {
              actions: {
                head: {
                  kind: "promotion_launch",
                  payload: { discount_pct: 15, product_ids: ["WIDGET-1"] },
                },
                tail: [
                  {
                    kind: "price_change",
                    payload: { new_price: 79.99, product_id: "WIDGET-1" },
                  },
                ],
              },
              context: { rationale: "Drive seasonal volume." },
            },
          },
        }}
        onResolved={() => {}}
      />
    );

    expect(html).toContain("promotion_launch");
    expect(html).toContain("price_change");
    expect(html).toContain("79.99");
    expect(html).toContain("Drive seasonal volume.");
  });
});
