// Layer gate. RULES is the only policy table; the walker just applies it.
// Not model-visible and not a Log entry: import text never leaves this process.
import { readdirSync, readFileSync } from "node:fs";
import { builtinModules } from "node:module";
import path from "node:path";
import { pathToFileURL } from "node:url";

/** @type {readonly { n: number, include: string | readonly string[], exclude?: string, allow?: readonly string[], deny?: readonly string[] }[]} */
export const RULES = [
  {
    n: 1,
    include: "packages/provider/src/contract/**",
    allow: ["same-dir-relative"],
  },
  {
    n: 2,
    include: "packages/provider/**",
    exclude: "packages/provider/src/contract/**",
    allow: [
      "relative",
      "@sage/provider/contract",
      "node-builtin",
      "third-party",
    ],
  },
  {
    n: 3,
    include: "packages/core/**",
    deny: [
      "@sage/plugins",
      "@sage/tools",
      "apps/**",
      "core-src-provider-root",
      "plugin-tool-names",
    ],
  },
  {
    n: 4,
    include: ["packages/plugins/**", "packages/tools/**"],
    deny: ["plugins-tools-cross", "apps/**"],
  },
  {
    n: 5,
    include: "apps/**",
    allow: ["*"],
  },
];

const SKIP_DIRS = new Set(["node_modules", "dist", ".git"]);
const BUILTINS = new Set(builtinModules);
const IMPORT_FROM =
  /\bimport\s+(?:type\s+)?(?:[^;]*?\sfrom\s+)?["']([^"']+)["']/g;
const EXPORT_FROM = /\bexport\s+(?:type\s+)?[^;]*?\sfrom\s+["']([^"']+)["']/g;
const IMPORT_CALL = /\bimport\s*\(\s*["']([^"']+)["']\s*\)/g;

/**
 * @param {string} rootDir
 * @returns {{ violations: { file: string, line: number, rule: number, source: string }[] }}
 */
export function run(rootDir) {
  const root = path.resolve(rootDir);
  /** @type {{ file: string, line: number, rule: number, source: string }[]} */
  const violations = [];

  const files = walk(root);
  const toolNames = pluginToolNames(root, files);
  for (const file of files) {
    const rel = toPosix(path.relative(root, file));
    const rule = ruleFor(rel);
    if (!rule) continue;
    const text = readFileSync(file, "utf8");
    // Core sources must not recognise a plugin tool by spelling its name;
    // core tests may.
    if (
      rule.deny?.includes("plugin-tool-names") &&
      rel.startsWith("packages/core/src/")
    ) {
      for (const found of findToolNameLiterals(text, toolNames)) {
        violations.push({ file: rel, rule: rule.n, ...found });
      }
    }
    for (const found of findImports(text)) {
      if (isAllowed(rule, found.source, rel)) continue;
      violations.push({
        file: rel,
        line: found.line,
        rule: rule.n,
        source: found.source,
      });
    }
  }

  violations.sort(
    (a, b) =>
      a.file.localeCompare(b.file) ||
      a.line - b.line ||
      a.source.localeCompare(b.source),
  );
  return { violations };
}

