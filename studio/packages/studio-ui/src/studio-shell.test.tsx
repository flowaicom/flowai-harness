import { describe, expect, test } from "bun:test";
import { DatabaseIcon, MessageSquareIcon } from "lucide-react";
import { renderToStaticMarkup } from "react-dom/server";
import { StudioShell } from "./studio-shell";

describe("studio shell", () => {
  test("renders injected nav and controls without importing runtime state", () => {
    const html = renderToStaticMarkup(
      <StudioShell
        appName="Acme pricing"
        workspaceControl={<div>Local workspace</div>}
        runtimeStatus={<span>Ready</span>}
        navItems={[
          {
            id: "playground",
            label: "Playground",
            href: "/playground",
            icon: MessageSquareIcon,
            active: true,
          },
          {
            id: "connect",
            label: "Connect",
            href: "/connect",
            icon: DatabaseIcon,
            disabledReason: "data.profile is disabled",
          },
        ]}
      >
        <section>Body</section>
      </StudioShell>
    );

    expect(html).toContain("Acme pricing");
    expect(html).toContain("Local workspace");
    expect(html).toContain("Playground");
    expect(html).toContain("Connect");
    expect(html).toContain("data.profile is disabled");
    expect(html).toContain("Ready");
    expect(html).toContain("Body");
  });
});
