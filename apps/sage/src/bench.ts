/**
 * Offline-triggerable benchmark runner. Not a CI gate.
 *
 * Contract: `bench run` reads a JSON array of
 * `{ id, repo, baseCommit, problem, testCommand, timeoutSec }` plus optional
 * `setupCommand` and `testPatch`. It exits 2
 * when the selected provider has no API key, before it reads the task file
 * or clones anything. With no `--provider`, any preset env key is enough,
 * in the same order and with the same missing-key message as `sage -p`.
 * Each task is cloned into a temp directory and checked out at `baseCommit`.
 * `setupCommand`, when present, runs with `sh -c` in that directory before
 * sage and under `timeoutSec`. A non-zero exit is `error` with reason
 * `setup-failed`: sage is not started and the test command is not run.
 * Then `sage -p <problem>` runs there. When `--budget <tokens>` is set, that
 * flag is appended to the argv, including under passthrough. The session is
 * copied next. An empty `git status` is `empty-patch` and the test command
 * is not run. `testPatch`,
 * when present, is a unified diff applied only after that copy and only when
 * status is non-empty. Each path in a `diff --git` header is restored from
 * `baseCommit` (`git checkout`, or `rm -f` when the blob is absent), then
 * `git apply` runs. Apply failure is `error` with reason `test-patch-failed`
 * and the test command is not run. The agent never sees `testPatch`. A test
 * command that exits 0 is `resolved`. Anything else is `error` with a short
 * reason (`clone-failed`, `sage-failed`, `diff-failed`, `tests-failed`,
 * `timeout`, `setup-failed`, `test-patch-failed`). `health` is `runHealth`
 * of the copied session, or null. The result directory is `<out>/results.json`
 * and `<out>/sessions/<taskId>.jsonl`. Those files hold ids, statuses,
 * reasons, and health numbers. They do not copy the problem text.
 * `results.json` also has top-level `budget`: the positive integer from
 * `--budget` for a compiler run, or null when the flag is omitted or the
 * policy is passthrough. Top-level `toolNetwork` is `none` unless
 * `--tool-network full` is set. The default `none` is always forwarded on
 * the sage argv. `bench report <runA> <runB>` prints one JSON
 * object `{ a, b, delta }`. `a` and `b` each include that `budget`.
 * Rates are counts over the task list. `tokensPerResolved` is the sum of
 * `health.tokens.total` divided by the resolved count, or null when nothing
 * resolved. `cacheHitRatio` and `expandRate` are the mean of the non-null
 * per-task values. `delta` is B minus A. A null side makes that delta null.
 * Repo names without a scheme clone as `https://github.com/<repo>.git`.
 *
 * Intentionally not in the Log: argv, API keys, clone directories, setup
 * commands, test patches, test output, and this report. The session jsonl is
 * the recorded run, copied before `testPatch` is applied. The report reads
 * health back off it.
 */

import { spawn } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { JsonlLogStore, type RunHealth, runHealth } from "@sage/core";
import { resolvePreset } from "@sage/provider";
import type { ToolNetwork } from "@sage/tools";

export type BenchPolicy = "passthrough" | "compiler";

export type BenchTask = {
  id: string;
  repo: string;
  baseCommit: string;
  problem: string;
  testCommand: string;
  timeoutSec: number;
  setupCommand?: string;
  testPatch?: string;
};

export type BenchStatus = "resolved" | "empty-patch" | "error";

export type BenchTaskResult = {
  id: string;
  status: BenchStatus;
  reason?: string;
  health: RunHealth | null;
};

export type BenchResults = {
  policy: BenchPolicy;
  model: string;
  budget: number | null;
  toolNetwork: ToolNetwork;
  tasks: BenchTaskResult[];
};

export type BenchMetrics = {
  resolvedRate: number | null;
  emptyPatchRate: number | null;
  errorRate: number | null;
  tokensPerResolved: number | null;
  cacheHitRatio: number | null;
  expandRate: number | null;
};

