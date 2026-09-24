import {
  existsSync,
  mkdtempSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { createFaux, stopScript, toolScript } from "@sage/core/testing";
import type { ProviderAdapter } from "@sage/provider";
import { afterEach, expect, it } from "vitest";
import { parseArgs } from "../src/args.ts";
import { runCli } from "../src/main.ts";

let dir = "";
let orphan = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  if (orphan) rmSync(orphan, { recursive: true, force: true });
  dir = "";
  orphan = "";
});

it("puts two tasks under one run root, hides the finished task, and deletes the root", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-case-"));
  const tasks = path.join(dir, "tasks.json");
  writeFileSync(tasks, JSON.stringify([task("one"), task("two")]));
  const cwds: string[] = [];
  let runRoot = "";
  const io = capture();
  const code = await runCli(
    [
      "bench",
      "run",
      "--tasks",
      tasks,
      "--policy",
      "compiler",
      "--model",
      "claude-test",
      "--out",
      path.join(dir, "out"),
    ],
    {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: io.stdout,
      stderr: io.stderr,
      benchDeps: {
        async prepare(spec, cwd) {
          const real = realpathSync(cwd);
          if (spec.id === "two") {
            expect(existsSync(cwds[0] ?? "")).toBe(false);
            expect(existsSync(path.dirname(real))).toBe(true);
          }
          cwds.push(real);
          runRoot = path.dirname(real);
        },
        async runSage() {
          return { code: 0 };
        },
        async diffEmpty() {
          return true;
        },
        async runTests() {
          return { code: 0 };
        },
      },
    },
  );
  expect(code).toBe(0);
  expect(cwds).toHaveLength(2);
  const first = cwds[0] ?? "";
  const second = cwds[1] ?? "";
  expect(path.dirname(first)).toBe(path.dirname(second));
  expect(path.basename(runRoot)).toMatch(/^sage-bench-.+/);
  expect(realpathSync(path.dirname(runRoot))).toBe(realpathSync(tmpdir()));
  expect(path.basename(first)).toBe("one");
  expect(path.basename(second)).toBe("two");
  expect(first.startsWith(`${second}${path.sep}`)).toBe(false);
  expect(second.startsWith(`${first}${path.sep}`)).toBe(false);
  expect(io.text().stderr).not.toContain(path.basename(runRoot));
  expect(existsSync(runRoot)).toBe(false);
  expect(existsSync(first)).toBe(false);
  expect(existsSync(second)).toBe(false);
});

it("lists leftover sage-bench directories and does not delete them", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-case-"));
  orphan = mkdtempSync(path.join(realpathSync(tmpdir()), "sage-bench-orphan-"));
  const tasks = path.join(dir, "tasks.json");
  writeFileSync(tasks, JSON.stringify([task("one")]));
  const io = capture();
  const code = await runCli(
    [
      "bench",
      "run",
      "--tasks",
      tasks,
      "--policy",
      "compiler",
      "--model",
      "claude-test",
      "--out",
      path.join(dir, "out"),
    ],
    {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: io.stdout,
      stderr: io.stderr,
      benchDeps: {
        async prepare() {},
        async runSage() {
          return { code: 0 };
        },
        async diffEmpty() {
          return true;
        },
        async runTests() {
          return { code: 0 };
        },
      },
    },
  );
  expect(code).toBe(0);
  expect(io.text().stderr).toContain(path.basename(orphan));
  expect(io.text().stderr.toLowerCase()).toContain("manually");
  expect(existsSync(orphan)).toBe(true);
});

