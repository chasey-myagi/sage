import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { JsonlLogStore } from "@sage/core";
import { afterEach, expect, it } from "vitest";
import { runCli } from "../src/main.ts";

const repoRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../..",
);

let dir = "";

afterEach(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
  dir = "";
});

it("bench run exits 2 when no API key is set", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-"));
  const io = capture();
  const code = await runCli(
    [
      "bench",
      "run",
      "--tasks",
      "bench/tasks/mini.json",
      "--policy",
      "passthrough",
      "--model",
      "claude-test",
      "--out",
      path.join(dir, "out"),
    ],
    {
      cwd: dir,
      env: {},
      stdout: io.stdout,
      stderr: io.stderr,
    },
  );
  expect(code).toBe(2);
  expect(io.text().stderr).toContain("Missing");
  expect(existsSync(path.join(dir, "out", "results.json"))).toBe(false);
});

it("bench run records resolved, empty-patch, and error without the problem text", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-"));
  const tasks = path.join(dir, "tasks.json");
  writeFileSync(
    tasks,
    JSON.stringify([task("ok"), task("blank"), task("bad")]),
  );
  const out = path.join(dir, "out");
  const tested: string[] = [];
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
      out,
    ],
    {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: io.stdout,
      stderr: io.stderr,
      benchDeps: {
        async prepare() {},
        async runSage(spec, cwd) {
          return { code: 0, sessionFile: writeSession(cwd, spec.id) };
        },
        async diffEmpty(spec) {
          return spec.id === "blank";
        },
        async runTests(spec) {
          tested.push(spec.id);
          return { code: spec.id === "bad" ? 1 : 0 };
        },
      },
    },
  );
  expect(code).toBe(0);
  expect(tested).toEqual(["ok", "bad"]);
  const results = JSON.parse(
    readFileSync(path.join(out, "results.json"), "utf8"),
  ) as {
    policy: string;
    model: string;
    tasks: {
      id: string;
      status: string;
      health: { tokens: { total: number } } | null;
    }[];
  };
  expect(results.policy).toBe("compiler");
  expect(results.model).toBe("claude-test");
  expect(results.tasks.map((item) => [item.id, item.status])).toEqual([
    ["ok", "resolved"],
    ["blank", "empty-patch"],
    ["bad", "error"],
  ]);
  expect(results.tasks[0]?.health?.tokens.total).toBe(6);
  expect(results.tasks[1]?.health?.tokens.total).toBe(6);
  const rendered = readFileSync(path.join(out, "results.json"), "utf8");
  expect(rendered).not.toContain("LEAKED_PROBLEM_TEXT");
  for (const id of ["ok", "blank", "bad"]) {
    expect(existsSync(path.join(out, "sessions", `${id}.jsonl`))).toBe(true);
  }
  const session = readFileSync(path.join(out, "sessions", "ok.jsonl"), "utf8");
  expect(session).not.toContain("LEAKED_PROBLEM_TEXT");
});

it("bench report compares rates and the three numbers", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-"));
  const runA = path.join(dir, "a");
  const runB = path.join(dir, "b");
  writeResults(runA, {
    policy: "passthrough",
    model: "claude-test",
    problem: "LEAKED_PROBLEM_TEXT",
    tasks: [
      row("t1", "resolved", {
        total: 10,
        cacheHitRatio: 0.5,
        expandRate: 0.25,
      }),
      row("t2", "error", null),
    ],
  });
  writeResults(runB, {
    policy: "compiler",
    model: "claude-test",
    tasks: [
      row("t1", "resolved", { total: 4, cacheHitRatio: 0.25, expandRate: 0.5 }),
      row("t2", "resolved", { total: 6, cacheHitRatio: 0.75, expandRate: 0.5 }),
    ],
  });
  const io = capture();
  const code = await runCli(["bench", "report", runA, runB], {
    cwd: dir,
    env: {},
    stdout: io.stdout,
    stderr: io.stderr,
  });
  expect(code).toBe(0);
  const text = io.text().stdout;
  const json = JSON.parse(text) as {
    a: Metrics;
    b: Metrics;
    delta: Metrics;
  };
  expect(json.a).toEqual({
    resolvedRate: 0.5,
    emptyPatchRate: 0,
    errorRate: 0.5,
    tokensPerResolved: 10,
    cacheHitRatio: 0.5,
    expandRate: 0.25,
  });
  expect(json.b).toEqual({
    resolvedRate: 1,
    emptyPatchRate: 0,
    errorRate: 0,
    tokensPerResolved: 5,
    cacheHitRatio: 0.5,
    expandRate: 0.5,
  });
  expect(json.delta).toEqual({
    resolvedRate: 0.5,
    emptyPatchRate: 0,
    errorRate: -0.5,
    tokensPerResolved: -5,
    cacheHitRatio: 0,
    expandRate: 0.25,
  });
  expect(text).not.toContain("LEAKED_PROBLEM_TEXT");
});