export type BenchDeps = {
  prepare(task: BenchTask, cwd: string): Promise<void>;
  runSage(
    task: BenchTask,
    cwd: string,
    opts: {
      policy: BenchPolicy;
      model: string;
      provider?: string;
      env: NodeJS.ProcessEnv;
      budget: number | null;
      toolNetwork: ToolNetwork;
      argv: string[];
    },
  ): Promise<{ code: number; sessionFile?: string; timedOut?: boolean }>;
  diffEmpty(task: BenchTask, cwd: string): Promise<boolean>;
  runTests(
    task: BenchTask,
    cwd: string,
  ): Promise<{ code: number; timedOut?: boolean }>;
};

type BenchIo = {
  env: NodeJS.ProcessEnv;
  cwd: string;
  stdout: { write(chunk: string): void };
  stderr: { write(chunk: string): void };
  deps?: BenchDeps;
};

const FLAGS = new Set([
  "tasks",
  "policy",
  "model",
  "provider",
  "out",
  "budget",
  "tool-network",
]);

export async function runBenchCli(
  argv: string[],
  io: BenchIo,
): Promise<number> {
  const command = argv[0];
  if (command === "report") return reportCommand(argv.slice(1), io);
  if (command !== "run") {
    io.stderr.write(`${benchUsage()}\n`);
    return 1;
  }
  const parsed = parseRun(argv.slice(1));
  if ("message" in parsed) {
    io.stderr.write(`${parsed.message}\n`);
    return parsed.exitCode ?? 1;
  }
  const missing = missingKey(parsed.provider, io.env);
  if (missing) {
    io.stderr.write(`${missing}\n`);
    return 2;
  }
  let tasks: BenchTask[];
  try {
    tasks = loadTasks(path.resolve(io.cwd, parsed.tasks));
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    io.stderr.write(`${message}\n`);
    return 1;
  }
  const out = path.resolve(io.cwd, parsed.out);
  const deps = io.deps ?? defaultDeps();
  const results: BenchResults = {
    policy: parsed.policy,
    model: parsed.model,
    budget: parsed.policy === "passthrough" ? null : parsed.budget,
    toolNetwork: parsed.toolNetwork,
    tasks: [],
  };
  for (const task of tasks) {
    results.tasks.push(
      await runTask(task, out, deps, {
        policy: parsed.policy,
        model: parsed.model,
        ...(parsed.provider ? { provider: parsed.provider } : {}),
        env: io.env,
        budget: parsed.budget,
        toolNetwork: parsed.toolNetwork,
      }),
    );
    writeResults(out, results);
  }
  writeResults(out, results);
  return 0;
}

export function loadTasks(file: string): BenchTask[] {
  const parsed: unknown = JSON.parse(readFileSync(file, "utf8"));
  if (!Array.isArray(parsed))
    throw new Error("tasks file must be a JSON array");
  return parsed.map((item, index) => parseTask(item, index));
}

export function benchMetrics(results: BenchResults): BenchMetrics {
  const tasks = results.tasks;
  const total = tasks.length;
  const resolved = tasks.filter((task) => task.status === "resolved").length;
  const empty = tasks.filter((task) => task.status === "empty-patch").length;
  const error = tasks.filter((task) => task.status === "error").length;
  const tokens = tasks.reduce((sum, task) => sum + tokenTotal(task), 0);
  return {
    resolvedRate: total === 0 ? null : resolved / total,
    emptyPatchRate: total === 0 ? null : empty / total,
    errorRate: total === 0 ? null : error / total,
    tokensPerResolved: resolved === 0 ? null : tokens / resolved,
    cacheHitRatio: mean(tasks.map((task) => task.health?.cacheHitRatio)),
    expandRate: mean(tasks.map((task) => task.health?.expandRate)),
  };
}

function reportCommand(argv: string[], io: BenchIo): number {
  if (argv.length !== 2 || argv.some((arg) => arg.startsWith("-"))) {
    io.stderr.write(`${benchUsage()}\n`);
    return 1;
  }
  const left = argv[0];
  const right = argv[1];
  if (!left || !right) {
    io.stderr.write(`${benchUsage()}\n`);
    return 1;
  }
  try {
    const a = loadResults(path.resolve(io.cwd, left));
    const b = loadResults(path.resolve(io.cwd, right));
    const metricsA = benchMetrics(a);
    const metricsB = benchMetrics(b);
    io.stdout.write(
      `${JSON.stringify(
        {
          a: { ...metricsA, budget: a.budget },
          b: { ...metricsB, budget: b.budget },
          delta: deltaOf(metricsA, metricsB),
        },
        null,
        2,
      )}\n`,
    );
    return 0;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    io.stderr.write(`${message}\n`);
    return 1;
  }
}