it("passes the run root to the sandbox as allowRoot", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-case-"));
  const tasks = path.join(dir, "tasks.json");
  writeFileSync(tasks, JSON.stringify([task("one")]));
  const io = capture();
  let runRoot = "";
  let allowRoot: string | undefined;
  const code = await runCli(
    [
      "bench",
      "run",
      "--tasks",
      tasks,
      "--policy",
      "compiler",
      "--model",
      "claude-test",
      "--out",
      path.join(dir, "out"),
    ],
    {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: io.stdout,
      stderr: io.stderr,
      benchDeps: {
        async prepare() {},
        async runSage(_task, cwd, opts) {
          runRoot = path.dirname(realpathSync(cwd));
          allowRoot = opts.sandbox.allowRoot;
          return { code: 0 };
        },
        async diffEmpty() {
          return true;
        },
        async runTests() {
          return { code: 0 };
        },
      },
    },
  );
  expect(code).toBe(0);
  expect(allowRoot).toBe(runRoot);
});

it("reads SAGE_SANDBOX_ALLOW_ROOT into the bash allow rule", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-case-"));
  const allow = mkdtempSync(path.join(realpathSync(tmpdir()), "sage-bench-"));
  try {
    const seen: string[] = [];
    const { adapter } = createFaux([
      toolScript("bash1", "bash", JSON.stringify({ command: "echo hi" })),
      stopScript("done"),
    ]);
    const io = capture();
    const code = await runCli(
      ["-p", "x", "--passthrough", "--tool-network", "none"],
      {
        cwd: dir,
        env: {
          ANTHROPIC_API_KEY: "test-key",
          SAGE_SANDBOX_ALLOW_ROOT: allow,
        },
        stdout: io.stdout,
        stderr: io.stderr,
        adapter: adapter as ProviderAdapter,
        platform: "darwin",
        exec: async (_command, opts) => {
          seen.push(String(opts.argv[2] ?? ""));
          return { stdout: "", stderr: "", code: 0 };
        },
      },
    );
    expect(code, io.text().stderr).toBe(0);
    expect(seen[0] ?? "").toContain(
      `(allow file-read* (subpath ${JSON.stringify(realpathSync(allow))}))`,
    );
  } finally {
    rmSync(allow, { recursive: true, force: true });
  }
});

it("accepts --tool-sandbox as an alias of --tool-network", async () => {
  const parsed = parseArgs(["-p", "x", "--tool-sandbox", "none"]);
  expect(parsed.ok).toBe(true);
  if (!parsed.ok) return;
  expect(parsed.toolNetwork).toBe("none");
  const bad = parseArgs(["-p", "x", "--tool-sandbox", "maybe"]);
  expect(bad).toEqual({
    ok: false,
    exitCode: 2,
    message: 'Invalid --tool-sandbox "maybe": expected none or full',
  });

  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-case-"));
  const tasks = path.join(dir, "tasks.json");
  writeFileSync(tasks, JSON.stringify([task("one")]));
  const argv: string[][] = [];
  const io = capture();
  const code = await runCli(
    [
      "bench",
      "run",
      "--tasks",
      tasks,
      "--policy",
      "compiler",
      "--model",
      "claude-test",
      "--tool-sandbox",
      "full",
      "--out",
      path.join(dir, "out"),
    ],
    {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: io.stdout,
      stderr: io.stderr,
      benchDeps: {
        async prepare() {},
        async runSage(_task, _cwd, opts) {
          argv.push(opts.argv);
          expect(opts.sandbox).toEqual({ network: "full", fsIsolate: false });
          return { code: 0 };
        },
        async diffEmpty() {
          return true;
        },
        async runTests() {
          return { code: 0 };
        },
      },
    },
  );
  expect(code).toBe(0);
  expect(argv[0] ?? []).toEqual(
    expect.arrayContaining(["--tool-network", "full"]),
  );
});

function task(id: string) {
  return {
    id,
    repo: "owner/repo",
    baseCommit: "abc",
    problem: "fix it",
    testCommand: "true",
    timeoutSec: 30,
  };
}

function capture() {
  let stdout = "";
  let stderr = "";
  return {
    stdout: {
      write(chunk: string) {
        stdout += chunk;
      },
    },
    stderr: {
      write(chunk: string) {
        stderr += chunk;
      },
    },
    text() {
      return { stdout, stderr };
    },
  };
}
