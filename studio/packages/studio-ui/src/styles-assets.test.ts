import { describe, expect, test } from "bun:test";
import { readdir, readFile, stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const srcRoot = path.dirname(fileURLToPath(import.meta.url));

describe("studio UI styles", () => {
  test("font URLs resolve relative to source CSS", async () => {
    const cssPath = path.resolve(srcRoot, "styles", "flow-fonts.css");
    const css = await readFile(cssPath, "utf8");
    const urls = [...css.matchAll(/url\((["']?)([^"')]+)\1\)/g)].map((match) => match[2]);

    expect(urls.length).toBeGreaterThan(0);
    for (const url of urls) {
      await stat(path.resolve(path.dirname(cssPath), url));
    }
  });

  test("all bundled font assets are present", async () => {
    const files = await readdir(path.resolve(srcRoot, "assets", "fonts"));
    expect(files).toContain("EuclidCircularB-Regular.woff2");
    expect(files).toContain("DMMono-Regular.woff");
  });
});
