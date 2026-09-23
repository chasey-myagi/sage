/**
 * Git work for one bench task: fetch the base commit into an isolated
 * worktree, and apply the hidden test patch after the agent finishes.
 *
 * Contract: `defaultPrepare` leaves a worktree with exactly one commit and
 * no tags or remotes, so the agent cannot reach the fix through history.
 * `applyTestPatch` first restores every path the patch touches to
 * `baseCommit`, then runs `git apply`; paths outside the worktree throw.
 *
 * Intentionally not in the Log: repository URLs, git output, and the test
 * patch. The agent never sees the patch.
 */

import { rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { exec } from "./exec.js";
import { BENCH_WORKSPACE_PREFIX, type BenchTask } from "./types.js";

/** Thrown by `defaultPrepare` when the fetched worktree has extra history. */
export class CloneUncleanError extends Error {
  constructor() {
    super("clone unclean");
    this.name = "CloneUncleanError";
  }
}

/**
 * Fetches `task.baseCommit` into `cwd` as a single commit: `git init`, add
 * origin, shallow fetch of that SHA without tags, check out FETCH_HEAD, and
 * remove origin. A repo without a scheme is `https://github.com/<repo>.git`.
 * Throws `Error("clone failed")` when a step fails, and
 * {@link CloneUncleanError} when the result is not exactly one commit with
 * no tags and no remotes.
 */
export async function defaultPrepare(
  task: BenchTask,
  cwd: string,
): Promise<void> {
  const url = /^[a-z][a-z0-9+.-]*:\/\//i.test(task.repo)
    ? task.repo
    : `https://github.com/${task.repo}.git`;
  const steps = [
    ["init"],
    ["remote", "add", "origin", url],
    ["fetch", "--depth", "1", "--no-tags", "origin", task.baseCommit],
    ["checkout", "-q", "FETCH_HEAD"],
    ["remote", "remove", "origin"],
  ];
  for (const args of steps) {
    const step = await exec("git", args, { cwd });
    if (step.code !== 0) throw new Error("clone failed");
  }
  if (!(await isSingleCommit(cwd))) throw new CloneUncleanError();
}

async function isSingleCommit(cwd: string): Promise<boolean> {
  const count = await exec("git", ["rev-list", "--all", "--count"], { cwd });
  const tags = await exec("git", ["tag"], { cwd });
  const remotes = await exec("git", ["remote"], { cwd });
  return (
    count.code === 0 &&
    count.stdout.trim() === "1" &&
    tags.code === 0 &&
    tags.stdout.trim() === "" &&
    remotes.code === 0 &&
    remotes.stdout.trim() === ""
  );
}

/**
 * Applies `task.testPatch` in `cwd`. Each path in a `diff --git` header is
 * restored from `baseCommit` (`git checkout`, or deleted when the blob is
 * absent there) so agent edits cannot make the patch fail or pass. Throws
 * when a restore or `git apply` fails, or a path escapes `cwd`.
 */
export async function applyTestPatch(
  task: BenchTask,
  cwd: string,
): Promise<void> {
  const patch = task.testPatch;
  if (!patch) return;
  for (const file of filesInDiff(patch)) {
    await restoreFromBase(task.baseCommit, cwd, file);
  }
  const patchPath = path.join(
    tmpdir(),
    `${BENCH_WORKSPACE_PREFIX}test-patch-${process.pid}-${Date.now()}`,
  );
  try {
    writeFileSync(patchPath, patch);
    const applied = await exec("git", ["-C", cwd, "apply", patchPath]);
    if (applied.code !== 0) throw new Error("git apply failed");
  } finally {
    rmSync(patchPath, { force: true });
  }
}

async function restoreFromBase(
  baseCommit: string,
  cwd: string,
  file: string,
): Promise<void> {
  const target = workPath(cwd, file);
  const probe = await exec("git", [
    "-C",
    cwd,
    "cat-file",
    "-e",
    `${baseCommit}:${file}`,
  ]);
  if (probe.code !== 0) {
    rmSync(target, { force: true });
    return;
  }
  const checkout = await exec("git", [
    "-C",
    cwd,
    "checkout",
    baseCommit,
    "--",
    file,
  ]);
  if (checkout.code !== 0) throw new Error(`checkout ${file} failed`);
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
