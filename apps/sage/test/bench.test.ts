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
import { isRecord, JsonlLogStore } from "@sage/core";
import { afterEach, expect, it } from "vitest";
import { sageBin } from "../src/bench/deps.ts";
import { defaultPrepare } from "../src/bench/git.ts";
import { runCli } from "../src/main.ts";

const repoRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../..",
);

let dir = "";

it("spawns task children from the package bin found above the module", () => {
  expect(sageBin()).toBe(
    path.join(repoRoot, "apps", "sage", "bin", "sage.mjs"),
  );
});

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

it("bench report compares rates, the three numbers, and the summed failed scorer requests", async () => {
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
      row("t1", "resolved", {
        total: 4,
        cacheHitRatio: 0.25,
        expandRate: 0.5,
        failed: { rebuild: 2, entry: 1, shadow: 0 },
      }),
      row("t2", "resolved", {
        total: 6,
        cacheHitRatio: 0.75,
        expandRate: 0.5,
        failed: { rebuild: 0, entry: 0, shadow: 3 },
      }),
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
    stoppedEarlyRate: 0,
    resolvedRate: 0.5,
    emptyPatchRate: 0,
    errorRate: 0.5,
    faultRate: 0,
    tokensPerResolved: 10,
    cacheHitRatio: 0.5,
    expandRate: 0.25,
    scorerFailedRequests: 0,
    budget: null,
    baseUrl: null,
    provider: null,
  });
  expect(json.b).toEqual({
    stoppedEarlyRate: 0,
    resolvedRate: 1,
    emptyPatchRate: 0,
    errorRate: 0,
    faultRate: 0,
    tokensPerResolved: 5,
    cacheHitRatio: 0.5,
    expandRate: 0.5,
    scorerFailedRequests: 6,
    budget: null,
    baseUrl: null,
    provider: null,
  });
  expect(json.delta).toEqual({
    stoppedEarlyRate: 0,
    resolvedRate: 0.5,
    emptyPatchRate: 0,
    errorRate: -0.5,
    faultRate: 0,
    tokensPerResolved: -5,
    cacheHitRatio: 0,
    expandRate: 0.25,
    scorerFailedRequests: 6,
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

it("bench run forwards --base-url and --api-key-env and records the provider, url, and variable name, not the key", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-gateway-"));
  const tasks = path.join(dir, "tasks.json");
  writeFileSync(tasks, JSON.stringify([task("one")]));
  const gatewayOut = path.join(dir, "gateway");
  const plainOut = path.join(dir, "plain");
  const gatewayArgv: string[][] = [];
  const plainArgv: string[][] = [];
  const gateway = await runCli(
    [
      "bench",
      "run",
      "--tasks",
      tasks,
      "--policy",
      "compiler",
      "--provider",
      "moonshot",
      "--model",
      "kimi-test",
      "--base-url",
      "https://gateway.example.test/v1",
      "--api-key-env",
      "GATEWAY_KEY",
      "--out",
      gatewayOut,
    ],
    {
      cwd: dir,
      env: { GATEWAY_KEY: "SECRET_GATEWAY_KEY_VALUE" },
      stdout: capture().stdout,
      stderr: capture().stderr,
      benchDeps: recordingDeps(gatewayArgv),
    },
  );
  const plain = await runCli(
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
      plainOut,
    ],
    {
      cwd: dir,
      env: { ANTHROPIC_API_KEY: "test-key" },
      stdout: capture().stdout,
      stderr: capture().stderr,
      benchDeps: recordingDeps(plainArgv),
    },
  );
  expect(gateway).toBe(0);
  expect(plain).toBe(0);
  const forwarded = gatewayArgv[0] ?? [];
  expect(valueAfter(forwarded, "--provider")).toBe("moonshot");
  expect(valueAfter(forwarded, "--base-url")).toBe(
    "https://gateway.example.test/v1",
  );
  expect(valueAfter(forwarded, "--api-key-env")).toBe("GATEWAY_KEY");
  expect(plainArgv[0]).not.toContain("--provider");
  expect(plainArgv[0]).not.toContain("--base-url");
  expect(plainArgv[0]).not.toContain("--api-key-env");
  const gatewayText = readFileSync(
    path.join(gatewayOut, "results.json"),
    "utf8",
  );
  expect(gatewayText).not.toContain("SECRET_GATEWAY_KEY_VALUE");
  const gatewayResults: unknown = JSON.parse(gatewayText);
  expect(gatewayResults).toMatchObject({
    provider: "moonshot",
    baseUrl: "https://gateway.example.test/v1",
    apiKeyEnv: "GATEWAY_KEY",
  });
  const plainResults: unknown = JSON.parse(
    readFileSync(path.join(plainOut, "results.json"), "utf8"),
  );
  expect(plainResults).toMatchObject({
    provider: "anthropic",
    baseUrl: null,
    apiKeyEnv: null,
  });
  const report = capture();
  const reportCode = await runCli(["bench", "report", plainOut, gatewayOut], {
    cwd: dir,
    env: {},
    stdout: report.stdout,
    stderr: report.stderr,
  });
  expect(reportCode).toBe(0);
  expect(JSON.parse(report.text().stdout)).toMatchObject({
    a: { provider: "anthropic", baseUrl: null },
    b: { provider: "moonshot", baseUrl: "https://gateway.example.test/v1" },
  });
  expect(report.text().stdout).not.toContain("SECRET_GATEWAY_KEY_VALUE");
});

