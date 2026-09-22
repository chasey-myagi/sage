import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import type { HarnessTool } from "@sage/core";
import { afterEach, describe, expect, it } from "vitest";
import { searchWithoutRg } from "../src/grep.ts";
import { type BashExec, createTools } from "../src/index.ts";
import { READ_MAX_BYTES } from "../src/read.ts";

const ctx = { sessionId: "s", toolUseId: "tool-default" };
let dir = "";
const outsidePaths: string[] = [];

function freshDir(): string {
  dir = mkdtempSync(path.join(tmpdir(), "sage-tools-"));
  return dir;
}

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  for (const target of outsidePaths.splice(0)) {
    rmSync(target, { recursive: true, force: true });
  }
});

function tool(tools: HarnessTool[], name: string): HarnessTool {
  const found = tools.find((item) => item.def.name === name);
  if (!found) throw new Error(`missing tool ${name}`);
  return found;
}

function outsideFile(name: string, content: string): string {
  const target = path.join(path.dirname(dir), name);
  outsidePaths.push(target);
  writeFileSync(target, content);
  return target;
}

describe("scenario 1 tool declarations", () => {
  it("declares danger flags and accesses for all seven tools", () => {
    const cwd = freshDir();
    const file = path.join(cwd, "a.txt");
    writeFileSync(file, "hello");
    mkdirSync(path.join(cwd, "sub"));
    const tools = createTools({ cwd });
    expect(tools.map((item) => item.def.name)).toEqual([
      "read",
      "write",
      "edit",
      "bash",
      "grep",
      "find",
      "ls",
    ]);

    const read = tool(tools, "read");
    const write = tool(tools, "write");
    const edit = tool(tools, "edit");
    const bash = tool(tools, "bash");
    const grep = tool(tools, "grep");
    const find = tool(tools, "find");
    const ls = tool(tools, "ls");

    for (const item of [read, grep, find, ls]) {
      expect(item.isReadOnly).toBe(true);
      expect(item.isDestructive).toBe(false);
      expect(item.isOpenWorld).toBe(false);
    }
    for (const item of [write, edit]) {
      expect(item.isReadOnly).toBe(false);
      expect(item.isDestructive).toBe(true);
      expect(item.isOpenWorld).toBe(false);
    }
    expect(bash.isReadOnly).toBe(false);
    expect(bash.isDestructive).toBe(true);
    expect(bash.isOpenWorld).toBe(true);

    const fileKey = realpathSync(file);
    const cwdKey = realpathSync(cwd);
    const subKey = realpathSync(path.join(cwd, "sub"));
    expect(read.accesses({ path: "a.txt" })).toEqual([
      { key: fileKey, mode: "read", recursive: false },
    ]);
    expect(write.accesses({ path: "a.txt", content: "x" })).toEqual([
      { key: fileKey, mode: "write", recursive: false },
    ]);
    expect(
      edit.accesses({ path: "a.txt", old_string: "h", new_string: "H" }),
    ).toEqual([{ key: fileKey, mode: "write", recursive: false }]);
    expect(grep.accesses({ pattern: "h", path: "sub" })).toEqual([
      { key: subKey, mode: "search", recursive: true },
    ]);
    expect(find.accesses({ glob: "*.txt" })).toEqual([
      { key: cwdKey, mode: "search", recursive: true },
    ]);
    expect(ls.accesses({})).toEqual([
      { key: cwdKey, mode: "search", recursive: true },
    ]);
    expect(bash.accesses({ command: "rm -rf /tmp/nope" })).toEqual([]);

    expect(read.def.inputSchema).toMatchObject({
      type: "object",
      required: ["path"],
    });
    expect(write.def.inputSchema).toMatchObject({
      required: ["path", "content"],
    });
    expect(edit.def.inputSchema).toMatchObject({
      required: ["path", "old_string", "new_string"],
    });
    expect(bash.def.inputSchema).toMatchObject({ required: ["command"] });
    expect(grep.def.inputSchema).toMatchObject({ required: ["pattern"] });
    expect(find.def.inputSchema).toMatchObject({ required: ["glob"] });
    expect(ls.def.description.length).toBeGreaterThan(0);
  });
});

