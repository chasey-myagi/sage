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
import { fileURLToPath, pathToFileURL } from "node:url";
import { JsonlLogStore } from "@sage/core";
import { afterEach, expect, it } from "vitest";
import { defaultPrepare } from "../src/bench/git.ts";
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
    budget: null,
  });
  expect(json.b).toEqual({
    resolvedRate: 1,
    emptyPatchRate: 0,
    errorRate: 0,
    tokensPerResolved: 5,
    cacheHitRatio: 0.5,
    expandRate: 0.5,
    budget: null,
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

it("bench run --budget 6000 forwards the flag and records it unless passthrough", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-budget-"));
  const tasks = path.join(dir, "tasks.json");
  writeFileSync(tasks, JSON.stringify([task("one")]));
  const compilerOut = path.join(dir, "compiler");
  const passthroughOut = path.join(dir, "passthrough");
  const compilerArgv: string[][] = [];
  const passthroughArgv: string[][] = [];
  const compiler = await runCli(
    [
      "bench",
      "run",
      "--tasks",
      tasks,
      "--policy",
      "compiler",
      "--model",
      "claude-test",
      "--budget",
      "6000",
      "--out",
      compilerOut,
    ],
    {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: capture().stdout,
      stderr: capture().stderr,
      benchDeps: {
        async prepare() {},
        async runSage(_spec, cwd, opts) {
          compilerArgv.push(flagArgv(opts));
          return { code: 0, sessionFile: writeSession(cwd, _spec.id) };
        },
        async diffEmpty() {
          return false;
        },
        async runTests() {
          return { code: 0 };
        },
      },
    },
  );
  const passthrough = await runCli(
    [
      "bench",
      "run",
      "--tasks",
      tasks,
      "--policy",
      "passthrough",
      "--model",
      "claude-test",
      "--budget",
      "6000",
      "--out",
      passthroughOut,
    ],
    {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: capture().stdout,
      stderr: capture().stderr,
      benchDeps: {
        async prepare() {},
        async runSage(_spec, cwd, opts) {
          passthroughArgv.push(flagArgv(opts));
          return { code: 0, sessionFile: writeSession(cwd, _spec.id) };
        },
        async diffEmpty() {
          return false;
        },
        async runTests() {
          return { code: 0 };
        },
      },
    },
  );
  expect(compiler).toBe(0);
  expect(passthrough).toBe(0);
  expect(compilerArgv[0]).toEqual(expect.arrayContaining(["--budget", "6000"]));
  expect(adjacentBudget(compilerArgv[0] ?? [])).toBe(true);
  expect(adjacentBudget(passthroughArgv[0] ?? [])).toBe(true);
  expect(readBudget(compilerOut)).toBe(6000);
  expect(readBudget(passthroughOut)).toBe(null);
  const report = capture();
  const reportCode = await runCli(
    ["bench", "report", compilerOut, passthroughOut],
    {
      cwd: dir,
      env: {},
      stdout: report.stdout,
      stderr: report.stderr,
    },
  );
  expect(reportCode).toBe(0);
  const json = JSON.parse(report.text().stdout) as {
    a: { budget?: number | null };
    b: { budget?: number | null };
  };
  expect(json.a.budget).toBe(6000);
  expect(json.b.budget).toBe(null);
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

it("fetch-multi-swe-ts matches ts-mini.json byte for byte from a fixture", () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-"));
  const out = path.join(dir, "tasks.json");
  const result = spawnSync(
    process.execPath,
    [
      path.join(repoRoot, "bench/tasks/fetch-multi-swe-ts.mjs"),
      "--fixture",
      path.join(repoRoot, "apps/sage/test/fixtures/multi-swe-ts"),
      "--out",
      out,
    ],
    { encoding: "utf8" },
  );
  expect(result.status).toBe(0);
  const expected = readFileSync(
    path.join(repoRoot, "bench/tasks/ts-mini.json"),
  );
  expect(Buffer.compare(expected, readFileSync(out))).toBe(0);
});

it("fetch-multi-swe-ts --set ts-mini-2 matches ts-mini-2.json byte for byte from a fixture", () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-"));
  const out = path.join(dir, "tasks.json");
  const result = spawnSync(
    process.execPath,
    [
      path.join(repoRoot, "bench/tasks/fetch-multi-swe-ts.mjs"),
      "--set",
      "ts-mini-2",
      "--fixture",
      path.join(repoRoot, "apps/sage/test/fixtures/multi-swe-ts"),
      "--out",
      out,
    ],
    { encoding: "utf8" },
  );
  expect(result.stderr).toBe("");
  expect(result.status).toBe(0);
  const expected = readFileSync(
    path.join(repoRoot, "bench/tasks/ts-mini-2.json"),
  );
  expect(Buffer.compare(expected, readFileSync(out))).toBe(0);
});

it("fetch-multi-swe-ts names a row without instance_id <org>__<repo>-<number>", () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-"));
  const fixture = path.join(dir, "fixture");
  mkdirSync(fixture);
  writeFileSync(
    path.join(fixture, "tj__commander.js_dataset.jsonl"),
    `${JSON.stringify({
      org: "tj",
      repo: "commander.js",
      number: 7,
      base: { sha: "abc123" },
      resolved_issues: [{ title: "the bug", body: "details" }],
      test_patch:
        "diff --git a/tests/x.test.js b/tests/x.test.js\n--- a/tests/x.test.js\n+++ b/tests/x.test.js\n",
    })}\n`,
  );
  const out = path.join(dir, "tasks.json");
  const result = spawnSync(
    process.execPath,
    [
      path.join(repoRoot, "bench/tasks/fetch-multi-swe-ts.mjs"),
      "--set",
      "ts-mini-2",
      "--fixture",
      fixture,
      "--only",
      "tj__commander.js-7",
      "--out",
      out,
    ],
    { encoding: "utf8" },
  );
  expect(result.stderr).toBe("");
  expect(result.status).toBe(0);
  const tasks = JSON.parse(readFileSync(out, "utf8")) as {
    id: string;
    repo: string;
    baseCommit: string;
  }[];
  expect(
    tasks.map(({ id, repo, baseCommit }) => ({ id, repo, baseCommit })),
  ).toEqual([
    { id: "tj__commander.js-7", repo: "tj/commander.js", baseCommit: "abc123" },
  ]);
});