it("bench run checks the --api-key-env variable before reading tasks, not the preset default", async () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-gateway-key-"));
  const io = capture();
  const code = await runCli(
    [
      "bench",
      "run",
      "--tasks",
      path.join(dir, "absent.json"),
      "--policy",
      "compiler",
      "--provider",
      "moonshot",
      "--model",
      "kimi-test",
      "--api-key-env",
      "GATEWAY_KEY",
      "--out",
      path.join(dir, "out"),
    ],
    {
      cwd: dir,
      env: { MOONSHOT_API_KEY: "preset-key" },
      stdout: io.stdout,
      stderr: io.stderr,
    },
  );
  expect(code).toBe(2);
  expect(io.text().stderr).toBe("Missing GATEWAY_KEY\n");
  expect(existsSync(path.join(dir, "out", "results.json"))).toBe(false);
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
  const tasks: unknown = JSON.parse(readFileSync(out, "utf8"));
  if (!Array.isArray(tasks)) throw new Error("tasks file is not an array");
  expect(tasks.map(taskIdentity)).toEqual([
    { id: "tj__commander.js-7", repo: "tj/commander.js", baseCommit: "abc123" },
  ]);
});

it.each([
  ["a base without sha", { base: {} }],
  ["resolved_issues that is not an array", { resolved_issues: "the bug" }],
])(
  "fetch-multi-swe-ts rejects a dataset row with %s by its file and line",
  (_label, broken) => {
    dir = mkdtempSync(path.join(tmpdir(), "sage-bench-"));
    const fixture = path.join(dir, "fixture");
    mkdirSync(fixture);
    const good = {
      org: "tj",
      repo: "commander.js",
      number: 7,
      base: { sha: "abc123" },
      resolved_issues: [{ title: "the bug", body: "details" }],
      test_patch:
        "diff --git a/tests/x.test.js b/tests/x.test.js\n--- a/tests/x.test.js\n+++ b/tests/x.test.js\n",
    };
    writeFileSync(
      path.join(fixture, "tj__commander.js_dataset.jsonl"),
      `${JSON.stringify(good)}\n${JSON.stringify({ ...good, number: 8, ...broken })}\n`,
    );
    const result = runMultiSweTs([
      "--fixture",
      fixture,
      "--only",
      "tj__commander.js-7",
      "--out",
      path.join(dir, "tasks.json"),
    ]);
    expect(result.status).toBe(1);
    expect(result.stderr).toBe(
      "tj__commander.js_dataset.jsonl:2: malformed dataset row\n",
    );
  },
);

it("fetch-multi-swe-ts reports a repo without config before reading its rows", () => {
  dir = mkdtempSync(path.join(tmpdir(), "sage-bench-"));
  const result = runMultiSweTs([
    "--fixture",
    dir,
    "--only",
    "nobody__nothing-1",
    "--out",
    path.join(dir, "tasks.json"),
  ]);
  expect(result.status).toBe(1);
  expect(result.stderr).toBe("no repo config for nobody/nothing\n");
});

it("fetch-multi-swe-ts prints the argument parser's reason under the usage line", () => {
  const result = runMultiSweTs(["--bogus"]);
  expect(result.status).toBe(1);
  const lines = result.stderr.trimEnd().split("\n");
  expect(lines).toHaveLength(2);
  expect(lines[0]).toMatch(
    /^usage: node bench\/tasks\/fetch-multi-swe-ts\.mjs /,
  );
  expect(lines[1]).toContain("Unknown option '--bogus'");
});

function runMultiSweTs(args: string[]): {
  status: number | null;
  stderr: string;
} {
  const result = spawnSync(
    process.execPath,
    [path.join(repoRoot, "bench/tasks/fetch-multi-swe-ts.mjs"), ...args],
    { encoding: "utf8" },
  );
  return { status: result.status, stderr: result.stderr };
}

function taskIdentity(value: unknown): {
  id: unknown;
  repo: unknown;
  baseCommit: unknown;
} {
  if (!isRecord(value)) throw new Error("task is not an object");
  return { id: value.id, repo: value.repo, baseCommit: value.baseCommit };
}

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
      stoppedEarly: null,
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
      stoppedEarly: null,
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
  scorerFailedRequests: number | null;
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
  health: {
    total: number;
    cacheHitRatio: number;
    expandRate: number;
    /** Omitted to mimic a results file written before these counts existed. */
    failed?: { rebuild: number; entry: number; shadow: number };
  } | null,
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
          ...(health.failed
            ? {
                scorerFailedRequests: health.failed.rebuild,
                entryScored: { failedRequests: health.failed.entry },
                shadow: { failedRequests: health.failed.shadow },
              }
            : {}),
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

function recordingDeps(seen: string[][]) {
  return {
    async prepare() {},
    async runSage(spec: { id: string }, cwd: string, opts: object) {
      seen.push(flagArgv(opts));
      return { code: 0, sessionFile: writeSession(cwd, spec.id) };
    },
    async diffEmpty() {
      return false;
    },
    async runTests() {
      return { code: 0 };
    },
  };
}

function valueAfter(argv: string[], flag: string): string | undefined {
  const index = argv.indexOf(flag);
  return index >= 0 ? argv[index + 1] : undefined;
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
