import { describe, expect, test } from "bun:test";
import { parseMarkdown } from "./markdown-parser";

describe("parseMarkdown", () => {
  test("escapes raw HTML while preserving normal Markdown", () => {
    const html = parseMarkdown(
      "# Title\n\n<script>alert('xss')</script>\n\n<img src=x onerror=alert(1)>\n\n**bold**"
    );

    expect(html).toContain("<h1>Title</h1>");
    expect(html).toContain("<strong>bold</strong>");
    expect(html).not.toContain("<script>");
    expect(html).not.toContain("<img");
    expect(html).toContain("&lt;script&gt;");
    expect(html).toContain("&lt;img src=x onerror=alert(1)&gt;");
  });
});
