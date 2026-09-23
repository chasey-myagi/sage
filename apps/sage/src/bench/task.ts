/**
 * Runs one bench task end to end and classifies the outcome.
 *
 * Contract: the task gets its own directory under the run root, deleted
 * when the task ends. Steps run in order and the first failure decides the
 * result: fetch (`clone-failed` / `clone-unclean`), `setupCommand`
 * (`setup-failed`), the child `sage -p` (`sage-failed` / `timeout`), an
 * empty `git status` (`empty-patch`, tests not run), the test patch
 * (`test-patch-failed`), and the test command (`resolved` on exit 0, else
 * `tests-failed` / `timeout`). The session jsonl is copied to
 * `<out>/sessions/` before the test patch is applied. Nothing here throws
 * for a task failure; only the final directory cleanup can.
 *
 * Intentionally not in the Log: setup commands, test patches, and test
 * output. The copied session jsonl is the recorded run.
 */

import { copyFileSync, mkdirSync, rmSync } from "node:fs";
import path from "node:path";
import { JsonlLogStore, type RunHealth, runHealth } from "@sage/core";
import { sageBin } from "./deps.js";
import { exec } from "./exec.js";
import { applyTestPatch, CloneUncleanError } from "./git.js";
import type {
  BenchDeps,
  BenchErrorReason,
  BenchTask,
  BenchTaskResult,
  SageRun,
  SageRunOptions,
} from "./types.js";

/** Runs `task` in `<runRoot>/<task id>` and returns its result row. */
export async function runTask(
  task: BenchTask,
  out: string,
  deps: BenchDeps,
  runRoot: string,
  opts: SageRunOptions,
): Promise<BenchTaskResult> {
  const work = path.join(runRoot, sessionName(task.id));
  mkdirSync(work, { recursive: true });
  try {
    const prepareFailure = await prepareWorkspace(task, work, deps);
    if (prepareFailure) return failed(task, prepareFailure, null);
    let sage: SageRun;
    try {
      sage = await deps.runSage(task, work, {
        ...opts,
        argv: sageArgv(task, opts),
      });
    } catch {
      // A spawn failure is the task's result; the run goes on.
      return failed(task, "sage-failed", null);
    }
    const health = sage.sessionFile
      ? copySession(sage.sessionFile, out, task.id)
      : null;
    if (sage.timedOut) return failed(task, "timeout", health);
    return await grade(task, work, deps, health);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

/**
 * The child argv: sage bin, `-p <problem>`, and the forwarded flags.
 * `--max-turns` and `--tool-network` are always present; `--budget` and
 * the scorer flags only when set.
 */
export function sageArgv(task: BenchTask, opts: SageRunOptions): string[] {
  const args = [sageBin(), "-p", task.problem, "--model", opts.model];
  if (opts.provider) args.push("--provider", opts.provider);
  if (opts.budget !== null) args.push("--budget", String(opts.budget));
  args.push("--max-turns", String(opts.maxTurns));
  if (opts.policy === "passthrough") args.push("--passthrough");
  args.push("--tool-network", opts.sandbox.network);
  if (opts.shadowScorer) args.push("--shadow-scorer", opts.shadowScorer);
  if (opts.scorer) args.push("--scorer", opts.scorer);
  if (opts.scorerApply) args.push("--scorer-apply", opts.scorerApply);
  return args;
}

async function prepareWorkspace(
  task: BenchTask,
  work: string,
  deps: BenchDeps,
): Promise<BenchErrorReason | undefined> {
  try {
    await deps.prepare(task, work);
  } catch (error) {
    return error instanceof CloneUncleanError
      ? "clone-unclean"
      : "clone-failed";
  }
  if (!task.setupCommand) return undefined;
  try {
    const setup = await exec("sh", ["-c", task.setupCommand], {
      cwd: work,
      timeoutMs: task.timeoutSec * 1000,
    });
    return setup.code === 0 ? undefined : "setup-failed";
  } catch {
    // `sh` could not be spawned; same outcome as a failing setup.
    return "setup-failed";
  }
}

async function grade(
  task: BenchTask,
  work: string,
  deps: BenchDeps,
  health: RunHealth | null,
): Promise<BenchTaskResult> {
  let empty: boolean;
  try {
    empty = await deps.diffEmpty(task, work);
  } catch {
    // Without a status there is nothing to grade.
    return failed(task, "diff-failed", health);
  }
  if (empty) return { id: task.id, status: "empty-patch", health };
  if (task.testPatch) {
    try {
      await applyTestPatch(task, work);
    } catch {
      // The reason code is the whole report; git output is not kept.
      return failed(task, "test-patch-failed", health);
    }
  }
  let tests: { code: number; timedOut?: boolean };
  try {
    tests = await deps.runTests(task, work);
  } catch {
    // The test command could not be spawned: counted as failing tests.
    return failed(task, "tests-failed", health);
  }
  if (tests.timedOut) return failed(task, "timeout", health);
  if (tests.code === 0) return { id: task.id, status: "resolved", health };
  return failed(task, "tests-failed", health);
}

function failed(
  task: BenchTask,
  reason: BenchErrorReason,
  health: RunHealth | null,
): BenchTaskResult {
  return { id: task.id, status: "error", reason, health };
}

function copySession(
  file: string,
  out: string,
  taskId: string,
): RunHealth | null {
  const sessions = path.join(out, "sessions");
  mkdirSync(sessions, { recursive: true });
  copyFileSync(file, path.join(sessions, `${sessionName(taskId)}.jsonl`));
  try {
    const store = new JsonlLogStore({ dir: sessions });
    return runHealth(store.path(sessionName(taskId)));
  } catch {
    // A corrupt or partial session still counts; health is just unknown.
    return null;
  }
}

function sessionName(id: string): string {
  const cleaned = id.replace(/[^A-Za-z0-9._-]+/g, "_");
  return cleaned.length > 0 ? cleaned : "task";
}
