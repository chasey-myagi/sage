// Generates docs/log-manifest.md from the entry kind registry.
// Not model-visible and not a Log entry: this only reads built output and
// writes a markdown table. Run `tsc -b` before this script; it imports
// packages/core/dist, whose workspace imports resolve to the other
// packages' dist through their default export condition.
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repoRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const defaultManifest = path.join(repoRoot, "docs", "log-manifest.md");
const distEntry = path.join(repoRoot, "packages", "core", "dist", "index.js");

/**
 * @param {readonly { kind: string, doc: string }[]} kinds
 */
export function renderLogManifest(kinds) {
  const rows = [...kinds].sort((a, b) => a.kind.localeCompare(b.kind));
  const lines = [
    "# Log entry 清单",
    "",
    "由 `scripts/gen-log-manifest.mjs` 根据 `@sage/core` 的 `entryKinds()` 生成。手改会被 `pnpm check:log-manifest` 拒绝。",
    "",
    "| kind | doc |",
    "| --- | --- |",
  ];
  for (const kind of rows) {
    lines.push(`| ${kind.kind} | ${escapeCell(kind.doc)} |`);
  }
  lines.push("");
  return `${lines.join("\n")}`;
}

/**
 * @param {string} text
 */
function escapeCell(text) {
  return text.replaceAll("|", "\\|").replaceAll("\n", " ");
}

export async function loadEntryKinds() {
  const mod = await import(pathToFileURL(distEntry).href);
  if (typeof mod.entryKinds !== "function") {
    throw new Error("entryKinds() is missing from packages/core/dist");
  }
  return mod.entryKinds();
}

/**
 * @param {readonly string[]} argv
 */
export async function run(argv) {
  const check = argv.includes("--check");
  const fileFlag = argv.indexOf("--file");
  const manifestPath =
    fileFlag >= 0 && argv[fileFlag + 1]
      ? path.resolve(argv[fileFlag + 1])
      : defaultManifest;
  let kinds;
  try {
    kinds = await loadEntryKinds();
  } catch (error) {
    process.stderr.write(`failed to load @sage/core dist: ${error}\n`);
    return 1;
  }
  const next = renderLogManifest(kinds);
  if (check) {
    const current = existsSync(manifestPath)
      ? readFileSync(manifestPath, "utf8")
      : "";
    if (current !== next) {
      process.stderr.write(`log manifest is stale: ${manifestPath}\n`);
      return 1;
    }
    return 0;
  }
  writeFileSync(manifestPath, next);
  return 0;
}

async function main() {
  const code = await run(process.argv.slice(2));
  process.exit(code);
}

function invokedAsCli() {
  const entry = process.argv[1];
  if (!entry) return false;
  return import.meta.url === pathToFileURL(path.resolve(entry)).href;
}

if (invokedAsCli()) {
  main();
}