// A tool def's `def: { name: "x"` or an exported `*_TOOL_NAME = "x"` constant.
const PLUGIN_TOOL_NAME =
  /\bdef:\s*\{\s*name:\s*(["'])([A-Za-z0-9_-]+)\1|_TOOL_NAME\s*=\s*(["'])([A-Za-z0-9_-]+)\3/g;
const COMMENT = /\/\*[\s\S]*?\*\/|\/\/[^\n]*/g;

/**
 * Tool names the plugins define, harvested from their sources. Names from
 * `@sage/tools` are left out: they collide with core's own literals such as
 * `Access.mode` "read" and "write".
 * @param {string} root
 * @param {readonly string[]} files
 * @returns {Set<string>}
 */
function pluginToolNames(root, files) {
  /** @type {Set<string>} */
  const names = new Set();
  for (const file of files) {
    const rel = toPosix(path.relative(root, file));
    if (!rel.startsWith("packages/plugins/src/")) continue;
    for (const match of readFileSync(file, "utf8").matchAll(PLUGIN_TOOL_NAME)) {
      const name = match[2] ?? match[4];
      if (name) names.add(name);
    }
  }
  return names;
}

/**
 * Quoted occurrences of `names` outside comments. Comments are blanked to
 * spaces first so line numbers stay true.
 * @param {string} text
 * @param {Set<string>} names
 * @returns {{ source: string, line: number }[]}
 */
function findToolNameLiterals(text, names) {
  if (names.size === 0) return [];
  const code = text.replace(COMMENT, (comment) =>
    comment.replace(/[^\n]/g, " "),
  );
  const literal = new RegExp(`["'](${[...names].join("|")})["']`, "g");
  /** @type {{ source: string, line: number }[]} */
  const found = [];
  for (const match of code.matchAll(literal)) {
    if (match.index === undefined) continue;
    found.push({
      source: `"${match[1]}" (plugin tool name)`,
      line: lineAt(code, match.index),
    });
  }
  return found;
}

/**
 * @param {string} dir
 * @param {string[]} acc
 * @returns {string[]}
 */
function walk(dir, acc = []) {
  for (const ent of readdirSync(dir, { withFileTypes: true })) {
    if (SKIP_DIRS.has(ent.name)) continue;
    const full = path.join(dir, ent.name);
    if (ent.isDirectory()) walk(full, acc);
    else if (ent.isFile() && /\.(?:ts|mts)$/.test(ent.name)) acc.push(full);
  }
  return acc;
}

/**
 * @param {string} rel
 */
function ruleFor(rel) {
  for (const rule of RULES) {
    const includes = Array.isArray(rule.include)
      ? rule.include
      : [rule.include];
    if (!includes.some((glob) => matchesGlob(glob, rel))) continue;
    if (rule.exclude && matchesGlob(rule.exclude, rel)) continue;
    return rule;
  }
  return undefined;
}

/**
 * @param {string} glob
 * @param {string} rel
 */
function matchesGlob(glob, rel) {
  const prefix = glob.endsWith("/**") ? glob.slice(0, -3) : glob;
  return rel === prefix || rel.startsWith(`${prefix}/`);
}

/**
 * @param {(typeof RULES)[number]} rule
 * @param {string} spec
 * @param {string} fileRel
 */
function isAllowed(rule, spec, fileRel) {
  if (rule.deny?.some((token) => matchesDeny(token, spec, fileRel)))
    return false;
  if (!rule.allow) return true;
  return rule.allow.some((token) => matchesAllow(token, spec));
}

/**
 * @param {string} token
 * @param {string} spec
 * @param {string} fileRel
 */
function matchesDeny(token, spec, fileRel) {
  if (token === "apps/**") return isUnder(spec, fileRel, "apps/");
  // Core sources see provider only through its contract; core tests may
  // drive a real adapter through the root export.
  if (token === "core-src-provider-root")
    return (
      spec === "@sage/provider" && fileRel.startsWith("packages/core/src/")
    );
  // Checked against string literals in `run`, not against imports.
  if (token === "plugin-tool-names") return false;
  if (token === "plugins-tools-cross")
    return isPluginsToolsCross(spec, fileRel);
  return spec === token || spec.startsWith(`${token}/`);
}

/**
 * @param {string} token
 * @param {string} spec
 */
function matchesAllow(token, spec) {
  switch (token) {
    case "*":
      return true;
    case "same-dir-relative":
      return isSameDirRelative(spec);
    case "relative":
      return spec.startsWith("./") || spec.startsWith("../");
    case "@sage/provider/contract":
      return spec === token || spec.startsWith(`${token}/`);
    case "node-builtin":
      return isNodeBuiltin(spec);
    case "third-party":
      return isThirdParty(spec);
    default:
      return spec === token || spec.startsWith(`${token}/`);
  }
}

/**
 * @param {string} spec
 */
function isSameDirRelative(spec) {
  if (!spec.startsWith("./")) return false;
  const rest = spec.slice(2);
  return rest.length > 0 && !rest.includes("/");
}

/**
 * @param {string} spec
 */
function isNodeBuiltin(spec) {
  const bare = spec.startsWith("node:") ? spec.slice(5) : spec;
  if (BUILTINS.has(spec) || BUILTINS.has(bare) || BUILTINS.has(`node:${bare}`))
    return true;
  const head = bare.split("/")[0] ?? bare;
  return BUILTINS.has(head) || BUILTINS.has(`node:${head}`);
}

/**
 * @param {string} spec
 */
function isThirdParty(spec) {
  if (
    spec.startsWith(".") ||
    spec.startsWith("node:") ||
    spec.startsWith("@sage/")
  )
    return false;
  if (spec === "apps" || spec.startsWith("apps/")) return false;
  return !isNodeBuiltin(spec);
}

/**
 * @param {string} spec
 * @param {string} fileRel
 */
function isPluginsToolsCross(spec, fileRel) {
  const fromPlugins = fileRel.startsWith("packages/plugins/");
  const fromTools = fileRel.startsWith("packages/tools/");
  if (!fromPlugins && !fromTools) return false;
  const hitsTools =
    spec === "@sage/tools" ||
    spec.startsWith("@sage/tools/") ||
    isUnder(spec, fileRel, "packages/tools/");
  const hitsPlugins =
    spec === "@sage/plugins" ||
    spec.startsWith("@sage/plugins/") ||
    isUnder(spec, fileRel, "packages/plugins/");
  return (fromPlugins && hitsTools) || (fromTools && hitsPlugins);
}

/**
 * @param {string} spec
 * @param {string} fileRel
 * @param {string} prefix directory prefix with a trailing slash
 */
function isUnder(spec, fileRel, prefix) {
  if (spec === prefix.slice(0, -1) || spec.startsWith(prefix)) return true;
  if (!spec.startsWith(".")) return false;
  const resolved = normalizeRelative(fileRel, spec);
  const bare = prefix.slice(0, -1);
  return resolved === bare || resolved.startsWith(prefix);
}

/**
 * @param {string} fileRel
 * @param {string} spec
 */
function normalizeRelative(fileRel, spec) {
  const stack = fileRel.split("/").slice(0, -1);
  for (const part of spec.split("/")) {
    if (part === "" || part === ".") continue;
    if (part === "..") stack.pop();
    else stack.push(part);
  }
  return stack.join("/");
}

/**
 * @param {string} text
 * @returns {{ source: string, line: number, index: number }[]}
 */
function findImports(text) {
  /** @type {{ source: string, line: number, index: number }[]} */
  const found = [];
  for (const re of [IMPORT_FROM, EXPORT_FROM, IMPORT_CALL]) {
    re.lastIndex = 0;
    for (const match of text.matchAll(re)) {
      const source = match[1];
      if (!source || match.index === undefined) continue;
      found.push({
        source,
        line: lineAt(text, match.index),
        index: match.index,
      });
    }
  }
  found.sort((a, b) => a.index - b.index);
  return found.filter(
    (item, i) => i === 0 || item.index !== found[i - 1]?.index,
  );
}

/**
 * @param {string} text
 * @param {number} index
 */
function lineAt(text, index) {
  let line = 1;
  for (let i = 0; i < index; i++) if (text.charCodeAt(i) === 10) line++;
  return line;
}

/**
 * @param {string} value
 */
function toPosix(value) {
  return value.split(path.sep).join("/");
}

if (invokedAsCli()) {
  const root = process.argv[2] ? path.resolve(process.argv[2]) : process.cwd();
  const { violations } = run(root);
  for (const v of violations) {
    process.stdout.write(
      `${v.file}:${v.line} 违反规则 ${v.rule}：${v.source}\n`,
    );
  }
  if (violations.length > 0) process.exit(1);
}

function invokedAsCli() {
  const entry = process.argv[1];
  if (!entry) return false;
  return import.meta.url === pathToFileURL(path.resolve(entry)).href;
}