function loadResults(dir: string): BenchResults {
  const file = path.join(dir, "results.json");
  if (!existsSync(file)) throw new Error(`Missing results: ${file}`);
  const parsed: unknown = JSON.parse(readFileSync(file, "utf8"));
  if (!isRecord(parsed) || !Array.isArray(parsed.tasks)) {
    throw new Error(`Invalid results: ${file}`);
  }
  const policy = parsed.policy === "passthrough" ? "passthrough" : "compiler";
  const model = typeof parsed.model === "string" ? parsed.model : "";
  const budget =
    typeof parsed.budget === "number" && Number.isFinite(parsed.budget)
      ? parsed.budget
      : null;
  const toolNetwork = parsed.toolNetwork === "full" ? "full" : "none";
  const tasks = parsed.tasks.map((item) => readTaskResult(item));
  return { policy, model, budget, toolNetwork, tasks };
}

function readTaskResult(value: unknown): BenchTaskResult {
  if (!isRecord(value) || typeof value.id !== "string") {
    return {
      id: "unknown",
      status: "error",
      reason: "invalid-result",
      health: null,
    };
  }
  const status = benchStatus(value.status);
  const reason = typeof value.reason === "string" ? value.reason : undefined;
  return {
    id: value.id,
    status,
    ...(reason ? { reason } : {}),
    health: isRecord(value.health) ? (value.health as RunHealth) : null,
  };
}

function benchStatus(value: unknown): BenchStatus {
  if (value === "resolved" || value === "empty-patch" || value === "error") {
    return value;
  }
  return "error";
}

function deltaOf(a: BenchMetrics, b: BenchMetrics): BenchMetrics {
  return {
    resolvedRate: subtract(a.resolvedRate, b.resolvedRate),
    emptyPatchRate: subtract(a.emptyPatchRate, b.emptyPatchRate),
    errorRate: subtract(a.errorRate, b.errorRate),
    tokensPerResolved: subtract(a.tokensPerResolved, b.tokensPerResolved),
    cacheHitRatio: subtract(a.cacheHitRatio, b.cacheHitRatio),
    expandRate: subtract(a.expandRate, b.expandRate),
  };
}

function subtract(left: number | null, right: number | null): number | null {
  if (left === null || right === null) return null;
  return right - left;
}

function tokenTotal(task: BenchTaskResult): number {
  const total = task.health?.tokens?.total;
  return typeof total === "number" && Number.isFinite(total) ? total : 0;
}

function mean(values: readonly (number | null | undefined)[]): number | null {
  const finite = values.filter(
    (value): value is number =>
      typeof value === "number" && Number.isFinite(value),
  );
  if (finite.length === 0) return null;
  return finite.reduce((sum, value) => sum + value, 0) / finite.length;
}

