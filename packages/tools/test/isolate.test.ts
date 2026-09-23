import {
  mkdirSync,
  mkdtempSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
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

function escapeSandboxRegex(value: string): string {
  return value.replace(/[\\^$.*+?()[\]{}|]/g, "\\$&");
}

function repoRoot(): string {
  return fileURLToPath(new URL("../../../", import.meta.url));
}

describe("bash bench workspace isolation", () => {
  it("denies sibling bench workspaces before it allows the cwd", async () => {
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
    const pattern = `^${escapeSandboxRegex(root)}/sage-bench-`;
    const deny = `(deny file-read* (regex #"${pattern}"))`;
    const allow = `(allow file-read* (subpath ${JSON.stringify(realCwd)}))`;
    expect(profile).not.toContain(
      `(deny file-read* (subpath ${JSON.stringify(root)}))`,
    );
    expect(profile.indexOf(deny)).toBeGreaterThanOrEqual(0);
    expect(profile.indexOf(deny)).toBeLessThan(profile.indexOf(allow));
  });

  it("escapes the caller temp root and bench prefix in the deny regex", async () => {
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
        tmpRoot: "/tmp/a.b+c",
        benchPrefix: "pre.fix+",
        exec,
      }),
      "bash",
    );
    await bash.execute({ command: "echo hi" }, ctx);
    const profile = seen[0]?.argv?.[2] ?? "";
    const deny = `(deny file-read* (regex #"^/tmp/a\\.b\\+c/pre\\.fix\\+"))`;
    const allow = `(allow file-read* (subpath ${JSON.stringify(realpathSync(cwd))}))`;
    expect(profile.indexOf(deny)).toBeGreaterThanOrEqual(0);
    expect(profile.indexOf(deny)).toBeLessThan(profile.indexOf(allow));
  });

  it.skipIf(process.platform !== "darwin")(
    "reads back a file written under os.tmpdir()",
    async () => {
      const cwd = freshDir();
      const bash = tool(
        createTools({
          cwd,
          sandbox: { network: "none", fsIsolate: true },
        }),
        "bash",
      );
      const script = [
        'const fs=require("node:fs");',
        'const os=require("node:os");',
        'const path=require("node:path");',
        'const file=path.join(os.tmpdir(),"sage-tmp-roundtrip-"+process.pid);',
        "try{",
        'fs.writeFileSync(file,"roundtrip-ok\\n");',
        'const text=fs.readFileSync(file,"utf8");',
        'if(text!=="roundtrip-ok\\n")process.exit(3);',
        "}finally{try{fs.unlinkSync(file)}catch(e){}}",
      ].join("");
      const result = await bash.execute(
        { command: `node -e ${shQuote(script)}` },
        ctx,
      );
      expect(result.isError, result.content).toBe(false);
      expect(result.content).toMatch(/^\[bash exit code 0\]/);
    },
    20_000,
  );

  it.skipIf(process.platform !== "darwin")(
    "runs a repo vitest file inside the sandbox",
    async () => {
      const bash = tool(
        createTools({
          cwd: repoRoot(),
          sandbox: { network: "none", fsIsolate: true },
        }),
        "bash",
      );
      const result = await bash.execute(
        {
          command:
            "node node_modules/vitest/vitest.mjs run packages/tools/test/truncate.test.ts --reporter=verbose",
        },
        ctx,
      );
      expect(result.isError, result.content).toBe(false);
      expect(result.content).toMatch(/^\[bash exit code 0\]/);
      expect(result.content).toContain("packages/tools/test/truncate.test.ts");
    },
    60_000,
  );

  it.skipIf(process.platform !== "darwin")(
    "cannot read a sibling bench file, and can read the cwd",
    async () => {
      const root = realpathSync(tmpdir());
      dir = mkdtempSync(path.join(root, "sage-bench-"));
      const cwd = path.join(dir, "task");
      mkdirSync(cwd);
      writeFileSync(path.join(cwd, "mine.txt"), "mine-secret\n");
      other = mkdtempSync(path.join(root, "sage-bench-"));
      writeFileSync(path.join(other, "f"), "sibling-secret\n");
      const bash = tool(
        createTools({
          cwd,
          sandbox: { network: "none", fsIsolate: true },
        }),
        "bash",
      );
      const stolen = await bash.execute(
        { command: `cat ${shQuote(path.join(other, "f"))}` },
        ctx,
      );
      expect(stolen.isError).toBe(true);
      expect(stolen.content).not.toContain("sibling-secret");
      expect(bodyOf(stolen.content)).toMatch(/Operation not permitted/);

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
