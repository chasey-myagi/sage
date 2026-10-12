/**
 * `sage bench run`: runs every task in a tasks file under one policy and
 * writes `<out>/results.json`.
 *
 * Contract: the provider key (or the `--api-key-env` variable) is checked
 * before the tasks file is read or any repository fetched (missing key
 * exits 2). Tasks run one at a time in
 * a fresh run root `<realpath(tmpdir)>/sage-bench-<random>/`, deleted when
 * the run ends; `results.json` is rewritten after each task. Leftover
 * `sage-bench-*` directories from earlier runs are reported on stderr and
 * left in place. With `--tool-network none` (the bench default), bash in
 * each child may read only this run root among bench workspaces.
 *
 * Intentionally not in the Log: argv, API keys, clone directories, the
 * leftover-workspace warning, and `results.json`.
 */

import {
  mkdirSync,
  mkdtempSync,
  readdirSync,
  realpathSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { errorText } from "@sage/core";
import { resolveToolSandbox, type ToolSandbox } from "@sage/tools";
import { resolveBenchProvider } from "../auth.js";
import { type BenchRunArgs, parseRun } from "./args.js";
import { defaultDeps } from "./deps.js";
import { runTask } from "./task.js";
import { loadTasks } from "./tasks.js";
import {
  BENCH_WORKSPACE_PREFIX,
  type BenchIo,
  type BenchResults,
  type BenchTask,
  type SageRunOptions,
} from "./types.js";

/**
 * Runs `sage bench run <argv>`. Returns 0 once every task has a result
 * row, 1 for a usage error or unreadable tasks file, and 2 for an invalid
 * value or missing API key.
 */
export async function runBenchRun(
  argv: string[],
  io: BenchIo,
): Promise<number> {
  const parsed = parseRun(argv);
  if (!parsed.ok) {
    io.stderr.write(`${parsed.message}\n`);
    return parsed.exitCode;
  }
  const args = parsed.value;
  const selected = resolveBenchProvider(args, io.env);
  if (!selected.ok) {
    io.stderr.write(`${selected.message}\n`);
    return 2;
  }
  let tasks: BenchTask[];
  try {
    tasks = loadTasks(path.resolve(io.cwd, args.tasks));
  } catch (error) {
    io.stderr.write(`${errorText(error)}\n`);
    return 1;
  }
  for (const leftover of leftoverBenchRoots()) {
    io.stderr.write(
      `warning: leftover bench workspace ${leftover}; remove it manually\n`,
    );
  }
  await runAll(tasks, args, selected.provider, io);
  return 0;
}

async function runAll(
  tasks: readonly BenchTask[],
  args: BenchRunArgs,
  provider: string,
  io: BenchIo,
): Promise<void> {
  const out = path.resolve(io.cwd, args.out);
  const deps = io.deps ?? defaultDeps();
  const sandbox = resolveToolSandbox({ network: args.toolNetwork });
  const results: BenchResults = {
    policy: args.policy,
    model: args.model,
    provider,
    budget: args.policy === "passthrough" ? null : args.budget,
    baseUrl: args.baseUrl ?? null,
    apiKeyEnv: args.apiKeyEnv ?? null,
    toolNetwork: sandbox.network,
    maxTurns: args.maxTurns,
    scorerApply: args.scorerApply,
    tasks: [],
  };
  const runRoot = mkdtempSync(path.join(tmpdir(), BENCH_WORKSPACE_PREFIX));
  try {
    const opts = sageRunOptions(args, io.env, taskSandbox(sandbox, runRoot));
    for (const task of tasks) {
      results.tasks.push(await runTask(task, out, deps, runRoot, opts));
      writeResults(out, results);
    }
    writeResults(out, results);
  } finally {
    rmSync(runRoot, { recursive: true, force: true });
  }
}

/** With fsIsolate on, bash may read this run root and no sibling one. */
function taskSandbox(sandbox: ToolSandbox, runRoot: string): ToolSandbox {
  if (!sandbox.fsIsolate) return sandbox;
  return resolveToolSandbox({
    network: sandbox.network,
    fsIsolate: true,
    allowRoot: realpathSync(runRoot),
  });
}

function sageRunOptions(
  args: BenchRunArgs,
  env: NodeJS.ProcessEnv,
  sandbox: ToolSandbox,
): SageRunOptions {
  return {
    policy: args.policy,
    model: args.model,
    ...(args.provider ? { provider: args.provider } : {}),
    ...(args.baseUrl ? { baseUrl: args.baseUrl } : {}),
    ...(args.apiKeyEnv ? { apiKeyEnv: args.apiKeyEnv } : {}),
    env,
    budget: args.budget,
    toolNetwork: sandbox.network,
    sandbox,
    maxTurns: args.maxTurns,
    ...(args.shadowScorer ? { shadowScorer: args.shadowScorer } : {}),
    ...(args.scorer ? { scorer: args.scorer } : {}),
    ...(args.forwardScorerApply ? { scorerApply: args.scorerApply } : {}),
  };
}

function writeResults(out: string, results: BenchResults): void {
  mkdirSync(out, { recursive: true });
  const body = {
    policy: results.policy,
    model: results.model,
    provider: results.provider,
    budget: results.budget,
    baseUrl: results.baseUrl,
    apiKeyEnv: results.apiKeyEnv,
    toolNetwork: results.toolNetwork,
    maxTurns: results.maxTurns,
    scorerApply: results.scorerApply,
    tasks: results.tasks.map((task) => ({
      id: task.id,
      status: task.status,
      ...(task.reason ? { reason: task.reason } : {}),
      health: task.health,
      stoppedEarly: task.stoppedEarly,
    })),
  };
  writeFileSync(
    path.join(out, "results.json"),
    `${JSON.stringify(body, null, 2)}\n`,
  );
}

/** `sage-bench-*` directories directly under the real temp root, sorted. */
function leftoverBenchRoots(): string[] {
  let root: string;
  try {
    root = realpathSync(tmpdir());
  } catch {
    // No temp root means no leftovers to warn about.
    return [];
  }
  let names: string[];
  try {
    names = readdirSync(root);
  } catch {
    // An unreadable temp root only means there is nothing to warn about.
    return [];
  }
  const found: string[] = [];
  for (const name of names) {
    if (!name.startsWith(BENCH_WORKSPACE_PREFIX)) continue;
    const full = path.join(root, name);
    if (isDirectory(full)) found.push(full);
  }
  return found.sort();
}

function isDirectory(file: string): boolean {
  try {
    return statSync(file).isDirectory();
  } catch {
    // Removed between readdir and stat: not a leftover.
    return false;
  }
}
