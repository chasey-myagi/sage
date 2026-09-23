import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { createFaux, stopScript } from "@sage/core/testing";
import type { ProviderAdapter } from "@sage/provider";
import { afterEach, expect, it } from "vitest";
import { runCli } from "../src/main.ts";

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

it("exits 2 when --scorer-apply entry has no model or jev scorer", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-scorer-apply-"));
  const { adapter } = createFaux([stopScript("done")]);
  const missing = capture();
  const missingCode = await runCli(
    ["--print", "hi", "--scorer-apply", "entry"],
    {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: missing.stdout,
      stderr: missing.stderr,
      adapter: adapter as ProviderAdapter,
    },
  );
  expect(missingCode).toBe(2);
  expect(missing.text().stderr).toContain("--scorer model|jev");

  const rule = capture();
  const ruleCode = await runCli(
    ["--print", "hi", "--scorer", "rule", "--scorer-apply", "entry"],
    {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: rule.stdout,
      stderr: rule.stderr,
      adapter: adapter as ProviderAdapter,
    },
  );
  expect(ruleCode).toBe(2);
  expect(rule.text().stderr).toContain("--scorer model|jev");
});

it("bench run forwards --scorer-apply and records it", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-scorer-apply-bench-"));
  const tasks = path.join(dir, "tasks.json");
  writeFileSync(
    tasks,
    JSON.stringify([
      {
        id: "one",
        repo: "owner/repo",
        baseCommit: "abc",
        problem: "fix",
        testCommand: "true",
        timeoutSec: 30,
      },
    ]),
  );
  const seen: string[][] = [];
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
      "--scorer",
      "model",
      "--scorer-apply",
      "entry",
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
          seen.push(opts.argv);
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
  expect(seen[0]).toEqual(
    expect.arrayContaining(["--scorer", "model", "--scorer-apply", "entry"]),
  );
  const results = JSON.parse(
    readFileSync(path.join(dir, "out", "results.json"), "utf8"),
  ) as { scorerApply?: string };
  expect(results.scorerApply).toBe("entry");
});

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
