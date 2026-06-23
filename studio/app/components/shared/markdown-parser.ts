import { marked, type Tokens } from "marked";

const MAX_CACHE_SIZE = 256;
const parseCache = new Map<string, string>();
const SAFE_URL_PROTOCOLS = new Set(["http:", "https:", "mailto:", "tel:"]);

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

function hasUnsafeUrlCharacter(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code <= 0x20 || code === 0x7f) {
      return true;
    }
  }
  return false;
}

function isSafeUrl(href: string): boolean {
  const value = href.trim();
  if (!value || hasUnsafeUrlCharacter(value)) {
    return false;
  }
  if (value.startsWith("#") || value.startsWith("?") || value.startsWith("./")) {
    return true;
  }
  if (value.startsWith("/") && !value.startsWith("//")) {
    return true;
  }
  if (value.startsWith("../")) {
    return true;
  }

  try {
    const parsed = new URL(value, "https://flowai.local");
    const hasExplicitScheme = /^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(value);
    if (!hasExplicitScheme && parsed.origin === "https://flowai.local") {
      return true;
    }
    return SAFE_URL_PROTOCOLS.has(parsed.protocol);
  } catch {
    return false;
  }
}

function optionalAttribute(name: string, value: string | null | undefined): string {
  return value ? ` ${name}="${escapeHtml(value)}"` : "";
}

const renderer = new marked.Renderer();
renderer.html = (token: unknown) => escapeHtml(htmlTokenText(token));
renderer.link = function ({ href, title, tokens }: Tokens.Link) {
  const text = this.parser.parseInline(tokens);
  if (!isSafeUrl(href)) {
    return `<a>${text}</a>`;
  }
  return `<a href="${escapeHtml(href)}"${optionalAttribute("title", title)}>${text}</a>`;
};
renderer.image = ({ href, title, text }: Tokens.Image) => {
  const alt = escapeHtml(text);
  if (!isSafeUrl(href)) {
    return `<span>${alt}</span>`;
  }
  return `<img src="${escapeHtml(href)}" alt="${alt}"${optionalAttribute("title", title)}>`;
};

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
