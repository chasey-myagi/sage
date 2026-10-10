import { execFileSync } from "node:child_process";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, expect, it } from "vitest";
import { defaultDeps } from "../../src/bench/deps.ts";
import { runCli } from "../../src/main.ts";

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

it("grades a work tree whose only changes are sage's own runtime files as an empty patch", async () => {
  const statuses = await runBench({
    "only-session": () => {},
    "spilled-output": (cwd) => {
      const spill = path.join(cwd, ".sage", "tool-output");
      mkdirSync(spill, { recursive: true });
      writeFileSync(path.join(spill, "call-1.txt"), "long output\n");
    },
    "edited-tracked": (cwd) => writeFileSync(path.join(cwd, "a.ts"), "2\n"),
    "added-untracked": (cwd) => writeFileSync(path.join(cwd, "b.ts"), "1\n"),
  });
  expect(statuses).toEqual([
    ["only-session", "empty-patch"],
    ["spilled-output", "empty-patch"],
    ["edited-tracked", "resolved"],
    ["added-untracked", "resolved"],
  ]);
});

/**
 * Runs `sage bench run` with the production `diffEmpty` against a real git
 * work tree; the faked child writes a session log plus whatever `edits` does.
 */
async function runBench(
  edits: Record<string, (cwd: string) => void>,
): Promise<[string, string][]> {
  dir = mkdtempSync(path.join(tmpdir(), "sage-issue-131-"));
  const tasks = path.join(dir, "tasks.json");
  writeFileSync(tasks, JSON.stringify(Object.keys(edits).map(task)));
  const out = path.join(dir, "out");
  const code = await runCli(
    [
      "bench",
      "run",
      "--tasks",
      tasks,
      "--policy",
      "passthrough",
      "--model",
      "claude-test",
      "--out",
      out,
    ],
    {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: { write() {} },
      stderr: { write() {} },
      benchDeps: {
        ...defaultDeps(),
        async prepare(_spec, cwd) {
          initRepo(cwd);
        },
        async runSage(spec, cwd) {
          const sessions = path.join(cwd, ".sage", "sessions");
          mkdirSync(sessions, { recursive: true });
          writeFileSync(path.join(sessions, "child.jsonl"), "");
          edits[spec.id]?.(cwd);
          return { code: 0 };
        },
        async runTests() {
          return { code: 0 };
        },
      },
    },
  );
  expect(code).toBe(0);
  const parsed: unknown = JSON.parse(
    readFileSync(path.join(out, "results.json"), "utf8"),
  );
  return statusesOf(parsed);
}

function initRepo(cwd: string): void {
  mkdirSync(cwd, { recursive: true });
  const git = (...args: string[]) =>
    execFileSync("git", ["-C", cwd, ...args], { stdio: "ignore" });
  git("init", "-q");
  writeFileSync(path.join(cwd, "a.ts"), "1\n");
  git("add", ".");
  git(
    "-c",
    "user.name=test",
    "-c",
    "user.email=test@example.com",
    "commit",
    "-qm",
    "base",
  );
}

function statusesOf(value: unknown): [string, string][] {
  if (typeof value !== "object" || value === null || !("tasks" in value)) {
    throw new Error("results.json has no tasks");
  }
  const { tasks } = value;
  if (!Array.isArray(tasks)) throw new Error("tasks is not an array");
  return tasks.map((item: unknown): [string, string] => {
    if (
      typeof item !== "object" ||
      item === null ||
      !("id" in item) ||
      !("status" in item) ||
      typeof item.id !== "string" ||
      typeof item.status !== "string"
    ) {
      throw new Error("malformed task result");
    }
    return [item.id, item.status];
  });
}

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
