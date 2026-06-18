import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { PageShell, StudioHeader, SurfaceHeader } from "./page-shell";

describe("studio page shell", () => {
  test("renders page frame, header, and injected links", () => {
    const html = renderToStaticMarkup(
      <PageShell>
        <SurfaceHeader
          crumbs={[{ label: "Workspace", href: "/workspace" }, { label: "Playground" }]}
          renderLink={({ href, className, children }) => (
            <a href={href} className={className} data-test-link="true">
              {children}
            </a>
          )}
        />
        <StudioHeader
          eyebrow="Workspace"
          title="Default workspace"
          description="Live workspace overview."
        />
      </PageShell>
    );

    expect(html).toContain('data-test-link="true"');
    expect(html).toContain("Workspace");
    expect(html).toContain("Playground");
    expect(html).toContain("Default workspace");
  });
});