async function runTask(
  task: BenchTask,
  out: string,
  deps: BenchDeps,
  opts: {
    policy: BenchPolicy;
    model: string;
    provider?: string;
    env: NodeJS.ProcessEnv;
    budget: number | null;
    toolNetwork: ToolNetwork;
  },
): Promise<BenchTaskResult> {
  const work = mkdtempSync(path.join(tmpdir(), "sage-bench-"));
  try {
    try {
      await deps.prepare(task, work);
    } catch {
      return {
        id: task.id,
        status: "error",
        reason: "clone-failed",
        health: null,
      };
    }
    if (task.setupCommand) {
      try {
        const setup = await exec("sh", ["-c", task.setupCommand], {
          cwd: work,
          timeoutMs: task.timeoutSec * 1000,
        });
        if (setup.code !== 0) {
          return {
            id: task.id,
            status: "error",
            reason: "setup-failed",
            health: null,
          };
        }
      } catch {
        return {
          id: task.id,
          status: "error",
          reason: "setup-failed",
          health: null,
        };
      }
    }
    let sage: { code: number; sessionFile?: string; timedOut?: boolean };
    try {
      sage = await deps.runSage(task, work, {
        ...opts,
        argv: sageArgv(task, opts),
      });
    } catch {
      return {
        id: task.id,
        status: "error",
        reason: "sage-failed",
        health: null,
      };
    }
    const health = sage.sessionFile
      ? copySession(sage.sessionFile, out, task.id)
      : null;
    if (sage.timedOut) {
      return { id: task.id, status: "error", reason: "timeout", health };
    }
    let empty = false;
    try {
      empty = await deps.diffEmpty(task, work);
    } catch {
      return { id: task.id, status: "error", reason: "diff-failed", health };
    }
    if (empty) return { id: task.id, status: "empty-patch", health };
    if (task.testPatch) {
      try {
        await applyTestPatch(task, work);
      } catch {
        return {
          id: task.id,
          status: "error",
          reason: "test-patch-failed",
          health,
        };
      }
    }
    let tests: { code: number; timedOut?: boolean };
    try {
      tests = await deps.runTests(task, work);
    } catch {
      return {
        id: task.id,
        status: "error",
        reason: "tests-failed",
        health,
      };
    }
    if (tests.timedOut) {
      return { id: task.id, status: "error", reason: "timeout", health };
    }
    if (tests.code === 0) return { id: task.id, status: "resolved", health };
    return { id: task.id, status: "error", reason: "tests-failed", health };
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

function copySession(
  file: string,
  out: string,
  taskId: string,
): RunHealth | null {
  const sessions = path.join(out, "sessions");
  mkdirSync(sessions, { recursive: true });
  const name = `${sessionName(taskId)}.jsonl`;
  copyFileSync(file, path.join(sessions, name));
  try {
    const store = new JsonlLogStore({ dir: sessions });
    return runHealth(store.path(sessionName(taskId)));
  } catch {
    return null;
  }
}

function sessionName(id: string): string {
  const cleaned = id.replace(/[^A-Za-z0-9._-]+/g, "_");
  return cleaned.length > 0 ? cleaned : "task";
}

function writeResults(out: string, results: BenchResults): void {
  mkdirSync(out, { recursive: true });
  const body = {
    policy: results.policy,
    model: results.model,
    budget: results.budget,
    toolNetwork: results.toolNetwork,
    tasks: results.tasks.map((task) => ({
      id: task.id,
      status: task.status,
      ...(task.reason ? { reason: task.reason } : {}),
      health: task.health,
    })),
  };
  writeFileSync(
    path.join(out, "results.json"),
    `${JSON.stringify(body, null, 2)}\n`,
  );
}

function defaultDeps(): BenchDeps {
  return {
    prepare: defaultPrepare,
    async runSage(task, cwd, opts) {
      const result = await exec(process.execPath, opts.argv, {
        cwd,
        env: opts.env,
        timeoutMs: task.timeoutSec * 1000,
      });
      const sessionFile = findSession(cwd);
      return {
        code: result.code,
        timedOut: result.timedOut,
        ...(sessionFile ? { sessionFile } : {}),
      };
    },
    async diffEmpty(_task, cwd) {
      const result = await exec("git", ["-C", cwd, "status", "--porcelain"], {
        cwd,
      });
      if (result.code !== 0) throw new Error("git status failed");
      return result.stdout.trim().length === 0;
    },
    async runTests(task, cwd) {
      const result = await exec("sh", ["-c", task.testCommand], {
        cwd,
        timeoutMs: task.timeoutSec * 1000,
      });
      return { code: result.code, timedOut: result.timedOut };
    },
  };
}

async function applyTestPatch(task: BenchTask, cwd: string): Promise<void> {
  const patch = task.testPatch;
  if (!patch) return;
  for (const file of filesInDiff(patch)) {
    const target = workPath(cwd, file);
    const probe = await exec("git", [
      "-C",
      cwd,
      "cat-file",
      "-e",
      `${task.baseCommit}:${file}`,
    ]);
    if (probe.code === 0) {
      const checkout = await exec("git", [
        "-C",
        cwd,
        "checkout",
        task.baseCommit,
        "--",
        file,
      ]);
      if (checkout.code !== 0) throw new Error(`checkout ${file} failed`);
    } else {
      rmSync(target, { force: true });
    }
  }
  const patchPath = path.join(
    tmpdir(),
    `sage-bench-test-patch-${process.pid}-${Date.now()}`,
  );
  try {
    writeFileSync(patchPath, patch);
    const applied = await exec("git", ["-C", cwd, "apply", patchPath]);
    if (applied.code !== 0) throw new Error("git apply failed");
  } finally {
    rmSync(patchPath, { force: true });
  }
}

function workPath(cwd: string, file: string): string {
  const root = path.resolve(cwd);
  const resolved = path.resolve(cwd, file);
  if (resolved !== root && !resolved.startsWith(`${root}${path.sep}`)) {
    throw new Error(`test patch path escapes worktree: ${file}`);
  }
  return resolved;
}

function filesInDiff(patch: string): string[] {
  const files: string[] = [];
  for (const line of patch.split("\n")) {
    const match = /^diff --git a\/\S+ b\/(\S+)$/.exec(line);
    const file = match?.[1];
    if (file && file !== "dev/null") files.push(file);
  }
  return files;
}

async function defaultPrepare(task: BenchTask, cwd: string): Promise<void> {
  const url = /^[a-z][a-z0-9+.-]*:\/\//i.test(task.repo)
    ? task.repo
    : `https://github.com/${task.repo}.git`;
  const clone = await exec("git", ["clone", url, cwd]);
  if (clone.code !== 0) throw new Error("clone failed");
  const checkout = await exec("git", ["-C", cwd, "checkout", task.baseCommit]);
  if (checkout.code !== 0) throw new Error("checkout failed");
}

function findSession(cwd: string): string | undefined {
  const dir = path.join(cwd, ".sage", "sessions");
  if (!existsSync(dir)) return undefined;
  const files = readdirSync(dir).filter((name) => name.endsWith(".jsonl"));
  if (files.length === 0) return undefined;
  files.sort(
    (left, right) =>
      statSync(path.join(dir, right)).mtimeMs -
      statSync(path.join(dir, left)).mtimeMs,
  );
  const name = files[0];
  return name ? path.join(dir, name) : undefined;
}

function sageBin(): string {
  return fileURLToPath(new URL("../bin/sage.mjs", import.meta.url));
}

function exec(
  command: string,
  args: string[],
  opts: { cwd?: string; env?: NodeJS.ProcessEnv; timeoutMs?: number } = {},
): Promise<{ code: number; timedOut: boolean; stdout: string }> {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: opts.cwd,
      env: opts.env ?? process.env,
      timeout: opts.timeoutMs,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      stdout += chunk;
    });
    child.stderr.resume();
    child.on("error", reject);
    child.on("close", (code, signal) => {
      resolve({
        code: code ?? 1,
        timedOut: code === null && signal !== null,
        stdout,
      });
    });
  });
}