it("fetch-swe-verified writes pytest commands from a fixture payload", () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-"));
  const fixture = path.join(dir, "hf.json");
  const out = path.join(dir, "tasks.json");
  writeFileSync(
    fixture,
    JSON.stringify({
      rows: [
        {
          row: {
            instance_id: "repo__repo-1",
            repo: "owner/repo",
            base_commit: "abc123",
            problem_statement: "the bug",
            FAIL_TO_PASS: JSON.stringify([
              "tests/test_a.py::test_one",
              "tests/test_b.py::test_two",
            ]),
          },
        },
        {
          row: {
            instance_id: "repo__repo-2",
            repo: "owner/repo",
            base_commit: "def456",
            problem_statement: "another",
            FAIL_TO_PASS: ["tests/test_c.py::test_three"],
          },
        },
      ],
    }),
  );
  const result = spawnSync(
    process.execPath,
    [
      path.join(repoRoot, "bench/tasks/fetch-swe-verified.mjs"),
      "--fixture",
      fixture,
      "--out",
      out,
    ],
    { encoding: "utf8" },
  );
  expect(result.status).toBe(0);
  const limited = spawnSync(
    process.execPath,
    [
      path.join(repoRoot, "bench/tasks/fetch-swe-verified.mjs"),
      "--fixture",
      fixture,
      "--out",
      path.join(dir, "one.json"),
      "--limit",
      "1",
    ],
    { encoding: "utf8" },
  );
  expect(limited.status).toBe(0);
  const one = JSON.parse(readFileSync(path.join(dir, "one.json"), "utf8")) as {
    id: string;
  }[];
  expect(one.map((item) => item.id)).toEqual(["repo__repo-1"]);
  expect(JSON.parse(readFileSync(out, "utf8"))).toEqual([
    {
      id: "repo__repo-1",
      repo: "owner/repo",
      baseCommit: "abc123",
      problem: "the bug",
      testCommand:
        "python -m pytest tests/test_a.py::test_one tests/test_b.py::test_two",
      timeoutSec: 1800,
    },
    {
      id: "repo__repo-2",
      repo: "owner/repo",
      baseCommit: "def456",
      problem: "another",
      testCommand: "python -m pytest tests/test_c.py::test_three",
      timeoutSec: 1800,
    },
  ]);
});

type Metrics = {
  resolvedRate: number | null;
  emptyPatchRate: number | null;
  errorRate: number | null;
  tokensPerResolved: number | null;
  cacheHitRatio: number | null;
  expandRate: number | null;
};

function task(id: string) {
  return {
    id,
    repo: "owner/repo",
    baseCommit: "abc",
    problem: "LEAKED_PROBLEM_TEXT",
    testCommand: "python -m pytest tests/test_example.py",
    timeoutSec: 30,
  };
}

function writeSession(cwd: string, id: string): string {
  const sessions = path.join(cwd, ".sage", "sessions");
  const store = new JsonlLogStore({ dir: sessions });
  store.append(id, {
    kind: "message",
    payload: {
      role: "user",
      content: [{ type: "text", text: "task" }],
    },
  });
  store.append(id, {
    kind: "message",
    payload: {
      role: "assistant",
      content: [{ type: "text", text: "done" }],
    },
    details: {
      usage: {
        inputOther: 3,
        inputCacheRead: 1,
        inputCacheCreation: 0,
        output: 2,
      },
    },
  });
  store.append(id, {
    kind: "manifest",
    payload: {
      epoch: 0,
      turn: 1,
      budget: { total: 10, used: 2, overrun: 0, omitted: 4, expands: 1 },
      claims: [],
      chunks: [],
      anomalies: [],
    },
  });
  return path.join(sessions, `${id}.jsonl`);
}

function row(
  id: string,
  status: "resolved" | "empty-patch" | "error",
  health: { total: number; cacheHitRatio: number; expandRate: number } | null,
) {
  return {
    id,
    status,
    health: health
      ? {
          turns: 1,
          tokens: {
            input: health.total,
            cacheRead: 0,
            cacheCreation: 0,
            output: 0,
            total: health.total,
          },
          cacheHitRatio: health.cacheHitRatio,
          expandRate: health.expandRate,
          omitted: 0,
          expands: 0,
          epochs: 1,
          modelChanges: 0,
          scorerCalls: 0,
          scorerFallbacks: 0,
        }
      : null,
  };
}

function writeResults(
  runDir: string,
  spec: {
    policy: string;
    model: string;
    problem?: string;
    tasks: ReturnType<typeof row>[];
  },
): void {
  mkdirSync(runDir, { recursive: true });
  writeFileSync(
    path.join(runDir, "results.json"),
    JSON.stringify({
      policy: spec.policy,
      model: spec.model,
      problem: spec.problem,
      tasks: spec.tasks,
    }),
  );
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
