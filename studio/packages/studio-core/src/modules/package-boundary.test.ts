import { describe, expect, test } from "bun:test";
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";

const featurePackageDirs = [
  "packages/studio-features-chat/src",
  "packages/studio-features-connect/src",
  "packages/studio-features-evals/src",
  "packages/studio-features-tests/src",
] as const;

const forbiddenHostImportPatterns = [
  /from\s+["']~\//,
  /import\s*\(\s*["']~\//,
  /from\s+["']\.\.\/\.\.\/app\//,
  /import\s*\(\s*["']\.\.\/\.\.\/app\//,
] as const;

async function collectSourceFiles(dir: string): Promise<string[]> {
  const entries = await readdir(dir, { withFileTypes: true });
  const files = await Promise.all(
    entries.map(async (entry) => {
      const entryPath = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        return collectSourceFiles(entryPath);
      }
      if (/\.(ts|tsx)$/.test(entry.name)) {
        return [entryPath];
      }
      return [];
    })
  );
  return files.flat();
}

describe("Studio feature package boundaries", () => {
  test("feature packages do not import app host modules", async () => {
    const sourceFiles = (
      await Promise.all(featurePackageDirs.map((dir) => collectSourceFiles(dir)))
    ).flat();

    const violations: string[] = [];
    await Promise.all(
      sourceFiles.map(async (sourceFile) => {
        const source = await readFile(sourceFile, "utf8");
        if (forbiddenHostImportPatterns.some((pattern) => pattern.test(source))) {
          violations.push(sourceFile);
        }
      })
    );

    expect(violations.sort()).toEqual([]);
  });
});
