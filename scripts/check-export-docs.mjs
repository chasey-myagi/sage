// Export JSDoc gate: every exported declaration in package and app sources
// carries a `/** ... */` block directly above it.
// Not model-visible and not a Log entry: this only reads source text.
import { readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

/** Source roots scanned, relative to the repository root. */
export const SOURCE_GLOBS = ["packages/*/src", "apps/*/src"];

// Any `export` statement except a re-export (`export {`, `export *`,
// `export type {`), which is documented at its declaration.
const EXPORT_STATEMENT = /^export\s+(?!\{|\*|type\s*\{)/;

const DECLARED_NAME =
  /^export\s+(?:declare\s+)?(?:default\s+)?(?:abstract\s+)?(?:async\s+)?(?:function\*?|const\s+enum|const|let|var|class|interface|type|enum|namespace)\s+([A-Za-z_$][\w$]*)/;

const FUNCTION_KEYWORD =
  /^export\s+(?:declare\s+)?(?:default\s+)?(?:async\s+)?function\b/;

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
 * Every `export` statement other than a re-export counts, including
 * `enum`, `namespace`, `var`, and anonymous `export default`. A statement is
 * documented when the nearest preceding non-blank line closes a comment
 * block that opened with `/**`. Decorator-free TypeScript as formatted by
 * biome is assumed.
 *
 * @returns One entry per undocumented declaration, with its 1-based line.
 */
export function scanText(text) {
  const lines = text.split("\n");
  /** @type {{ line: number, name: string }[]} */
  const missing = [];
  let previousName = "";
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index] ?? "";
    if (!EXPORT_STATEMENT.test(line)) continue;
    const name = DECLARED_NAME.exec(line)?.[1] ?? "default";
    const isFunction = FUNCTION_KEYWORD.test(line);
    const isOverload = isFunction && name === previousName;
    previousName = isFunction ? name : "";
    if (isOverload) continue;
    if (!hasDocAbove(lines, index)) missing.push({ line: index + 1, name });
  }
  return missing;
}

/**
 * True when the nearest non-blank line above `index` ends a comment block
 * that opened with `/**`. Every line between the closing and opening
 * delimiters must be a comment line, so a trailing `/* note *\/` after code
 * cannot borrow an earlier declaration's block.
 */
function hasDocAbove(lines, index) {
  let cursor = index - 1;
  while (cursor >= 0 && (lines[cursor] ?? "").trim() === "") cursor -= 1;
  if (cursor < 0 || !(lines[cursor] ?? "").trim().endsWith("*/")) return false;
  for (; cursor >= 0; cursor -= 1) {
    const trimmed = (lines[cursor] ?? "").trim();
    if (trimmed.startsWith("/**")) return true;
    if (trimmed.startsWith("/*") || !trimmed.startsWith("*")) return false;
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
