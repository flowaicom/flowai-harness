import { spawn } from "node:child_process";
import { copyFile, mkdir, readdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const studioRoot = path.resolve(__dirname, "..");

const packages = [
  "studio-core",
  "studio-ui",
  "studio-features-chat",
  "studio-features-connect",
  "studio-features-evals",
  "studio-features-tests",
];

const staticAssetExtensions = new Set([".css", ".otf", ".svg", ".woff", ".woff2"]);

const tscBinary = path.resolve(
  studioRoot,
  "node_modules",
  ".bin",
  process.platform === "win32" ? "tsc.cmd" : "tsc"
);

function run(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: studioRoot,
      stdio: "inherit",
      env: process.env,
    });

    child.on("error", reject);
    child.on("exit", (code) => {
      if (code === 0) {
        resolve();
        return;
      }
      reject(new Error(`Command failed with exit code ${code}: ${command} ${args.join(" ")}`));
    });
  });
}

async function removeStaleSourceArtifacts(root) {
  const entries = await readdir(root, { withFileTypes: true });
  for (const entry of entries) {
    const entryPath = path.resolve(root, entry.name);
    if (entry.isDirectory()) {
      await removeStaleSourceArtifacts(entryPath);
      continue;
    }

    if (entry.name.endsWith(".js") || entry.name.endsWith(".d.ts")) {
      await rm(entryPath, { force: true });
    }
  }
}

async function copyStaticAssets(srcRoot, distRoot, root = srcRoot) {
  const entries = await readdir(root, { withFileTypes: true });
  for (const entry of entries) {
    const entryPath = path.resolve(root, entry.name);
    if (entry.isDirectory()) {
      await copyStaticAssets(srcRoot, distRoot, entryPath);
      continue;
    }

    if (!staticAssetExtensions.has(path.extname(entry.name))) {
      continue;
    }

    const relativePath = path.relative(srcRoot, entryPath);
    const destination = path.resolve(distRoot, relativePath);
    await mkdir(path.dirname(destination), { recursive: true });
    await copyFile(entryPath, destination);
  }
}

function rewriteRelativeSpecifier(specifier) {
  if (!specifier.startsWith(".")) {
    return specifier;
  }

  const extension = path.posix.extname(specifier);
  if (extension.length > 0) {
    return specifier;
  }

  return `${specifier}.js`;
}

async function rewriteDistImports(root) {
  const entries = await readdir(root, { withFileTypes: true });
  for (const entry of entries) {
    const entryPath = path.resolve(root, entry.name);
    if (entry.isDirectory()) {
      await rewriteDistImports(entryPath);
      continue;
    }

    if (!entry.name.endsWith(".js") && !entry.name.endsWith(".d.ts")) {
      continue;
    }

    const source = await readFile(entryPath, "utf8");
    const rewritten = source
      .replace(/(from\s+["'])(\.{1,2}\/[^"']+)(["'])/g, (_match, prefix, specifier, suffix) => {
        return `${prefix}${rewriteRelativeSpecifier(specifier)}${suffix}`;
      })
      .replace(
        /(import\(\s*["'])(\.{1,2}\/[^"']+)(["']\s*\))/g,
        (_match, prefix, specifier, suffix) => {
          return `${prefix}${rewriteRelativeSpecifier(specifier)}${suffix}`;
        }
      );

    if (rewritten !== source) {
      await writeFile(entryPath, rewritten);
    }
  }
}

async function validateCssAssetUrls(root) {
  const entries = await readdir(root, { withFileTypes: true });
  for (const entry of entries) {
    const entryPath = path.resolve(root, entry.name);
    if (entry.isDirectory()) {
      await validateCssAssetUrls(entryPath);
      continue;
    }

    if (!entry.name.endsWith(".css")) {
      continue;
    }

    const source = await readFile(entryPath, "utf8");
    const urlPattern = /url\((["']?)([^"')]+)\1\)/g;
    for (const match of source.matchAll(urlPattern)) {
      const rawSpecifier = match[2].trim();
      if (
        rawSpecifier.startsWith("data:") ||
        rawSpecifier.startsWith("http:") ||
        rawSpecifier.startsWith("https:") ||
        rawSpecifier.startsWith("/") ||
        rawSpecifier.startsWith("#")
      ) {
        continue;
      }

      const specifier = rawSpecifier.split("#")[0].split("?")[0];
      const target = path.resolve(path.dirname(entryPath), specifier);
      try {
        await stat(target);
      } catch {
        throw new Error(
          `[build:packages] CSS asset does not exist: ${rawSpecifier} referenced from ${path.relative(
            studioRoot,
            entryPath
          )}`
        );
      }
    }
  }
}

for (const packageName of packages) {
  const packageRoot = path.resolve(studioRoot, "packages", packageName);
  const packageConfig = path.resolve(packageRoot, "tsconfig.build.json");
  const distRoot = path.resolve(packageRoot, "dist");
  const srcRoot = path.resolve(packageRoot, "src");

  console.log(`[build:packages] Building ${packageName}`);
  await rm(distRoot, { recursive: true, force: true });
  await removeStaleSourceArtifacts(srcRoot);
  await run(tscBinary, ["-p", packageConfig]);
  await copyStaticAssets(srcRoot, distRoot);
  await rewriteDistImports(distRoot);
  await validateCssAssetUrls(distRoot);
  await stat(path.resolve(distRoot, "index.js"));
  await stat(path.resolve(distRoot, "index.d.ts"));
}