function sageArgv(
  task: BenchTask,
  opts: {
    policy: BenchPolicy;
    model: string;
    provider?: string;
    budget: number | null;
    toolNetwork: ToolNetwork;
  },
): string[] {
  const args = [sageBin(), "-p", task.problem, "--model", opts.model];
  if (opts.provider) args.push("--provider", opts.provider);
  if (opts.budget !== null) args.push("--budget", String(opts.budget));
  if (opts.policy === "passthrough") args.push("--passthrough");
  args.push("--tool-network", opts.toolNetwork);
  return args;
}

function positiveInt(value: string): number | undefined {
  if (!/^[0-9]+$/.test(value)) return undefined;
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed <= 0) return undefined;
  return parsed;
}

function invalidBudget(value: string): string {
  return `Invalid --budget ${JSON.stringify(value)}: expected a positive integer`;
}

function invalidToolNetwork(value: string): string {
  return `Invalid --tool-network ${JSON.stringify(value)}: expected none or full`;
}

function parseRun(argv: string[]):
  | {
      tasks: string;
      policy: BenchPolicy;
      model: string;
      provider?: string;
      out: string;
      budget: number | null;
      toolNetwork: ToolNetwork;
    }
  | { message: string; exitCode?: number } {
  const values = new Map<string, string>();
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index] ?? "";
    const inline = /^--([A-Za-z0-9-]+)=(.*)$/.exec(arg);
    if (inline) {
      const name = inline[1];
      const value = inline[2];
      if (!name || !value || !FLAGS.has(name)) return { message: benchUsage() };
      values.set(name, value);
      continue;
    }
    if (!arg.startsWith("--")) return { message: benchUsage() };
    const name = arg.slice(2);
    if (!FLAGS.has(name)) return { message: benchUsage() };
    const value = argv[index + 1];
    if (!value || value.startsWith("-")) return { message: benchUsage() };
    values.set(name, value);
    index += 1;
  }
  const tasks = values.get("tasks");
  const policy = values.get("policy");
  const model = values.get("model");
  const out = values.get("out");
  if (!tasks || !policy || !model || !out) return { message: benchUsage() };
  if (policy !== "passthrough" && policy !== "compiler") {
    return { message: benchUsage() };
  }
  const provider = values.get("provider");
  if (provider && !resolvePreset(provider)) {
    return { message: benchUsage() };
  }
  const rawBudget = values.get("budget");
  let budget: number | null = null;
  if (rawBudget !== undefined) {
    const parsed = positiveInt(rawBudget);
    if (parsed === undefined) {
      return { message: invalidBudget(rawBudget), exitCode: 2 };
    }
    budget = parsed;
  }
  const rawNetwork = values.get("tool-network");
  let toolNetwork: ToolNetwork = "none";
  if (rawNetwork !== undefined) {
    if (rawNetwork !== "none" && rawNetwork !== "full") {
      return { message: invalidToolNetwork(rawNetwork), exitCode: 2 };
    }
    toolNetwork = rawNetwork;
  }
  return {
    tasks,
    policy,
    model,
    out,
    budget,
    toolNetwork,
    ...(provider ? { provider } : {}),
  };
}

