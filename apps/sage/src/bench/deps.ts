/**
 * Production side effects of a bench task: git fetch, the child `sage -p`,
 * `git status`, and the test command.
 *
 * Contract: `runSage` spawns the sage bin with the argv the runner built,
 * under the task timeout, and reports the newest session jsonl in
 * `<cwd>/.sage/sessions`. When the sandbox has an `allowRoot`, the child
 * receives it as `SAGE_SANDBOX_ALLOW_ROOT`; there is no CLI flag for it.
 *
 * Intentionally not in the Log: the child env, `SAGE_SANDBOX_ALLOW_ROOT`,
 * and test output. The child writes its own session jsonl.
 */

import { existsSync, readdirSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { SANDBOX_ALLOW_ROOT_ENV } from "@sage/tools";
import { exec } from "./exec.js";
import { defaultPrepare } from "./git.js";
import type { BenchDeps } from "./types.js";

/** The deps a real `sage bench run` uses. */
export function defaultDeps(): BenchDeps {
  return {
    prepare: defaultPrepare,
    async runSage(task, cwd, opts) {
      const env = { ...opts.env };
      if (opts.sandbox.allowRoot) {
        env[SANDBOX_ALLOW_ROOT_ENV] = opts.sandbox.allowRoot;
      }
      const result = await exec(process.execPath, opts.argv, {
        cwd,
        env,
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

/** Absolute path of the sage bin; the first element of every child argv. */
export function sageBin(): string {
  return fileURLToPath(new URL("../../bin/sage.mjs", import.meta.url));
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
