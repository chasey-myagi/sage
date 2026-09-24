import {
  copyFileSync,
  existsSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import type { HarnessTool } from "@sage/core";
import { afterEach, describe, expect, it } from "vitest";
import { searchWithoutRg } from "../../src/grep.ts";
import { createTools } from "../../src/index.ts";

const ctx = { sessionId: "s", toolUseId: "tool-51" };
const repoRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../../..",
);
const fixture = path.join(
  repoRoot,
  "packages/plugins/test/replay/fixtures/issue-46/long-deepseek-flash.jsonl",
);

let dir = "";

function freshDir(): string {
  dir = mkdtempSync(path.join(tmpdir(), "sage-grep-51-"));
  return dir;
}

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

function tool(tools: HarnessTool[], name: string): HarnessTool {
  const found = tools.find((item) => item.def.name === name);
  if (!found) throw new Error(`missing tool ${name}`);
  return found;
}

describe("grep hit line cap", () => {
  it("caps a 5000-character mid-line hit at 320 characters", async () => {
    const cwd = freshDir();
    const needle = "NEEDLE";
    const line = `${"a".repeat(2497)}${needle}${"b".repeat(2497)}`;
    expect(line).toHaveLength(5000);
    const at = line.indexOf(needle);
    const shown = line.slice(at - 150, at + 150);
    const expected = `f:1: …${shown}…[+${line.length - shown.length} chars]`;
    expect(expected.length).toBeLessThanOrEqual(320);
    writeFileSync(path.join(cwd, "f"), line);

    const grep = tool(createTools({ cwd }), "grep");
    const viaTool = await grep.execute({ pattern: needle, path: "f" }, ctx);
    const viaWalk = await searchWithoutRg({
      cwd,
      pattern: needle,
      root: "f",
      maxResults: 200,
    });
    expect(viaTool.isError).toBe(false);
    expect(viaWalk.isError).toBe(false);
    expect(viaWalk.content).toBe(viaTool.content);

    const hit = viaTool.content;
    expect(hit.length).toBeLessThanOrEqual(320);
    expect(hit).toContain(needle);
    expect(hit).toMatch(/….*NEEDLE.*…/);
    expect(hit).toMatch(/\[\+\d+ chars\]/);
    expect(hit).toBe(expected);
  });

  it("keeps a source hit under 300 characters byte-for-byte", async () => {
    const cwd = freshDir();
    const source = "export const GREP_MAX_RESULTS = 200;";
    expect(source.length).toBeLessThan(300);
    writeFileSync(path.join(cwd, "grep.ts"), `${source}\n`);
    const grep = tool(createTools({ cwd }), "grep");
    const viaTool = await grep.execute(
      { pattern: "GREP_MAX_RESULTS", path: "grep.ts" },
      ctx,
    );
    const viaWalk = await searchWithoutRg({
      cwd,
      pattern: "GREP_MAX_RESULTS",
      root: "grep.ts",
      maxResults: 200,
    });
    expect(viaTool.isError).toBe(false);
    expect(viaWalk.isError).toBe(false);
    expect(viaTool.content).toBe(
      "grep.ts:1: export const GREP_MAX_RESULTS = 200;",
    );
    expect(viaWalk.content).toBe(viaTool.content);
  });

  it("keeps a recorded long-line grep call under 8000 characters", async () => {
    // Hermetic: the real call swept the whole repo, and later fixtures
    // (issue-50 and beyond) add more hits. Only the long-line fixture matters
    // for the cap, so copy that one file into an empty directory.
    expect(existsSync(fixture)).toBe(true);
    const cwd = freshDir();
    copyFileSync(fixture, path.join(cwd, "long-deepseek-flash.jsonl"));
    const grep = tool(createTools({ cwd }), "grep");
    const result = await grep.execute(
      { pattern: "formatHits|optionalBool|searchWithoutRg" },
      ctx,
    );
    expect(result.isError).toBe(false);
    const lines = result.content.split("\n");
    expect(lines.length).toBe(9);
    for (const line of lines) {
      expect(line.startsWith("long-deepseek-flash.jsonl:")).toBe(true);
      expect(line).toContain("[+");
    }
    expect(result.content.length).toBeLessThan(8000);
  });
});