const AUTO_PROVIDERS = [
  "deepseek",
  "dashscope",
  "moonshot",
  "openai",
  "anthropic",
] as const;

function missingKey(
  provider: string | undefined,
  env: NodeJS.ProcessEnv,
): string | undefined {
  if (provider) {
    const preset = resolvePreset(provider);
    if (!preset) return `Unknown provider: ${provider}`;
    return env[preset.envKey] ? undefined : `Missing ${preset.envKey}`;
  }
  const presets = AUTO_PROVIDERS.flatMap((name) => {
    const preset = resolvePreset(name);
    return preset ? [preset] : [];
  });
  if (presets.some((preset) => env[preset.envKey])) return undefined;
  const keys = presets.map((preset) => preset.envKey);
  if (keys.length === 0) return "Missing API key";
  if (keys.length === 1) return `Missing ${keys[0]}`;
  return `Missing ${keys.slice(0, -1).join(", ")}, or ${keys.at(-1)}`;
}

function parseTask(value: unknown, index: number): BenchTask {
  if (!isRecord(value)) throw new Error(`tasks[${index}] is not an object`);
  const timeoutSec = value.timeoutSec;
  if (
    typeof timeoutSec !== "number" ||
    !Number.isFinite(timeoutSec) ||
    timeoutSec <= 0
  ) {
    throw new Error(`tasks[${index}] timeoutSec must be a positive number`);
  }
  const setupCommand = optionalString(value, "setupCommand", index);
  const testPatch = optionalString(value, "testPatch", index);
  return {
    id: requiredString(value, "id", index),
    repo: requiredString(value, "repo", index),
    baseCommit: requiredString(value, "baseCommit", index),
    problem: requiredString(value, "problem", index),
    testCommand: requiredString(value, "testCommand", index),
    timeoutSec,
    ...(setupCommand ? { setupCommand } : {}),
    ...(testPatch ? { testPatch } : {}),
  };
}

function optionalString(
  value: Record<string, unknown>,
  key: string,
  index: number,
): string | undefined {
  if (!Object.hasOwn(value, key) || value[key] === undefined) return undefined;
  const found = value[key];
  if (typeof found !== "string") {
    throw new Error(`tasks[${index}] ${key} must be a string`);
  }
  if (found.length === 0) return undefined;
  return found;
}

function requiredString(
  value: Record<string, unknown>,
  key: string,
  index: number,
): string {
  const found = value[key];
  if (typeof found !== "string" || found.length === 0) {
    throw new Error(`tasks[${index}] missing ${key}`);
  }
  return found;
}

function benchUsage(): string {
  return [
    "usage: sage bench run --tasks <file> --policy passthrough|compiler --model <id> [--provider <preset>] [--budget <tokens>] [--tool-network none|full] --out <dir>",
    "       sage bench report <runA> <runB>",
  ].join("\n");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
