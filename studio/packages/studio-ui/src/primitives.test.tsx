import { describe, expect, test } from "bun:test";
import { CheckIcon } from "lucide-react";
import { renderToStaticMarkup } from "react-dom/server";
import { Badge, Button, EmptyState, JsonPane, SegmentedControl, StatusDot } from "./primitives";

describe("studio UI primitives", () => {
  test("render token-backed controls without app stores", () => {
    const html = renderToStaticMarkup(
      <div>
        <Button variant="primary">Save</Button>
        <Badge tone="orange">Late</Badge>
        <StatusDot tone="violet" label="Live" />
        <SegmentedControl
          label="Density"
          value="compact"
          options={[
            { value: "compact", label: "Compact" },
            { value: "roomy", label: "Roomy" },
          ]}
          onChange={() => {}}
        />
      </div>
    );

    expect(html).toContain("Save");
    expect(html).toContain("Late");
    expect(html).toContain('aria-label="Live"');
    expect(html).toContain("--accent-orange-bg");
    expect(html).toContain("--dot-purple");
    expect(html).toContain('aria-pressed="true"');
  });

  test("render JSON panes and empty states", () => {
    const html = renderToStaticMarkup(
      <div>
        <JsonPane ariaLabel="Payload" value={{ status: "ok", count: 2 }} />
        <EmptyState icon={CheckIcon} title="Nothing queued" description="All jobs are complete." />
      </div>
    );

    expect(html).toContain("Payload");
    expect(html).toContain("&quot;status&quot;: &quot;ok&quot;");
    expect(html).toContain("Nothing queued");
  });
});