describe("scenario 2 path escape", () => {
  it("returns isError and does not read or write outside the workspace", async () => {
    const cwd = freshDir();
    const secret = outsideFile("sage-secret.txt", "secret-token");
    symlinkSync(secret, path.join(cwd, "link.txt"));
    const outsideDir = path.join(path.dirname(cwd), "sage-outside-dir");
    outsidePaths.push(outsideDir);
    mkdirSync(outsideDir);
    writeFileSync(path.join(outsideDir, "x.txt"), "secret-token");
    symlinkSync(outsideDir, path.join(cwd, "link-dir"));
    const tools = createTools({ cwd });
    const cases: Array<{ name: string; input: unknown }> = [
      { name: "read", input: { path: "../../etc/passwd" } },
      { name: "read", input: { path: "link.txt" } },
      { name: "read", input: { path: "link-dir/x.txt" } },
      { name: "write", input: { path: "../sage-pwned.txt", content: "nope" } },
      {
        name: "write",
        input: { path: "link-dir/pwned.txt", content: "nope" },
      },
      {
        name: "edit",
        input: { path: "link.txt", old_string: "secret", new_string: "x" },
      },
      { name: "grep", input: { pattern: "secret-token", path: ".." } },
      { name: "find", input: { glob: "**/*", path: ".." } },
      { name: "ls", input: { path: ".." } },
    ];
    for (const item of cases) {
      const result = await tool(tools, item.name).execute(item.input, ctx);
      expect(result.isError, item.name).toBe(true);
      expect(result.content).not.toContain("secret-token");
      expect(result.content).not.toMatch(/^root:/m);
    }
    expect(readFileSync(secret, "utf8")).toBe("secret-token");
    expect(path.join(path.dirname(cwd), "sage-pwned.txt")).toBeTruthy();
    const pwned = path.join(path.dirname(cwd), "sage-pwned.txt");
    expect(() => readFileSync(pwned, "utf8")).toThrow();
    expect(() =>
      readFileSync(path.join(outsideDir, "pwned.txt"), "utf8"),
    ).toThrow();
  });
});

describe("scenario 3 read paging", () => {
  it("returns the first 2000 lines and a continuation stub", async () => {
    const cwd = freshDir();
    const lines = Array.from({ length: 5000 }, (_, index) => `L${index + 1}`);
    writeFileSync(path.join(cwd, "big.txt"), lines.join("\n"));
    const read = tool(createTools({ cwd }), "read");
    const first = await read.execute({ path: "big.txt" }, ctx);
    const marker =
      "[truncated: showing lines 1-2000 of 5000; call read with offset=2001 to continue]";
    expect(first.isError).toBe(false);
    expect(first.content.endsWith(marker)).toBe(true);
    const body = first.content.slice(0, -marker.length).replace(/\n$/, "");
    const shown = body.split("\n");
    expect(shown).toHaveLength(2000);
    expect(shown[0]).toBe("L1");
    expect(shown[1999]).toBe("L2000");
    expect(body).not.toContain("\nL2001");

    const second = await read.execute(
      { path: "big.txt", offset: 2001, limit: 2 },
      ctx,
    );
    expect(second.isError).toBe(false);
    expect(second.content.startsWith("L2001\nL2002\n")).toBe(true);
    expect(second.content).toContain(
      "[truncated: showing lines 2001-2002 of 5000; call read with offset=2003 to continue]",
    );
  });

  it("does not return a partial line when a line exceeds the byte budget", async () => {
    const cwd = freshDir();
    const huge = "a".repeat(READ_MAX_BYTES + 8);
    writeFileSync(path.join(cwd, "wide.txt"), huge);
    const result = await tool(createTools({ cwd }), "read").execute(
      { path: "wide.txt" },
      ctx,
    );
    expect(result.isError).toBe(true);
    expect(result.content).not.toContain(huge.slice(0, 100));
  });
});

describe("scenario 4 bash spill", () => {
  it("spills output past 30000 bytes and names the file after the tool use id", async () => {
    const cwd = freshDir();
    const output = "x".repeat(30_001);
    const exec: BashExec = async () => ({
      stdout: output,
      stderr: "",
      code: 0,
    });
    const bash = tool(createTools({ cwd, exec }), "bash");
    const result = await bash.execute(
      { command: "ignored" },
      { sessionId: "s", toolUseId: "toolu_spill" },
    );
    const saved = path.join(cwd, ".sage", "tool-output", "toolu_spill.txt");
    expect(result.isError).toBe(false);
    expect(result.content.startsWith("[bash exit code 0]\n")).toBe(true);
    const body = result.content.replace(/^\[bash exit code 0\]\n/, "");
    expect(Buffer.byteLength(body)).toBeLessThanOrEqual(30_000);
    expect(body.startsWith("x".repeat(20_000))).toBe(true);
    expect(body.endsWith("x".repeat(5_000))).toBe(true);
    expect(body).toContain(
      `[… omitted 5001 bytes; full output saved to ${saved}; read it with read(path, offset=…) …]`,
    );
    expect(readFileSync(saved, "utf8")).toBe(output);
  });
});

