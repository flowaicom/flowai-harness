/**
 * Performance-optimized markdown renderer.
 *
 * Uses `marked` for parsing (fastest JS markdown parser) with a global
 * LRU-style cache to avoid re-parsing identical strings across renders.
 * Raw HTML blocks are escaped before rendering.
 *
 * Supports PII scrambling (Demo Mode): when enabled, text content is
 * scrambled before markdown parsing. This preserves markdown structure
 * (headers, lists, bold, code blocks) because structure characters
 * (#, *, -, `) are punctuation and pass through unchanged.
 *
 * Code blocks (`<pre>`) get a hover copy-to-clipboard button via
 * post-render DOM injection (ref callback + MutationObserver-free).
 *
 * @module components/shared/markdown
 */

import { memo, useCallback, useMemo } from "react";
import { useScramble } from "~/lib/scramble";
import { cn } from "~/lib/utils";
import { parseMarkdown } from "./markdown-parser";

/** SVG icon strings — avoids bundling lucide for DOM-injected buttons. */
const COPY_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>`;
const CHECK_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>`;

/**
 * Inject copy buttons on all `<pre>` elements inside a container.
 * Uses a ref callback pattern — runs once on mount, cleans up on unmount.
 */
function injectCopyButtons(container: HTMLDivElement) {
  const preElements = container.querySelectorAll("pre");
  if (preElements.length === 0) return;

  for (const pre of preElements) {
    // Skip if already injected
    if (pre.querySelector(".code-copy-btn")) continue;

    // Make pre relative for absolute positioning of button
    pre.style.position = "relative";

    const btn = document.createElement("button");
    btn.className = "code-copy-btn";
    btn.type = "button";
    btn.title = "Copy to clipboard";
    btn.innerHTML = COPY_SVG;
    btn.setAttribute("aria-label", "Copy code to clipboard");

    btn.addEventListener("click", () => {
      const code = pre.querySelector("code")?.textContent ?? pre.textContent ?? "";
      navigator.clipboard.writeText(code).then(() => {
        btn.innerHTML = CHECK_SVG;
        btn.classList.add("copied");
        setTimeout(() => {
          btn.innerHTML = COPY_SVG;
          btn.classList.remove("copied");
        }, 1500);
      });
    });

    pre.appendChild(btn);
  }
}

interface MarkdownProps {
  readonly text: string;
  readonly className?: string;
}

export const Markdown = memo(function Markdown({ text, className }: MarkdownProps) {
  const { s } = useScramble();
  const html = useMemo(() => parseMarkdown(s(text)), [text, s]);

  const refCallback = useCallback(
    (node: HTMLDivElement | null) => {
      if (node) injectCopyButtons(node);
    },
    // Re-run injection when html changes (new code blocks may appear).
    []
  );

  return (
    <div
      ref={refCallback}
      className={cn("markdown-content", className)}
      // biome-ignore lint/security/noDangerouslySetInnerHtml: Marked output is constrained by a renderer that escapes raw HTML tokens.
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
});
