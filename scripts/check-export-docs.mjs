// Export JSDoc gate: every exported declaration in package and app sources
// carries a `/** ... */` block directly above it.
// Not model-visible and not a Log entry: this only reads source text.
import { readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

/** Source roots scanned, relative to the repository root. */
export const SOURCE_GLOBS = ["packages/*/src", "apps/*/src"];

const DECLARATION =
  /^export\s+(?:declare\s+)?(?:default\s+)?(?:abstract\s+)?(?:async\s+)?(function\*?|const|let|class|interface|type)\s+([A-Za-z_$][\w$]*)/;

/**
 * Scans every `.ts` file under the source roots of `rootDir`.
 *
 * Re-exports (`export {`, `export *`, `export type {`) are skipped: their
 * documentation lives at the declaration. Consecutive overload signatures of
 * one function need a single block above the first signature.
 *
 * @returns Violations sorted by file then line; empty when every exported
 *   declaration is documented.
 */
export function run(rootDir) {
  const root = path.resolve(rootDir);
  /** @type {{ file: string, line: number, name: string }[]} */
  const violations = [];
  for (const file of sourceFiles(root)) {
    const rel = toPosix(path.relative(root, file));
    for (const v of scanText(readFileSync(file, "utf8"))) {
      violations.push({ file: rel, ...v });
    }
  }
  violations.sort((a, b) =>
    a.file === b.file ? a.line - b.line : a.file.localeCompare(b.file),
  );
  return { violations };
}

/**
 * Finds exported declarations in one file's text that lack a JSDoc block.
 *
 * A declaration is documented when the nearest preceding non-blank line
 * closes a block comment that opened with `/**`. Decorator-free TypeScript
 * as formatted by biome is assumed.
 *
 * @returns One entry per undocumented declaration, with its 1-based line.
 */
export function scanText(text) {
  const lines = text.split("\n");
  /** @type {{ line: number, name: string }[]} */
  const missing = [];
  let previousName = "";
  for (let index = 0; index < lines.length; index += 1) {
    const match = DECLARATION.exec(lines[index] ?? "");
    if (!match) continue;
    const name = match[2] ?? "";
    const isOverload =
      match[1]?.startsWith("function") && name === previousName;
    previousName = match[1]?.startsWith("function") ? name : "";
    if (isOverload) continue;
    if (!hasDocAbove(lines, index)) missing.push({ line: index + 1, name });
  }
  return missing;
}

function hasDocAbove(lines, index) {
  let cursor = index - 1;
  while (cursor >= 0 && (lines[cursor] ?? "").trim() === "") cursor -= 1;
  if (cursor < 0 || !(lines[cursor] ?? "").trim().endsWith("*/")) return false;
  while (cursor >= 0) {
    const trimmed = (lines[cursor] ?? "").trim();
    if (trimmed.startsWith("/**")) return true;
    if (trimmed.startsWith("/*")) return false;
    cursor -= 1;
  }
  return false;
}

function sourceFiles(root) {
  /** @type {string[]} */
  const files = [];
  for (const pattern of SOURCE_GLOBS) {
    const [base, , leaf] = pattern.split("/");
    const baseDir = path.join(root, base ?? "");
    if (!isDirectory(baseDir)) continue;
    for (const entry of readdirSync(baseDir)) {
      const dir = path.join(baseDir, entry, leaf ?? "");
      if (isDirectory(dir)) walk(dir, files);
    }
  }
  return files;
}

function walk(dir, files) {
  for (const entry of readdirSync(dir)) {
    const full = path.join(dir, entry);
    if (isDirectory(full)) walk(full, files);
    else if (entry.endsWith(".ts") && !entry.endsWith(".d.ts"))
      files.push(full);
  }
}

function isDirectory(target) {
  try {
    return statSync(target).isDirectory();
  } catch {
    // A missing root is not an error: a checkout may lack apps/ or packages/.
    return false;
  }
}

function toPosix(value) {
  return value.split(path.sep).join("/");
}

function invokedAsCli() {
  const entry = process.argv[1];
  if (!entry) return false;
  return import.meta.url === pathToFileURL(path.resolve(entry)).href;
}

if (invokedAsCli()) {
  const root = process.argv[2] ? path.resolve(process.argv[2]) : process.cwd();
  const { violations } = run(root);
  for (const v of violations) {
    process.stdout.write(`${v.file}:${v.line} 导出缺少 JSDoc：${v.name}\n`);
  }
  if (violations.length > 0) process.exit(1);
}