it("setupCommand failure is setup-failed and does not start the agent", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-"));
  const tasks = path.join(dir, "tasks.json");
  writeFileSync(
    tasks,
    JSON.stringify([
      {
        ...task("setup-bad"),
        setupCommand: "exit 1",
      },
    ]),
  );
  const out = path.join(dir, "out");
  const started: string[] = [];
  const tested: string[] = [];
  const io = capture();
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
      stdout: io.stdout,
      stderr: io.stderr,
      benchDeps: {
        async prepare() {},
        async runSage(spec) {
          started.push(spec.id);
          return { code: 0, sessionFile: writeSession(dir, spec.id) };
        },
        async diffEmpty() {
          return false;
        },
        async runTests(spec) {
          tested.push(spec.id);
          return { code: 0 };
        },
      },
    },
  );
  expect(code).toBe(0);
  expect(started).toEqual([]);
  expect(tested).toEqual([]);
  const results = JSON.parse(
    readFileSync(path.join(out, "results.json"), "utf8"),
  ) as {
    tasks: { id: string; status: string; reason?: string; health: unknown }[];
  };
  expect(results.tasks).toEqual([
    {
      id: "setup-bad",
      status: "error",
      reason: "setup-failed",
      health: null,
    },
  ]);
  expect(existsSync(path.join(out, "sessions", "setup-bad.jsonl"))).toBe(false);
});

