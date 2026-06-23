import { marked } from "marked";

const MAX_CACHE_SIZE = 256;
const parseCache = new Map<string, string>();

export function escapeHtml(text: string): string {
  return text
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function htmlTokenText(token: unknown): string {
  if (typeof token === "string") return token;
  if (token && typeof token === "object") {
    const candidate = token as { text?: unknown; raw?: unknown };
    if (typeof candidate.text === "string") return candidate.text;
    if (typeof candidate.raw === "string") return candidate.raw;
  }
  return "";
}

const renderer = new marked.Renderer();
renderer.html = (token: unknown) => escapeHtml(htmlTokenText(token));

marked.setOptions({
  gfm: true,
  breaks: true,
});
marked.use({ renderer });

export function parseMarkdown(text: string): string {
  const cached = parseCache.get(text);
  if (cached !== undefined) return cached;

  const html = marked.parse(text, { async: false }) as string;

  if (parseCache.size >= MAX_CACHE_SIZE) {
    const firstKey = parseCache.keys().next().value;
    if (firstKey !== undefined) parseCache.delete(firstKey);
  }
  parseCache.set(text, html);
  return html;
}
