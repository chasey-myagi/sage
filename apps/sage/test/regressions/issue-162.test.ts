import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { JsonlLogStore } from "@sage/core";
import { afterEach, expect, it } from "vitest";
import { runCli } from "../../src/main.ts";

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

type FakeRun = {
  /** Terminal outcome written to the child's session. */
  outcome: string;
  /** `details.finish` of the last assistant message. */
  finish: Record<string, unknown>;
  /** Whether the child left a change in the work tree. */
  edited: boolean;
};

const UNAUTHORIZED = {
  kind: "error",
  error: { message: "invalid api key", status: 401 },
};

it("grades a run whose first provider call failed as a session error, not an empty patch", async () => {
  const { rows, tested } = await runBench({
    "auth-failed": { outcome: "error", finish: UNAUTHORIZED, edited: false },
  });
  expect(rows).toEqual([
    {
      id: "auth-failed",
      status: "error",
      reason: "session-error",
      providerError: { status: 401, transport: false },
      health: expect.any(Object),
      stoppedEarly: null,
    },
  ]);
  expect(tested).toEqual([]);
});

it("carries a transport failure's classification without a status", async () => {
  const { rows } = await runBench({
    unreachable: {
      outcome: "error",
      finish: {
        kind: "error",
        error: { message: "fetch failed", transport: true },
      },
      edited: false,
    },
  });
  expect(rows[0]).toMatchObject({
    reason: "session-error",
    providerError: { status: null, transport: true },
  });
});

it("records a harness error with no provider failure as a session error with a null provider error", async () => {
  const { rows } = await runBench({
    "seam-threw": {
      outcome: "error",
      finish: { kind: "tool_use" },
      edited: false,
    },
  });
  expect(rows[0]).toMatchObject({
    status: "error",
    reason: "session-error",
    providerError: null,
  });
});

it("does not run tests on a patch left by a run that ended on a provider error", async () => {
  const { rows, tested } = await runBench({
    "failed-after-edit": {
      outcome: "error",
      finish: UNAUTHORIZED,
      edited: true,
    },
  });
  expect(rows[0]).toMatchObject({
    status: "error",
    reason: "session-error",
    stoppedEarly: null,
  });
  expect(tested).toEqual([]);
});

it("grades an aborted run as a session abort", async () => {
  const { rows, tested } = await runBench({
    interrupted: {
      outcome: "aborted",
      finish: { kind: "aborted" },
      edited: true,
    },
  });
  expect(rows[0]).toMatchObject({
    status: "error",
    reason: "session-aborted",
  });
  expect(rows[0]).not.toHaveProperty("providerError");
  expect(tested).toEqual([]);
});

it("keeps grading runs that hit the turn cap by their diff and tests", async () => {
  const { rows, tested } = await runBench({
    "capped-empty": {
      outcome: "max_turns",
      finish: { kind: "tool_use" },
      edited: false,
    },
    "capped-edited": {
      outcome: "max_turns",
      finish: { kind: "tool_use" },
      edited: true,
    },
  });
  expect(rows.map((row) => [row.id, row.status])).toEqual([
    ["capped-empty", "empty-patch"],
    ["capped-edited", "resolved"],
  ]);
  expect(rows.some((row) => "providerError" in row)).toBe(false);
  expect(tested).toEqual(["capped-edited"]);
});

/**
 * Runs `sage bench run` with every side effect faked: the child writes a
 * session that ends as `runs[id]` says and reports whether it edited.
 */
async function runBench(runs: Record<string, FakeRun>): Promise<{
  rows: Record<string, unknown>[];
  tested: string[];
}> {
  dir = mkdtempSync(path.join(tmpdir(), "sage-issue-162-"));
  const tasks = path.join(dir, "tasks.json");
  writeFileSync(tasks, JSON.stringify(Object.keys(runs).map(task)));
  const out = path.join(dir, "out");
  const tested: string[] = [];
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
        async prepare() {},
        async runSage(spec, cwd) {
          const run = runs[spec.id];
          if (!run) throw new Error(`no fake run for ${spec.id}`);
          return { code: 1, sessionFile: writeSession(cwd, spec.id, run) };
        },
        async diffEmpty(spec) {
          return runs[spec.id]?.edited !== true;
        },
        async runTests(spec) {
          tested.push(spec.id);
          return { code: 0 };
        },
      },
    },
  );
  expect(code).toBe(0);
  return { rows: rowsOf(readResults(out)), tested };
}

function writeSession(cwd: string, id: string, run: FakeRun): string {
  const sessions = path.join(cwd, ".sage", "sessions");
  const store = new JsonlLogStore({ dir: sessions });
  store.append(id, {
    kind: "message",
    payload: { role: "user", content: [{ type: "text", text: "task" }] },
  });
  store.append(id, {
    kind: "message",
    payload: { role: "assistant", content: [] },
    details: {
      usage: {
        inputOther: 0,
        inputCacheRead: 0,
        inputCacheCreation: 0,
        output: 0,
      },
      finish: run.finish,
    },
  });
  store.append(id, { kind: "terminal", payload: { outcome: run.outcome } });
  return path.join(sessions, `${id}.jsonl`);
}

function readResults(out: string): unknown {
  return JSON.parse(readFileSync(path.join(out, "results.json"), "utf8"));
}

function rowsOf(value: unknown): Record<string, unknown>[] {
  if (typeof value !== "object" || value === null || !("tasks" in value)) {
    throw new Error("results.json has no tasks");
  }
  const { tasks } = value;
  if (!Array.isArray(tasks)) throw new Error("tasks is not an array");
  return tasks.map((item: unknown) => {
    if (typeof item !== "object" || item === null || Array.isArray(item)) {
      throw new Error("malformed task result");
    }
    return { ...item };
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