it("testPatch overwrites agent edits to test files before tests run", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-"));
  const origin = path.join(dir, "origin");
  const repo = initFixtureRepo(origin);
  const tasks = path.join(dir, "tasks.json");
  const base = {
    repo: origin,
    baseCommit: repo.sha,
    problem: "fix the bug",
    timeoutSec: 30,
    setupCommand: "true",
  };
  writeFileSync(
    tasks,
    JSON.stringify([
      {
        ...base,
        id: "edited-test",
        testCommand: "node -e 'process.exit(0)'",
        testPatch: `${repo.patchExisting}${repo.patchNew}`,
      },
      {
        ...base,
        id: "edited-source",
        testCommand: "node -e 'process.exit(0)'",
        testPatch: repo.patchExisting,
      },
    ]),
  );
  const out = path.join(dir, "out");
  const seen = new Map<string, Record<string, string | null>>();
  const io = capture();
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
      stdout: io.stdout,
      stderr: io.stderr,
      benchDeps: {
        async prepare(_spec, cwd) {
          git(cwd, ["clone", origin, "."]);
        },
        async runSage(spec, cwd) {
          if (spec.id === "edited-test") {
            writeFileSync(path.join(cwd, "test/app.test.js"), "agent-test\n");
            writeFileSync(
              path.join(cwd, "test/extra.test.js"),
              "agent-extra\n",
            );
          }
          writeFileSync(path.join(cwd, "src/app.js"), "agent-source\n");
          return { code: 0, sessionFile: writeSession(cwd, spec.id) };
        },
        async diffEmpty(_spec, cwd) {
          const status = git(cwd, ["status", "--porcelain"]);
          return status.trim().length === 0;
        },
        async runTests(spec, cwd) {
          const extra = path.join(cwd, "test/extra.test.js");
          seen.set(spec.id, {
            test: readFileSync(path.join(cwd, "test/app.test.js"), "utf8"),
            extra: existsSync(extra) ? readFileSync(extra, "utf8") : null,
            source: readFileSync(path.join(cwd, "src/app.js"), "utf8"),
          });
          return { code: 0 };
        },
      },
    },
  );
  expect(code).toBe(0);
  expect(seen.get("edited-test")).toEqual({
    test: "test-hidden\n",
    extra: "extra-hidden\n",
    source: "agent-source\n",
  });
  expect(seen.get("edited-source")).toEqual({
    test: "test-hidden\n",
    extra: null,
    source: "agent-source\n",
  });
  const results = JSON.parse(
    readFileSync(path.join(out, "results.json"), "utf8"),
  ) as { tasks: { id: string; status: string }[] };
  expect(results.tasks.map((item) => [item.id, item.status])).toEqual([
    ["edited-test", "resolved"],
    ["edited-source", "resolved"],
  ]);
});

it("bench run keeps only baseCommit when the repo has a later commit", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-"));
  const origin = path.join(dir, "origin");
  const history = initHistoryRepo(origin);
  const tasks = path.join(dir, "tasks.json");
  writeFileSync(
    tasks,
    JSON.stringify([
      {
        id: "future",
        repo: pathToFileURL(origin).href,
        baseCommit: history.base,
        problem: "fix the bug",
        testCommand: "true",
        timeoutSec: 30,
      },
    ]),
  );
  const seen: { count: string; log: string; remote: string; tag: string } = {
    count: "",
    log: "",
    remote: "",
    tag: "",
  };
  let started = false;
  const out = path.join(dir, "out");
  const io = capture();
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
      stdout: io.stdout,
      stderr: io.stderr,
      benchDeps: {
        prepare: defaultPrepare,
        async runSage(_spec, cwd) {
          started = true;
          seen.count = git(cwd, ["rev-list", "--all", "--count"]).trim();
          seen.log = git(cwd, ["log", "--all", "--format=%H"]).trim();
          seen.remote = git(cwd, ["remote"]).trim();
          seen.tag = git(cwd, ["tag"]).trim();
          return { code: 0, sessionFile: writeSession(cwd, _spec.id) };
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
  expect(started).toBe(true);
  expect(seen).toEqual({
    count: "1",
    log: history.base,
    remote: "",
    tag: "",
  });
  expect(seen.log).not.toContain(history.later);
});

it("bench run records clone-failed and does not start the agent when fetch fails", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-"));
  const origin = path.join(dir, "origin");
  initHistoryRepo(origin);
  const tasks = path.join(dir, "tasks.json");
  writeFileSync(
    tasks,
    JSON.stringify([
      {
        id: "missing",
        repo: pathToFileURL(origin).href,
        baseCommit: "0000000000000000000000000000000000000000",
        problem: "fix the bug",
        testCommand: "true",
        timeoutSec: 30,
      },
    ]),
  );
  const started: string[] = [];
  const out = path.join(dir, "out");
  const io = capture();
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
      stdout: io.stdout,
      stderr: io.stderr,
      benchDeps: {
        prepare: defaultPrepare,
        async runSage(spec) {
          started.push(spec.id);
          return { code: 0 };
        },
        async diffEmpty() {
          return false;
        },
        async runTests() {
          return { code: 0 };
        },
      },
    },
  );
  expect(code).toBe(0);
  expect(started).toEqual([]);
  const results = JSON.parse(
    readFileSync(path.join(out, "results.json"), "utf8"),
  ) as {
    tasks: { id: string; status: string; reason?: string; health: unknown }[];
  };
  expect(results.tasks).toEqual([
    {
      id: "missing",
      status: "error",
      reason: "clone-failed",
      health: null,
    },
  ]);
  expect(existsSync(path.join(out, "sessions", "missing.jsonl"))).toBe(false);
});

