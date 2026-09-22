// README gate for the Model Experience section.
// Not model-visible and not a Log entry: this only reads package READMEs.
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const SECTION = "## Model Experience";
const REQUIRED_HEADINGS = [
  "### What the model sees",
  "### Token effect",
  "### KV cache effect",
];

/**
 * @param {string} rootDir
 * @returns {{ violations: { package: string, message: string }[] }}
 */
export function run(rootDir) {
  const root = path.resolve(rootDir);
  const exemptions = loadExemptions(root);
  /** @type {{ package: string, message: string }[]} */
  const violations = [];
  /** @type {Set<string>} */
  const exempted = new Set();
  /** @type {Set<string>} */
  const tainted = new Set();

  for (const entry of exemptions) {
    const packageName = typeof entry?.package === "string" ? entry.package : "";
    if (!packageName) continue;
    const reason = typeof entry.reason === "string" ? entry.reason.trim() : "";
    if (reason.length === 0) {
      tainted.add(packageName);
      violations.push({
        package: packageName,
        message: "exemption reason is empty",
      });
    } else {
      exempted.add(packageName);
    }
  }
  for (const packageName of tainted) exempted.delete(packageName);

  violations.sort((a, b) => a.package.localeCompare(b.package));

  const packages = listPackages(root).sort((a, b) =>
    a.package.localeCompare(b.package),
  );
  for (const pkg of packages) {
    if (exempted.has(pkg.package)) continue;
    const text = readFileSync(pkg.readme, "utf8");
    const headings = headingsUnderSection(text);
    if (!headings) {
      violations.push({ package: pkg.package, message: `missing ${SECTION}` });
      continue;
    }
    for (const heading of REQUIRED_HEADINGS) {
      if (!headings.has(heading)) {
        violations.push({
          package: pkg.package,
          message: `missing ${heading}`,
        });
      }
    }
  }

  return { violations };
}

/**
 * @param {string} root
 * @returns {{ package?: string, reason?: string }[]}
 */
function loadExemptions(root) {
  const file = path.join(root, "scripts", "readme-exemptions.json");
  if (!existsSync(file)) return [];
  const parsed = JSON.parse(readFileSync(file, "utf8"));
  if (!Array.isArray(parsed)) {
    throw new Error("scripts/readme-exemptions.json must be a JSON array");
  }
  return parsed;
}

/**
 * @param {string} root
 * @returns {{ package: string, readme: string }[]}
 */
function listPackages(root) {
  /** @type {{ package: string, readme: string }[]} */
  const found = [];
  for (const top of ["packages", "apps"]) {
    const dir = path.join(root, top);
    if (!existsSync(dir)) continue;
    for (const name of readdirSync(dir)) {
      const readme = path.join(dir, name, "README.md");
      if (!existsSync(readme) || !statSync(readme).isFile()) continue;
      found.push({ package: `${top}/${name}`, readme });
    }
  }
  return found;
}

/**
 * @param {string} markdown
 * @returns {Set<string> | null}
 */
function headingsUnderSection(markdown) {
  const lines = markdown.split(/\r?\n/);
  const start = lines.findIndex((line) => line.trim() === SECTION);
  if (start < 0) return null;
  /** @type {Set<string>} */
  const found = new Set();
  for (const line of lines.slice(start + 1)) {
    const trimmed = line.trim();
    if (/^## [^#]/.test(trimmed)) break;
    if (REQUIRED_HEADINGS.includes(trimmed)) found.add(trimmed);
  }
  return found;
}

if (invokedAsCli()) {
  const root = process.argv[2] ? path.resolve(process.argv[2]) : process.cwd();
  const { violations } = run(root);
  for (const v of violations) {
    process.stdout.write(`${v.package}: ${v.message}\n`);
  }
  if (violations.length > 0) process.exit(1);
}

function invokedAsCli() {
  const entry = process.argv[1];
  if (!entry) return false;
  return import.meta.url === pathToFileURL(path.resolve(entry)).href;
}
