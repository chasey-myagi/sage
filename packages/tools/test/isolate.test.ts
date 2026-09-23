import { mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import type { HarnessTool } from "@sage/core";
import { afterEach, describe, expect, it } from "vitest";
import { type BashExec, createTools } from "../src/index.ts";

const ctx = { sessionId: "s", toolUseId: "tool-isolate" };

let dir = "";
let other = "";

function freshDir(): string {
  dir = mkdtempSync(path.join(tmpdir(), "sage-tools-isolate-"));
  return dir;
}

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  if (other) rmSync(other, { recursive: true, force: true });
  dir = "";
  other = "";
});

function tool(tools: HarnessTool[], name: string): HarnessTool {
  const found = tools.find((item) => item.def.name === name);
  if (!found) throw new Error(`missing tool ${name}`);
  return found;
}

function shQuote(value: string): string {
  return `'${value.replace(/'/g, `'\\''`)}'`;
}

function bodyOf(content: string): string {
  return content.replace(/^\[bash exit code \d+\]\n?/, "");
}

describe("bash temp-root isolation", () => {
  it("denies reading the temp root before it allows the cwd", async () => {
    const cwd = freshDir();
    const seen: { argv?: readonly string[] }[] = [];
    const exec: BashExec = async (_command, opts) => {
      seen.push({ argv: opts.argv });
      return { stdout: "", stderr: "", code: 0 };
    };
    const bash = tool(
      createTools({
        cwd,
        sandbox: { network: "none", fsIsolate: true },
        platform: "darwin",
        exec,
      }),
      "bash",
    );
    await bash.execute({ command: "echo hi" }, ctx);
    const profile = seen[0]?.argv?.[2] ?? "";
    const root = realpathSync(tmpdir());
    const realCwd = realpathSync(cwd);
    const deny = `(deny file-read* (subpath ${JSON.stringify(root)}))`;
    const allow = `(allow file-read* (subpath ${JSON.stringify(realCwd)}))`;
    expect(profile.indexOf(deny)).toBeGreaterThanOrEqual(0);
    expect(profile.indexOf(deny)).toBeLessThan(profile.indexOf(allow));
  });

  it.skipIf(process.platform !== "darwin")(
    "cannot list the temp root or read a sibling file, and can read the cwd",
    async () => {
      const cwd = freshDir();
      writeFileSync(path.join(cwd, "mine.txt"), "mine-secret\n");
      const root = realpathSync(tmpdir());
      other = mkdtempSync(path.join(root, "sage-tools-other-"));
      writeFileSync(path.join(other, "secret.txt"), "other-secret\n");
      const bash = tool(
        createTools({
          cwd,
          sandbox: { network: "none", fsIsolate: true },
        }),
        "bash",
      );
      const listed = await bash.execute(
        { command: `ls -1 ${shQuote(root)}` },
        ctx,
      );
      expect(listed.isError).toBe(true);
      expect(bodyOf(listed.content)).not.toContain(path.basename(other));
      expect(listed.content).not.toContain("other-secret");

      const stolen = await bash.execute(
        { command: `cat ${shQuote(path.join(other, "secret.txt"))}` },
        ctx,
      );
      expect(stolen.isError).toBe(true);
      expect(stolen.content).not.toContain("other-secret");

      const mine = await bash.execute({ command: "cat mine.txt" }, ctx);
      expect(mine.isError).toBe(false);
      expect(mine.content).toContain("mine-secret");
      const absolute = await bash.execute(
        { command: `cat ${shQuote(path.join(realpathSync(cwd), "mine.txt"))}` },
        ctx,
      );
      expect(absolute.isError).toBe(false);
      expect(absolute.content).toContain("mine-secret");
    },
    20_000,
  );

  it("leaves linux on unshare because that cannot deny file reads", async () => {
    const cwd = freshDir();
    const seen: { argv?: readonly string[] }[] = [];
    const exec: BashExec = async (_command, opts) => {
      seen.push({ argv: opts.argv });
      return { stdout: "", stderr: "", code: 0 };
    };
    const bash = tool(
      createTools({
        cwd,
        sandbox: { network: "none", fsIsolate: true },
        platform: "linux",
        unshareAvailable: true,
        exec,
      }),
      "bash",
    );
    await bash.execute({ command: "echo hi" }, ctx);
    expect(seen[0]?.argv).toEqual(["unshare", "-n", "sh", "-c", "echo hi"]);
  });
});