describe("scenario 5 bash exec injection", () => {
  it("uses the injected exec and forwards cwd, signal, and timeout", async () => {
    const cwd = freshDir();
    const seen: Array<{
      command: string;
      cwd: string;
      timeoutMs: number;
      aborted: boolean;
    }> = [];
    const controller = new AbortController();
    const exec: BashExec = async (command, opts) => {
      seen.push({
        command,
        cwd: opts.cwd,
        timeoutMs: opts.timeoutMs,
        aborted: opts.signal?.aborted === true,
      });
      return { stdout: "from-exec\n", stderr: "", code: 0 };
    };
    const bash = tool(createTools({ cwd, exec }), "bash");
    const result = await bash.execute(
      { command: "echo should-not-run", timeout_ms: 5000 },
      { sessionId: "s", toolUseId: "exec-inject", signal: controller.signal },
    );
    expect(result.isError).toBe(false);
    expect(result.content).toBe("[bash exit code 0]\nfrom-exec\n");
    expect(seen).toEqual([
      {
        command: "echo should-not-run",
        cwd: realpathSync(cwd),
        timeoutMs: 5000,
        aborted: false,
      },
    ]);
  });

  it.skipIf(process.platform === "win32")(
    "runs echo with the default exec",
    async () => {
      const cwd = freshDir();
      const bash = tool(createTools({ cwd }), "bash");
      const result = await bash.execute({ command: "echo hello" }, ctx);
      expect(result.isError).toBe(false);
      expect(result.content).toBe("[bash exit code 0]\nhello\n");
    },
  );
});

describe("scenario 6 edit", () => {
  it("replaces old_string only when it occurs exactly once", async () => {
    const cwd = freshDir();
    const file = path.join(cwd, "a.txt");
    writeFileSync(file, "one greet here");
    const edit = tool(createTools({ cwd }), "edit");
    const ok = await edit.execute(
      { path: "a.txt", old_string: "greet", new_string: "sayHello" },
      ctx,
    );
    expect(ok.isError).toBe(false);
    expect(readFileSync(file, "utf8")).toBe("one sayHello here");

    writeFileSync(file, "greet greet");
    const twice = await edit.execute(
      { path: "a.txt", old_string: "greet", new_string: "sayHello" },
      ctx,
    );
    expect(twice.isError).toBe(true);
    expect(twice.content).toBe(
      "old_string matched 2 times; expected exactly one",
    );
    expect(readFileSync(file, "utf8")).toBe("greet greet");

    const missing = await edit.execute(
      { path: "a.txt", old_string: "absent", new_string: "x" },
      ctx,
    );
    expect(missing.isError).toBe(true);
    expect(missing.content).toBe(
      "old_string matched 0 times; expected exactly one",
    );
    expect(readFileSync(file, "utf8")).toBe("greet greet");
  });
});

describe("grep find ls", () => {
  it("formats grep hits and caps them", async () => {
    const cwd = freshDir();
    const lines = Array.from({ length: 5 }, (_, index) => `hit ${index}`);
    writeFileSync(path.join(cwd, "notes.txt"), lines.join("\n"));
    const grep = tool(createTools({ cwd }), "grep");
    const result = await grep.execute(
      { pattern: "hit", path: "notes.txt", max_results: 2 },
      ctx,
    );
    expect(result.isError).toBe(false);
    expect(result.content).toBe(
      [
        "notes.txt:1: hit 0",
        "notes.txt:2: hit 1",
        "[truncated: showing 2 of 5 matches; narrow the path or raise max_results]",
      ].join("\n"),
    );
  });

  it("falls back to the in-process searcher with the same hit format", async () => {
    const cwd = freshDir();
    mkdirSync(path.join(cwd, "src"));
    writeFileSync(path.join(cwd, "src", "a.ts"), "const greet = 1;\n");
    const hits = await searchWithoutRg({
      cwd,
      pattern: "greet",
      root: cwd,
      maxResults: 200,
    });
    expect(hits.isError).toBe(false);
    expect(hits.content).toBe("src/a.ts:1: const greet = 1;");
  });

  it("finds files by glob and lists a directory", async () => {
    const cwd = freshDir();
    mkdirSync(path.join(cwd, "src"));
    writeFileSync(path.join(cwd, "src", "a.ts"), "export {}");
    writeFileSync(path.join(cwd, "readme.md"), "hi");
    const tools = createTools({ cwd });
    const found = await tool(tools, "find").execute({ glob: "**/*.ts" }, ctx);
    expect(found.isError).toBe(false);
    expect(found.content).toBe("src/a.ts");
    const listed = await tool(tools, "ls").execute({ path: "src" }, ctx);
    expect(listed.isError).toBe(false);
    expect(listed.content).toBe("a.ts");
  });
});
