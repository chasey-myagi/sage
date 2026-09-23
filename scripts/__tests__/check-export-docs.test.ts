import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { run, scanText } from "../check-export-docs.mjs";

const script = fileURLToPath(
  new URL("../check-export-docs.mjs", import.meta.url),
);
const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) {
    rmSync(root, { recursive: true, force: true });
  }
});

function makeRoot(files: Record<string, string>): string {
  const root = mkdtempSync(join(tmpdir(), "sage-export-docs-"));
  roots.push(root);
  for (const [rel, body] of Object.entries(files)) {
    const file = join(root, rel);
    mkdirSync(join(file, ".."), { recursive: true });
    writeFileSync(file, body);
  }
  return root;
}

describe("scanText", () => {
  it("accepts every declaration kind when a JSDoc block sits directly above", () => {
    const text = [
      "/** A. */",
      "export function a() {}",
      "/** B. */",
      "export async function b() {}",
      "/**",
      " * C.",
      " */",
      "export const C = 1;",
      "/** D. */",
      "export type D = string;",
      "/** E. */",
      "export interface E {}",
      "/** F. */",
      "export class F {}",
      "",
    ].join("\n");
    expect(scanText(text)).toEqual([]);
  });

  it("reports an undocumented export with its name and 1-based line", () => {
    const text =
      "/** Doc. */\nexport const ok = 1;\n\nexport function bare() {}\n";
    expect(scanText(text)).toEqual([{ line: 4, name: "bare" }]);
  });

  it("does not count a plain block or line comment as documentation", () => {
    const text = [
      "/* plain */",
      "export const a = 1;",
      "// line",
      "export const b = 2;",
      "",
    ].join("\n");
    expect(scanText(text).map((v) => v.name)).toEqual(["a", "b"]);
  });

  it("skips re-exports because their docs live at the declaration", () => {
    const text = [
      'export { a } from "./a.js";',
      'export * from "./b.js";',
      'export type { C } from "./c.js";',
      "",
    ].join("\n");
    expect(scanText(text)).toEqual([]);
  });

  it("needs one block for consecutive overload signatures", () => {
    const text = [
      "/** Parse. */",
      "export function parse(a: string): number;",
      "export function parse(a: number): number;",
      "export function parse(a: unknown): number { return 0; }",
      "",
    ].join("\n");
    expect(scanText(text)).toEqual([]);
  });

  it("checks enum, namespace, var, and anonymous default exports", () => {
    const text = [
      "export enum Color { Red }",
      "export const enum Flag { On }",
      "export var legacy = 1;",
      "export namespace Space {}",
      "export default function () {}",
      "",
    ].join("\n");
    expect(scanText(text).map((v) => v.name)).toEqual([
      "Color",
      "Flag",
      "legacy",
      "Space",
      "default",
    ]);
  });

  it("does not let a trailing comment after code borrow an earlier block", () => {
    const text = [
      "/** A. */",
      "export const a = 1;",
      "const b = 2; /* note */",
      "export const c = 3;",
      "",
    ].join("\n");
    expect(scanText(text)).toEqual([{ line: 4, name: "c" }]);
  });

  it("allows blank lines between the block and the declaration", () => {
    expect(scanText("/** Doc. */\n\nexport const a = 1;\n")).toEqual([]);
  });
});

describe("run", () => {
  it("scans package and app sources and ignores tests", () => {
    const root = makeRoot({
      "packages/core/src/a.ts": "export const a = 1;\n",
      "packages/core/test/a.test.ts": "export const t = 1;\n",
      "apps/sage/src/b.ts": "/** B. */\nexport const b = 1;\n",
    });
    expect(run(root).violations).toEqual([
      { file: "packages/core/src/a.ts", line: 1, name: "a" },
    ]);
  });

  it("exits 1 from the CLI and names each violation", () => {
    const root = makeRoot({ "packages/x/src/a.ts": "export type A = 1;\n" });
    const result = spawnSync(process.execPath, [script, root], {
      encoding: "utf8",
    });
    expect(result.status).toBe(1);
    expect(result.stdout).toContain("packages/x/src/a.ts:1");
  });

  it("exits 0 from the CLI when every export is documented", () => {
    const root = makeRoot({
      "packages/x/src/a.ts": "/** A. */\nexport type A = 1;\n",
    });
    const result = spawnSync(process.execPath, [script, root], {
      encoding: "utf8",
    });
    expect(result.status).toBe(0);
  });
});