type Metrics = {
  resolvedRate: number | null;
  emptyPatchRate: number | null;
  errorRate: number | null;
  tokensPerResolved: number | null;
  cacheHitRatio: number | null;
  expandRate: number | null;
};

function git(cwd: string, args: string[]): string {
  const result = spawnSync(
    "git",
    [
      "-c",
      "commit.gpgsign=false",
      "-c",
      "user.name=bench",
      "-c",
      "user.email=bench@example.com",
      ...args,
    ],
    { cwd, encoding: "utf8" },
  );
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || "git failed");
  }
  return result.stdout;
}

function initHistoryRepo(origin: string): { base: string; later: string } {
  mkdirSync(origin, { recursive: true });
  git(origin, ["init", "-b", "main"]);
  writeFileSync(path.join(origin, "README"), "base\n");
  git(origin, ["add", "."]);
  git(origin, ["commit", "-m", "base"]);
  const base = git(origin, ["rev-parse", "HEAD"]).trim();
  writeFileSync(path.join(origin, "README"), "later\n");
  git(origin, ["add", "."]);
  git(origin, ["commit", "-m", "later fix"]);
  const later = git(origin, ["rev-parse", "HEAD"]).trim();
  git(origin, ["tag", "v-future"]);
  return { base, later };
}

function initFixtureRepo(origin: string): {
  sha: string;
  patchExisting: string;
  patchNew: string;
} {
  mkdirSync(path.join(origin, "src"), { recursive: true });
  mkdirSync(path.join(origin, "test"), { recursive: true });
  git(origin, ["init", "-b", "main"]);
  writeFileSync(path.join(origin, "src/app.js"), "source-base\n");
  writeFileSync(path.join(origin, "test/app.test.js"), "test-base\n");
  git(origin, ["add", "."]);
  git(origin, ["commit", "-m", "base"]);
  const sha = git(origin, ["rev-parse", "HEAD"]).trim();
  writeFileSync(path.join(origin, "test/app.test.js"), "test-hidden\n");
  const patchExisting = git(origin, ["diff", "--", "test/app.test.js"]);
  git(origin, ["checkout", "--", "."]);
  writeFileSync(path.join(origin, "test/extra.test.js"), "extra-hidden\n");
  git(origin, ["add", "-N", "test/extra.test.js"]);
  const patchNew = git(origin, ["diff", "--", "test/extra.test.js"]);
  git(origin, ["reset", "-q"]);
  rmSync(path.join(origin, "test/extra.test.js"), { force: true });
  return { sha, patchExisting, patchNew };
}

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

function flagArgv(opts: object): string[] {
  if (!("argv" in opts) || !Array.isArray(opts.argv)) return [];
  return opts.argv.filter((item): item is string => typeof item === "string");
}

function adjacentBudget(argv: string[]): boolean {
  const index = argv.indexOf("--budget");
  return index >= 0 && argv[index + 1] === "6000";
}

function readBudget(out: string): unknown {
  const parsed: unknown = JSON.parse(
    readFileSync(path.join(out, "results.json"), "utf8"),
  );
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    return undefined;
  }
  return "budget" in parsed ? parsed.budget : undefined;
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
